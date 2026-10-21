//! Leaves a trace when whaNTV goes down. Panics are appended to panic.log,
//! with when and on which thread, instead of each one overwriting the last
//! (quitting the app can itself panic while Windows tears its threads down,
//! which used to replace the crash that mattered). Native faults, such as an
//! access violation inside Windows or a driver, never reach the panic hook,
//! so an unhandled-exception filter writes their code and the module they
//! happened in to crash.log. Code that ends the process straight away, or a
//! library that replaces that filter, would skip it, so a vectored handler
//! also notes the first time each kind of fatal fault is raised at all.

use std::{
	io::Write,
	sync::atomic::{AtomicU32, Ordering},
};

use windows::Win32::{
	Foundation::HMODULE,
	System::{
		Diagnostics::Debug::{
			AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, RtlCaptureStackBackTrace,
				SetUnhandledExceptionFilter,
		},
		LibraryLoader::{
			GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
			GetModuleHandleExW,
		},
	},
};
use windows::core::PCWSTR;

use crate::paths;

pub(crate) fn install() {
	std::panic::set_hook(Box::new(|info| append("panic.log", &format!("{info}"))));
	unsafe {
		SetUnhandledExceptionFilter(Some(on_native_crash));
		AddVectoredExceptionHandler(1, Some(on_fault_raised));
	}
}

/// Faults that end a process unless something catches them: access
/// violation, illegal instruction, stack overflow, heap corruption, and a
/// stack buffer overrun or other fail-fast.
const FATAL_CODES: [u32; 5] = [0xC000_0005, 0xC000_001D, 0xC000_00FD, 0xC000_0374, 0xC000_0409];
/// One bit per entry of `FATAL_CODES`, set once that kind has been logged:
/// some libraries raise and catch access violations on purpose.
static RAISED: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn on_fault_raised(info: *mut EXCEPTION_POINTERS) -> i32 {
	if let Some(record) = unsafe { info.as_ref().and_then(|info| info.ExceptionRecord.as_ref()) } {
		let code = record.ExceptionCode.0 as u32;
		if let Some(i) = FATAL_CODES.iter().position(|&c| c == code)
			&& RAISED.fetch_or(1 << i, Ordering::Relaxed) & (1 << i) == 0
		{
			let address = record.ExceptionAddress as usize;
			// For an access violation: 0 read, 1 write, 8 execute, then the address.
			let info = &record.ExceptionInformation[..(record.NumberParameters as usize).min(2)];
			append(
				"crash.log",
				&format!("fault {code:#010x} raised at {} ({info:x?}), stack:{}", describe(address), stack()),
			);
		}
	}
	EXCEPTION_CONTINUE_SEARCH
}

/// The calling thread's stack as module+offset frames; from inside the
/// handler that runs through the exception dispatcher into the faulting code.
fn stack() -> String {
	let mut frames = [std::ptr::null_mut(); 32];
	let count = unsafe { RtlCaptureStackBackTrace(0, &mut frames, None) } as usize;
	frames[..count].iter().map(|&frame| format!("\n    {}", describe(frame as usize))).collect()
}

fn describe(address: usize) -> String {
	match module_of(address) {
		Some((name, base)) => format!("{name}+{:#x}", address - base),
		None => format!("{address:#x}"),
	}
}

fn append(file: &str, what: &str) {
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
	let thread = std::thread::current();
	let line = format!(
		"[{}.{:03}] thread {}: {what}\n",
		now.as_secs(),
		now.subsec_millis(),
		thread.name().unwrap_or("unnamed")
	);
	if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(paths::log_dir().join(file)) {
		let _ = f.write_all(line.as_bytes());
	}
}

unsafe extern "system" fn on_native_crash(info: *const EXCEPTION_POINTERS) -> i32 {
	let record = unsafe { info.as_ref().and_then(|info| info.ExceptionRecord.as_ref()) };
	if let Some(record) = record {
		let place = describe(record.ExceptionAddress as usize);
		append("crash.log", &format!("unhandled exception {:#010x} at {place}", record.ExceptionCode.0 as u32));
	}
	// Windows still reports the crash and ends the process as before.
	EXCEPTION_CONTINUE_SEARCH
}

/// The file name and load address of the module holding `address`.
fn module_of(address: usize) -> Option<(String, usize)> {
	unsafe {
		let mut module = HMODULE::default();
		GetModuleHandleExW(
			GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
			PCWSTR(address as *const u16),
			&mut module,
		)
		.ok()?;
		let mut path = [0u16; 260];
		let len = GetModuleFileNameW(Some(module), &mut path) as usize;
		let path = String::from_utf16_lossy(&path[..len.min(path.len())]);
		let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
		Some((name, module.0 as usize))
	}
}
