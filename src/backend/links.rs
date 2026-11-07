//! One backend per linked account, each with its own pipe.
//!
//! Every account keeps its own backend process running, so every account stays
//! online and gets its notifications. Each backend's pipe thread tags what it
//! reads with the account it belongs to; the UI shows the active account and
//! hands the others' events to `ui_commands::accounts`.

use std::{
	io::{BufRead, BufReader, Write},
	path::PathBuf,
	sync::{
		Arc, Mutex, OnceLock,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	thread,
	time::Duration,
};

use crate::{UiCommand, backend::process::BackendProcess, ui::wake::UiCommandSender};

struct Launcher {
	exe: Option<PathBuf>,
	dir: PathBuf,
	verbose: bool,
}

static LAUNCHER: OnceLock<Launcher> = OnceLock::new();

/// Every backend that is running, so Exit can stop them all.
static RUNNING: Mutex<Vec<Arc<Running>>> = Mutex::new(Vec::new());

struct Running {
	account: Arc<str>,
	process: Option<Arc<BackendProcess>>,
	pipe_id: String,
	/// Set when the account's backend is being stopped for good, so its pipe
	/// thread lets go instead of restarting it.
	stopped: AtomicBool,
	/// The pipe thread waits for the UI before reading, so nothing an early
	/// backend says is sent to a UI that isn't there yet.
	ui: Mutex<Option<(UiCommandSender, Arc<AtomicBool>)>>,
	ws_rx: Mutex<Option<mpsc::Receiver<String>>>,
	ws_tx: mpsc::Sender<String>,
}

/// What the UI keeps for each running backend: where to send its commands.
#[derive(Clone)]
pub struct Link {
	pub account: Arc<str>,
	pub ws_tx: mpsc::Sender<String>,
}

/// Where the backend executable is. Called once at startup.
pub fn set_launcher(exe: Option<PathBuf>, dir: PathBuf, verbose: bool) {
	let _ = LAUNCHER.set(Launcher { exe, dir, verbose });
}

/// Starts the backend for an account (if it isn't running already) and
/// returns where to send its commands.
pub fn start(account: &str) -> Link {
	let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
	if let Some(r) = running.iter().find(|r| &*r.account == account) {
		return Link { account: r.account.clone(), ws_tx: r.ws_tx.clone() };
	}
	let launcher = LAUNCHER.get().expect("set_launcher runs before any backend starts");
	let pipe_id = format!("whantv-{}", crate::util::random_hex());
	let db_path = crate::platform::accounts::db_path(account).to_string_lossy().into_owned();
	let process = launcher.exe.clone().map(|exe| {
		Arc::new(BackendProcess::start(
			exe,
			launcher.dir.clone(),
			db_path,
			format!(r"\\.\pipe\{pipe_id}"),
			launcher.verbose,
		))
	});
	let (ws_tx, ws_rx) = mpsc::channel();
	let r = Arc::new(Running {
		account: Arc::from(account),
		process,
		pipe_id,
		stopped: AtomicBool::new(false),
		ui: Mutex::new(None),
		ws_rx: Mutex::new(Some(ws_rx)),
		ws_tx: ws_tx.clone(),
	});
	running.push(r.clone());
	drop(running);
	if let Some((ui_tx, alive)) = UI.get() {
		connect(&r, ui_tx.clone(), alive.clone());
	}
	Link { account: r.account.clone(), ws_tx }
}

static UI: OnceLock<(UiCommandSender, Arc<AtomicBool>)> = OnceLock::new();

/// Connects the pipes of every backend started so far (and of every backend
/// started after this) to the UI.
pub fn attach_ui(ui_tx: UiCommandSender, alive: Arc<AtomicBool>) {
	let _ = UI.set((ui_tx.clone(), alive.clone()));
	let running: Vec<Arc<Running>> = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).clone();
	for r in running {
		connect(&r, ui_tx.clone(), alive.clone());
	}
}

fn connect(r: &Arc<Running>, ui_tx: UiCommandSender, alive: Arc<AtomicBool>) {
	{
		let mut ui = r.ui.lock().unwrap_or_else(|e| e.into_inner());
		if ui.is_some() {
			return;
		}
		*ui = Some((ui_tx.clone(), alive.clone()));
	}
	let Some(ws_rx) = r.ws_rx.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
	let r = r.clone();
	let _ = thread::Builder::new().name(format!("pipe-{}", &r.account[..r.account.len().min(8)])).spawn(move || {
		pipe_loop(r, ui_tx, alive, Arc::new(Mutex::new(ws_rx)));
	});
}

