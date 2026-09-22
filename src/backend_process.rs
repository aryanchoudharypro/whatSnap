//! Owns the Go backend process and brings it back if it dies while whaNTV is
//! still running. Without this a backend crash left the UI up but inert: the
//! pipe thread kept retrying a pipe nobody was listening on any more, and the
//! only fix was restarting the whole app.

use std::{
	os::windows::process::CommandExt,
	path::PathBuf,
	process::{Child, Command, Stdio},
	sync::{
		Mutex,
		atomic::{AtomicBool, AtomicU32, Ordering},
	},
	thread,
	time::{Duration, Instant},
};

use crate::win_job::KillOnCloseJob;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Running at least this long counts as healthy and resets the backoff, so
/// a crash after hours of use restarts immediately while a crash loop at
/// startup (corrupt database, missing DLL) slows down instead of spinning.
const HEALTHY_UPTIME: Duration = Duration::from_secs(60);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);

pub struct BackendProcess {
	exe: PathBuf,
	dir: PathBuf,
	db_path: String,
	pipe_name: String,
	verbose_logging: bool,
	job: Option<KillOnCloseJob>,
	child: Mutex<Option<Child>>,
	/// PID of the live backend (0 = none), for the pipe peer check.
	pid: AtomicU32,
	started_at: Mutex<Instant>,
	quick_crashes: AtomicU32,
	shutting_down: AtomicBool,
}

impl BackendProcess {
	pub fn start(exe: PathBuf, dir: PathBuf, db_path: String, pipe_name: String, verbose_logging: bool) -> Self {
		let backend = Self {
			exe,
			dir,
			db_path,
			pipe_name,
			verbose_logging,
			job: KillOnCloseJob::new().ok(),
			child: Mutex::new(None),
			pid: AtomicU32::new(0),
			started_at: Mutex::new(Instant::now()),
			quick_crashes: AtomicU32::new(0),
			shutting_down: AtomicBool::new(false),
		};
		let mut child = backend.child.lock().unwrap();
		*child = backend.spawn();
		drop(child);
		backend
	}

	fn spawn(&self) -> Option<Child> {
		// The backend redirects its own os.Stdout/os.Stderr to backend.log,
		// but that only catches what Go code itself writes - an unrecovered
		// panic's trace comes from the Go runtime, which writes straight to
		// the OS-level stderr handle and never goes through os.Stderr. With
		// that handle pointed at Stdio::null(), a crash used to leave no
		// trace anywhere. Pointing it at a file instead is what makes a
		// crash's actual cause recoverable rather than just "it restarted".
		let stderr_file = std::fs::OpenOptions::new()
			.create(true)
			.append(true)
			.open(crate::paths::log_dir().join("backend_stderr.log"))
			.map(Stdio::from)
			.unwrap_or_else(|_| Stdio::null());

		let mut cmd = Command::new(&self.exe);
		cmd.arg("-db").arg(&self.db_path).arg("-pipe").arg(&self.pipe_name);
		if self.verbose_logging {
			cmd.arg("-verbose-logging");
		}
		let child = cmd
			.current_dir(&self.dir)
			.stdout(Stdio::null())
			.stderr(stderr_file)
			.creation_flags(CREATE_NO_WINDOW)
			.spawn()
			.map_err(|e| log(&format!("Failed to start backend: {e}")))
			.ok()?;
		if let Some(job) = &self.job {
			let _ = job.assign(&child);
		}
		self.pid.store(child.id(), Ordering::SeqCst);
		*self.started_at.lock().unwrap() = Instant::now();
		Some(child)
	}

	pub fn pid(&self) -> Option<u32> {
		match self.pid.load(Ordering::SeqCst) {
			0 => None,
			pid => Some(pid),
		}
	}

