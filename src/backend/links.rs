//! One backend for every linked account, over one pipe.
//!
//! The backend runs a WhatsApp client for each account the app opens, so every
//! account stays online and gets its notifications. Each command the app sends
//! carries the account it is for, and each event the backend sends carries the
//! account it came from. The UI keeps a `Link` per account whose sender looks
//! just like the old single pipe; this module tags what goes through it, and
//! tags what comes back with `UiCommand::BackendEvent`. The UI shows the
//! active account and hands the others' events to `ui_commands::accounts`.

use std::{
	collections::HashMap,
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

/// How long removing an account waits for the backend to let go of its files.
const CLOSE_WAIT: Duration = Duration::from_secs(20);

struct Backend {
	process: Option<Arc<BackendProcess>>,
	pipe_id: String,
	/// Commands on their way to the backend, already tagged with their account.
	out_tx: mpsc::Sender<String>,
	out_rx: Mutex<Option<mpsc::Receiver<String>>>,
	/// Set at Exit, so the pipe thread lets go instead of restarting it.
	stopped: AtomicBool,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// The accounts the backend should have open, in the order they were opened.
static OPEN: Mutex<Vec<Arc<str>>> = Mutex::new(Vec::new());

/// Removals waiting for the backend to close their account.
static CLOSING: Mutex<Option<HashMap<String, mpsc::Sender<()>>>> = Mutex::new(None);

static UI: OnceLock<(UiCommandSender, Arc<AtomicBool>)> = OnceLock::new();

/// What the UI keeps for each account: where to send its commands.
#[derive(Clone)]
pub struct Link {
	pub account: Arc<str>,
	pub ws_tx: mpsc::Sender<String>,
}

/// Starts the backend. Called once at startup, before any account is opened.
pub fn set_launcher(exe: Option<PathBuf>, dir: PathBuf, verbose: bool) {
	let pipe_id = format!("whantv-{}", crate::util::random_hex());
	let process = exe.map(|exe| Arc::new(BackendProcess::start(exe, dir, format!(r"\\.\pipe\{pipe_id}"), verbose)));
	let (out_tx, out_rx) = mpsc::channel();
	let _ = BACKEND.set(Backend {
		process,
		pipe_id,
		out_tx,
		out_rx: Mutex::new(Some(out_rx)),
		stopped: AtomicBool::new(false),
	});
}

fn backend() -> &'static Backend {
	BACKEND.get().expect("set_launcher runs before any account opens")
}

fn open_list() -> std::sync::MutexGuard<'static, Vec<Arc<str>>> {
	OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

fn json_str(s: &str) -> String {
	serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Puts the account into a command the UI built as `{"action":...}`.
fn tag(account: &str, command: &str) -> Option<String> {
	let rest = command.trim().strip_prefix('{')?;
	let sep = if rest.trim_start().starts_with('}') { "" } else { "," };
	Some(format!("{{\"account\":{}{sep}{rest}", json_str(account)))
}

fn open_command(account: &str) -> String {
	let db = crate::platform::accounts::db_path(account).to_string_lossy().into_owned();
	format!("{{\"action\":\"open_account\",\"account\":{},\"data\":{{\"db\":{}}}}}", json_str(account), json_str(&db))
}

/// Opens an account in the backend (if it isn't open already) and returns
/// where to send its commands.
pub fn start(account: &str) -> Link {
	let backend = backend();
	let account: Arc<str> = {
		let mut open = open_list();
		match open.iter().find(|a| &***a == account) {
			Some(a) => a.clone(),
			None => {
				let a: Arc<str> = Arc::from(account);
				open.push(a.clone());
				let _ = backend.out_tx.send(open_command(account));
				a
			}
		}
	};
	let (ws_tx, ws_rx) = mpsc::channel::<String>();
	let out_tx = backend.out_tx.clone();
	let forwarded = account.clone();
	let _ = thread::Builder::new().name(format!("link-{}", &account[..account.len().min(8)])).spawn(move || {
		for command in ws_rx {
			if !open_list().iter().any(|a| Arc::ptr_eq(a, &forwarded)) {
				break;
			}
			if let Some(line) = tag(&forwarded, &command) {
				let _ = out_tx.send(line);
			}
		}
	});
	Link { account, ws_tx }
}

/// Connects the backend's pipe to the UI. Nothing is read before this, so
/// nothing the backend says early goes to a UI that isn't there yet.
pub fn attach_ui(ui_tx: UiCommandSender, alive: Arc<AtomicBool>) {
	if UI.set((ui_tx.clone(), alive.clone())).is_err() {
		return;
	}
	let Some(out_rx) = backend().out_rx.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
	let _ = thread::Builder::new().name("backend-pipe".into()).spawn(move || {
		pipe_loop(ui_tx, alive, Arc::new(Mutex::new(out_rx)));
	});
}

/// The account an event belongs to. The backend writes every event as
/// `{"type":"…","account":"…",…}`, so this needs no full parse.
fn event_account(line: &str) -> Option<&str> {
	let rest = line.strip_prefix("{\"type\":\"")?;
	let rest = &rest[rest.find('"')? + 1..];
	let rest = rest.strip_prefix(",\"account\":\"")?;
	Some(&rest[..rest.find('"')?])
}

fn event_type(line: &str) -> Option<&str> {
	let rest = line.strip_prefix("{\"type\":\"")?;
	Some(&rest[..rest.find('"')?])
}

fn pipe_loop(ui_tx: UiCommandSender, alive: Arc<AtomicBool>, out_rx: Arc<Mutex<mpsc::Receiver<String>>>) {
	let backend = backend();
	let live = || alive.load(Ordering::SeqCst) && !backend.stopped.load(Ordering::SeqCst);
	let deliver = |account: &str, payload: String| {
		let account = open_list().iter().find(|a| &***a == account).cloned();
		if let Some(account) = account {
			let _ = ui_tx.send(UiCommand::BackendEvent { account, payload });
		}
	};
	let mut connected_before = false;
	while live() {
		if backend.process.as_ref().is_some_and(|b| b.restart_if_dead()) {
			let accounts: Vec<Arc<str>> = open_list().clone();
			for account in accounts {
				let notice = serde_json::json!({
					"type": "connection_state",
					"data": {
						"state": "offline",
						"message": "whatSnap's background service stopped unexpectedly and has been restarted."
					}
				});
				deliver(&account, notice.to_string());
			}
		}

		if let Ok(socket) = crate::backend::pipe::PipeStream::connect(&backend.pipe_id) {
			let expected_pid = backend.process.as_ref().map(|b| b.pid());
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
			// A new connection may be to a restarted backend that has no
			// account open: open them all again before anything else. The
			// first connection gets the opens already queued by start().
			if connected_before {
				let accounts: Vec<Arc<str>> = open_list().clone();
				let opens: String = accounts.iter().map(|a| open_command(a) + "\n").collect();
				if socket_writer.write_all(opens.as_bytes()).is_err() {
					thread::sleep(Duration::from_secs(2));
					continue;
				}
			}
			connected_before = true;
			let mut buf_reader = BufReader::new(socket);
			let mut line = String::new();

			let writer_alive = alive.clone();
			let rx = out_rx.clone();
			let resend_tx = backend.out_tx.clone();
			let reader_done = Arc::new(AtomicBool::new(false));
			let writer_done = reader_done.clone();
			thread::spawn(move || {
				while writer_alive.load(Ordering::SeqCst)
					&& !backend.stopped.load(Ordering::SeqCst)
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

			while live() {
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
						let payload = std::mem::take(&mut line);
						let Some(account) = event_account(&payload).map(str::to_owned) else { continue };
						match event_type(&payload) {
							Some("account_closed") => {
								let waiter = CLOSING
									.lock()
									.unwrap_or_else(|e| e.into_inner())
									.as_mut()
									.and_then(|m| m.remove(&account));
								if let Some(waiter) = waiter {
									let _ = waiter.send(());
								}
							}
							Some("account_error") => crate::backend::events::ui_timing_log(&format!(
								"Backend: couldn't open account {account}: {payload}"
							)),
							_ => deliver(&account, payload),
						}
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

		if live() {
			thread::sleep(Duration::from_secs(2));
		}
	}
}

/// Closes an account in the backend for good (account removed or abandoned)
/// on a thread of its own, then calls `done` once the backend has let go of
/// its files, so they can be deleted.
pub fn stop(account: &str, done: impl FnOnce() + Send + 'static) {
	let was_open = {
		let mut open = open_list();
		let before = open.len();
		open.retain(|a| &**a != account);
		open.len() != before
	};
	let Some(backend) = BACKEND.get().filter(|_| was_open) else {
		done();
		return;
	};
	let (closed_tx, closed_rx) = mpsc::channel();
	CLOSING
		.lock()
		.unwrap_or_else(|e| e.into_inner())
		.get_or_insert_with(HashMap::new)
		.insert(account.to_string(), closed_tx);
	let _ = backend.out_tx.send(format!("{{\"action\":\"close_account\",\"account\":{}}}", json_str(account)));
	let account = account.to_string();
	thread::spawn(move || {
		if closed_rx.recv_timeout(CLOSE_WAIT).is_err() {
			crate::backend::events::ui_timing_log(&format!("Backend: account {account} didn't close in time"));
			CLOSING.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|m| m.remove(&account));
		}
		done();
	});
}

/// Stops the backend at Exit and returns once it is down.
pub fn shut_down_all() {
	let Some(backend) = BACKEND.get() else { return };
	backend.stopped.store(true, Ordering::SeqCst);
	if let Some(process) = &backend.process {
		let pipe_id = backend.pipe_id.clone();
		process.shutdown(move || crate::ui::chat_ui::request_backend_shutdown(&pipe_id));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tags_commands_with_their_account() {
		assert_eq!(
			tag("default", r#"{"action":"send","data":{"text":"hi"}}"#).as_deref(),
			Some(r#"{"account":"default","action":"send","data":{"text":"hi"}}"#)
		);
		assert_eq!(tag("default", "{}").as_deref(), Some(r#"{"account":"default"}"#));
		assert_eq!(tag("default", "not json"), None);
	}

	#[test]
	fn reads_the_account_of_an_event() {
		let line = r#"{"type":"message","account":"0123456789abcdef0123456789abcdef","data":{}}"#;
		assert_eq!(event_account(line), Some("0123456789abcdef0123456789abcdef"));
		assert_eq!(event_type(line), Some("message"));
		assert_eq!(event_account(r#"{"type":"message","data":{}}"#), None);
	}
}
