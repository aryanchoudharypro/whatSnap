use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicUsize, Ordering},
	mpsc::{self, Sender},
};

use wxdragon::{ffi, prelude::*};

pub const ID_UI_WAKE: i32 = 10001;

pub const UI_WAKE_HANDLERS: usize = 8;

#[derive(Clone)]
pub struct UiWaker {
	frame_ptr: usize,
	pending: Arc<AtomicBool>,
	alive: Arc<AtomicBool>,
	depth: Arc<AtomicUsize>,
}

impl UiWaker {
	pub fn new(frame: Frame, alive: Arc<AtomicBool>) -> Self {
		Self {
			frame_ptr: frame.handle_ptr() as usize,
			pending: Arc::new(AtomicBool::new(false)),
			alive,
			depth: Arc::new(AtomicUsize::new(0)),
		}
	}

	pub fn wake(&self) {
		if !self.pending.swap(true, Ordering::SeqCst) {
			if !self.alive.load(Ordering::SeqCst) {
				return;
			}
			let handle = self.frame_ptr as *mut ffi::wxd_Window_t;
			if handle.is_null() {
				return;
			}
			let slot = self.depth.load(Ordering::SeqCst).min(UI_WAKE_HANDLERS - 1);
			unsafe { ffi::wxd_Window_PostMenuCommand(handle, ID_UI_WAKE + slot as i32) };
		}
	}

	pub fn run(&self, mut drain: impl FnMut()) {
		self.depth.fetch_add(1, Ordering::SeqCst);
		loop {
			self.pending.store(false, Ordering::SeqCst);
			drain();
			if !self.pending.load(Ordering::SeqCst) {
				break;
			}
		}
		self.depth.fetch_sub(1, Ordering::SeqCst);
	}
}

#[derive(Clone)]
pub struct UiCommandSender {
	tx: Sender<crate::UiCommand>,
	waker: UiWaker,
}

impl UiCommandSender {
	pub const fn new(tx: Sender<crate::UiCommand>, waker: UiWaker) -> Self {
		Self { tx, waker }
	}

	pub fn send(&self, cmd: crate::UiCommand) -> Result<(), Box<mpsc::SendError<crate::UiCommand>>> {
		let result = self.tx.send(cmd).map_err(Box::new);
		self.waker.wake();
		result
	}
}