	/// Called by the pipe thread whenever the pipe drops or won't connect.
	/// If the backend process is gone (and we're not quitting), waits out
	/// the crash backoff and starts a fresh one. Returns true if it did, so
	/// the caller can tell the user; false means the backend is still
	/// running (a pipe blip) or the app is shutting down.
	pub fn restart_if_dead(&self) -> bool {
		let delay = {
			let mut child = self.child.lock().unwrap();
			if self.shutting_down.load(Ordering::SeqCst) {
				return false;
			}
			if let Some(c) = child.as_mut() {
				match c.try_wait() {
					Ok(None) => return false,
					Ok(Some(status)) => log(&format!("Backend exited unexpectedly ({status}); restarting")),
					Err(e) => log(&format!("Backend status check failed ({e}); restarting")),
				}
			}
			*child = None;
			self.pid.store(0, Ordering::SeqCst);

			let crashes = if self.started_at.lock().unwrap().elapsed() >= HEALTHY_UPTIME {
				self.quick_crashes.store(0, Ordering::SeqCst);
				0
			} else {
				self.quick_crashes.fetch_add(1, Ordering::SeqCst)
			};
			// 0s, 1s, 2s, 4s ... capped at 30s.
			if crashes == 0 {
				Duration::ZERO
			} else {
				Duration::from_secs(1u64 << (crashes - 1).min(5)).min(MAX_RESTART_DELAY)
			}
		};

		thread::sleep(delay);

		let mut child = self.child.lock().unwrap();
		if self.shutting_down.load(Ordering::SeqCst) || child.is_some() {
			return false;
		}
		*child = self.spawn();
		child.is_some()
	}

	/// Stops the backend for good at app exit: asks it to shut down cleanly
	/// via `request_shutdown`, then kills it if it hasn't exited in 5s.
	/// Setting `shutting_down` first is what keeps `restart_if_dead` from
	/// resurrecting it the moment it exits.
	pub fn shutdown(&self, request_shutdown: impl FnOnce()) {
		self.shutting_down.store(true, Ordering::SeqCst);
		let Some(mut child) = self.child.lock().unwrap().take() else {
			return;
		};
		request_shutdown();

		let deadline = Instant::now() + Duration::from_secs(5);
		loop {
			match child.try_wait() {
				Ok(Some(_)) => break,
				Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
				_ => {
					let _ = child.kill();
					break;
				}
			}
		}
		let _ = child.wait();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// `where.exe` rejects our -db/-pipe arguments and exits at once, which
	/// is exactly what a backend crashing on startup looks like.
	fn crashing_backend() -> BackendProcess {
		let exe = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32").join("where.exe");
		BackendProcess::start(exe, std::env::temp_dir(), "unused".into(), "unused".into(), false)
	}

	fn wait_for_exit(b: &BackendProcess) {
		let deadline = Instant::now() + Duration::from_secs(5);
		while Instant::now() < deadline {
			if let Some(c) = b.child.lock().unwrap().as_mut() {
				if c.try_wait().ok().flatten().is_some() {
					return;
				}
			}
			thread::sleep(Duration::from_millis(20));
		}
		panic!("stand-in backend never exited");
	}

	#[test]
	fn restarts_dead_backend_with_growing_backoff() {
		let b = crashing_backend();
		let first_pid = b.pid().expect("spawned");

		wait_for_exit(&b);
		let t = Instant::now();
		assert!(b.restart_if_dead(), "first crash restarts");
		assert!(t.elapsed() < Duration::from_millis(500), "first restart is immediate");
		assert_ne!(b.pid(), Some(first_pid), "new process");

		wait_for_exit(&b);
		let t = Instant::now();
		assert!(b.restart_if_dead(), "second crash restarts");
		assert!(t.elapsed() >= Duration::from_millis(900), "quick second crash backs off");
		b.shutdown(|| {});
	}

	#[test]
	fn no_restart_after_shutdown() {
		let b = crashing_backend();
		wait_for_exit(&b);
		b.shutdown(|| {});
		assert!(!b.restart_if_dead());
		assert!(b.child.lock().unwrap().is_none());
	}
}

fn log(msg: &str) {
	use std::io::Write;
	if let Ok(mut f) =
		std::fs::OpenOptions::new().create(true).append(true).open(crate::paths::log_dir().join("backend_process.log"))
	{
		let _ = writeln!(f, "[{}] {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
	}
}