fn pipe_loop(
	r: Arc<Running>,
	ui_tx: UiCommandSender,
	alive: Arc<AtomicBool>,
	ws_rx: Arc<Mutex<mpsc::Receiver<String>>>,
) {
	let live = |r: &Running| alive.load(Ordering::SeqCst) && !r.stopped.load(Ordering::SeqCst);
	let send = |payload: String| {
		let _ = ui_tx.send(UiCommand::BackendEvent { account: r.account.clone(), payload });
	};
	while live(&r) {
		if r.process.as_ref().is_some_and(|b| b.restart_if_dead()) {
			let notice = serde_json::json!({
				"type": "connection_state",
				"data": {
					"state": "offline",
					"message": "whatSnap's background service stopped unexpectedly and has been restarted."
				}
			});
			send(notice.to_string());
		}

		if let Ok(socket) = crate::backend::pipe::PipeStream::connect(&r.pipe_id) {
			let expected_pid = r.process.as_ref().map(|b| b.pid());
			let peer_ok = match (expected_pid, socket.server_pid()) {
				(Some(Some(expected)), Some(actual)) => actual == expected,
				(None, _) => true,
				_ => false,
			};
			if !peer_ok {
				drop(socket);
				thread::sleep(Duration::from_secs(2));
				continue;
			}

			let mut socket_writer = socket.try_clone().expect("Failed to clone pipe");
			let mut buf_reader = BufReader::new(socket);
			let mut line = String::new();

			let writer_r = r.clone();
			let writer_alive = alive.clone();
			let rx = ws_rx.clone();
			let resend_tx = r.ws_tx.clone();
			let reader_done = Arc::new(AtomicBool::new(false));
			let writer_done = reader_done.clone();
			thread::spawn(move || {
				while writer_alive.load(Ordering::SeqCst)
					&& !writer_r.stopped.load(Ordering::SeqCst)
					&& !writer_done.load(Ordering::SeqCst)
				{
					let msg =
						rx.lock().unwrap_or_else(|e| e.into_inner()).recv_timeout(Duration::from_millis(500)).ok();
					if let Some(out_msg) = msg
						&& let Err(e) = socket_writer.write_all(format!("{out_msg}\n").as_bytes())
					{
						crate::backend::events::ui_timing_log(&format!(
							"Backend pipe: sending a command failed ({e}); resending on the next connection"
						));
						let _ = resend_tx.send(out_msg);
						break;
					}
				}
			});

			while live(&r) {
				match buf_reader.read_line(&mut line) {
					Ok(0) => {
						crate::backend::events::ui_timing_log("Backend pipe: closed by the backend; reconnecting");
						break;
					}
					Ok(_) => {
						line.truncate(line.trim_end().len());
						let leading = line.len() - line.trim_start().len();
						if leading > 0 {
							line.drain(..leading);
						}
						send(std::mem::take(&mut line));
					}
					Err(e) => {
						crate::backend::events::ui_timing_log(&format!(
							"Backend pipe: reading failed ({e}); reconnecting"
						));
						break;
					}
				}
			}
			reader_done.store(true, Ordering::SeqCst);
		}

		if live(&r) {
			thread::sleep(Duration::from_secs(2));
		}
	}
}

/// Stops an account's backend for good (account removed or abandoned) on a
/// thread of its own, then calls `done` once it has exited, so its files can
/// be deleted.
pub fn stop(account: &str, done: impl FnOnce() + Send + 'static) {
	let r = {
		let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
		let Some(index) = running.iter().position(|r| &*r.account == account) else {
			drop(running);
			done();
			return;
		};
		running.remove(index)
	};
	r.stopped.store(true, Ordering::SeqCst);
	thread::spawn(move || {
		shut_down(&r);
		done();
	});
}

fn shut_down(r: &Running) {
	if let Some(process) = &r.process {
		let pipe_id = r.pipe_id.clone();
		process.shutdown(move || crate::ui::chat_ui::request_backend_shutdown(&pipe_id));
	}
}

/// Stops every backend at once (Exit), each on its own thread, and returns
/// when all of them are down.
pub fn shut_down_all() {
	let running: Vec<Arc<Running>> = std::mem::take(&mut *RUNNING.lock().unwrap_or_else(|e| e.into_inner()));
	let threads: Vec<_> = running
		.into_iter()
		.map(|r| {
			r.stopped.store(true, Ordering::SeqCst);
			thread::spawn(move || shut_down(&r))
		})
		.collect();
	for t in threads {
		let _ = t.join();
	}
}
