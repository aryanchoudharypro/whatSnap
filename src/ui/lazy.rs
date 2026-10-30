//! A window part built the first time it's used rather than at startup, as
//! Fedra does with its dialogs: a dialog nobody opens never costs its native
//! controls. Reading a field through it (`parts.poll_dialog.send_btn`) builds
//! it then; handlers bound at startup wait in `when_built`.

use std::{
	cell::{OnceCell, RefCell},
	ops::Deref,
};

type Hook<T> = Box<dyn FnOnce(&T)>;

pub struct Lazy<T> {
	cell: OnceCell<T>,
	build: RefCell<Option<Box<dyn FnOnce() -> T>>>,
	hooks: RefCell<Vec<Hook<T>>>,
}

impl<T> Lazy<T> {
	pub fn new(build: impl FnOnce() -> T + 'static) -> Self {
		Self { cell: OnceCell::new(), build: RefCell::new(Some(Box::new(build))), hooks: RefCell::new(Vec::new()) }
	}

	/// The part, if something has needed it yet.
	pub fn get(&self) -> Option<&T> {
		self.cell.get()
	}

	/// Runs `hook` on the part once it's built (now, if it already is).
	pub fn when_built(&self, hook: impl FnOnce(&T) + 'static) {
		match self.cell.get() {
			Some(part) => hook(part),
			None => self.hooks.borrow_mut().push(Box::new(hook)),
		}
	}

	fn force(&self) -> &T {
		if let Some(part) = self.cell.get() {
			return part;
		}
		let build = self.build.borrow_mut().take().expect("window part built while it was being built");
		let part = self.cell.get_or_init(build);
		// So the logs show what got built when, and anything built early.
		let name = std::any::type_name::<T>().rsplit("::").next().unwrap_or_default();
		crate::websocket_events::ui_timing_log(&format!("Built {name}"));
		// Taken one at a time: a hook may itself add another.
		loop {
			let Some(hook) = ({
				let mut hooks = self.hooks.borrow_mut();
				(!hooks.is_empty()).then(|| hooks.remove(0))
			}) else {
				break;
			};
			hook(part);
		}
		part
	}
}

impl<T> Deref for Lazy<T> {
	type Target = T;

	fn deref(&self) -> &T {
		self.force()
	}
}
