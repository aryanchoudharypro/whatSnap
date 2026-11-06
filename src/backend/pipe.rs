use std::{
	io::{self, Read, Write},
	sync::Arc,
};

use windows::{
	Win32::{
		Foundation::{
			CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_PIPE_BUSY,
			ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
		},
		Storage::FileSystem::{
			CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, FlushFileBuffers,
			OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
		},
		System::{
			IO::{GetOverlappedResult, OVERLAPPED},
			Pipes::{
				ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
				PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
			},
			Threading::CreateEventW,
		},
	},
	core::{HRESULT, HSTRING},
};

struct OwnedHandle(HANDLE);

unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
	fn drop(&mut self) {
		unsafe {
			let _ = CloseHandle(self.0);
		}
	}
}

fn pipe_path(name: &str) -> HSTRING {
	HSTRING::from(format!(r"\\.\pipe\{name}"))
}

fn io_error(e: windows::core::Error) -> io::Error {
	let hr = e.code().0 as u32;
	if hr >> 16 == 0x8007 { io::Error::from_raw_os_error((hr & 0xffff) as i32) } else { io::Error::other(e) }
}

fn is(e: &windows::core::Error, code: windows::Win32::Foundation::WIN32_ERROR) -> bool {
	e.code() == HRESULT::from_win32(code.0)
}

pub(crate) struct PipeStream {
	pipe: Arc<OwnedHandle>,
	event: OwnedHandle,
}

impl PipeStream {
	fn new(pipe: Arc<OwnedHandle>) -> io::Result<Self> {
		let event = unsafe { CreateEventW(None, false, false, None) }.map_err(io_error)?;
		Ok(PipeStream { pipe, event: OwnedHandle(event) })
	}

	pub(crate) fn connect(name: &str) -> io::Result<Self> {
		let path = pipe_path(name);
		let mut waited = false;
		loop {
			let handle = unsafe {
				CreateFileW(
					&path,
					(GENERIC_READ | GENERIC_WRITE).0,
					FILE_SHARE_NONE,
					None,
					OPEN_EXISTING,
					// The server may identify us but never act as us.
					FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
					None,
				)
			};
			match handle {
				Ok(h) => return PipeStream::new(Arc::new(OwnedHandle(h))),
				Err(e) if is(&e, ERROR_PIPE_BUSY) && !waited => {
					waited = true;
					let _ = unsafe { WaitNamedPipeW(&path, 2000) };
				}
				Err(e) => return Err(io_error(e)),
			}
		}
	}

	pub(crate) fn try_clone(&self) -> io::Result<Self> {
		PipeStream::new(self.pipe.clone())
	}

	pub(crate) fn server_pid(&self) -> Option<u32> {
		let mut pid = 0;
		unsafe { GetNamedPipeServerProcessId(self.pipe.0, &mut pid) }.ok()?;
		Some(pid)
	}

	fn wait(&self, start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>) -> windows::core::Result<u32> {
		let mut ov = OVERLAPPED { hEvent: self.event.0, ..Default::default() };
		match start(&mut ov) {
			Ok(()) => {}
			Err(e) if is(&e, ERROR_IO_PENDING) => {}
			Err(e) => return Err(e),
		}
		let mut n = 0;
		unsafe { GetOverlappedResult(self.pipe.0, &ov, &mut n, true) }.map(|()| n)
	}
}

impl Read for PipeStream {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		if buf.is_empty() {
			return Ok(0);
		}
		let len = buf.len().min(u32::MAX as usize);
		let pipe = self.pipe.0;
		match self.wait(|ov| unsafe { ReadFile(pipe, Some(&mut buf[..len]), None, Some(ov)) }) {
			Ok(n) => Ok(n as usize),
			Err(e) if is(&e, ERROR_BROKEN_PIPE) => Ok(0),
			Err(e) if is(&e, ERROR_MORE_DATA) => Ok(len),
			Err(e) => Err(io_error(e)),
		}
	}
}

impl Write for PipeStream {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let len = buf.len().min(u32::MAX as usize);
		let pipe = self.pipe.0;
		self.wait(|ov| unsafe { WriteFile(pipe, Some(&buf[..len]), None, Some(ov)) })
			.map(|n| n as usize)
			.map_err(io_error)
	}

	fn flush(&mut self) -> io::Result<()> {
		unsafe { FlushFileBuffers(self.pipe.0) }.map_err(io_error)
	}
}

/// The server end of a pipe, always holding a free instance so a
/// client never finds the name missing between two connections.
pub(crate) struct PipeListener {
	path: HSTRING,
	next: OwnedHandle,
	event: OwnedHandle,
}

impl PipeListener {
	pub(crate) fn bind(name: &str) -> io::Result<Self> {
		let path = pipe_path(name);
		let next = Self::instance(&path, true)?;
		let event = OwnedHandle(unsafe { CreateEventW(None, false, false, None) }.map_err(io_error)?);
		Ok(PipeListener { path, next, event })
	}

	fn instance(path: &HSTRING, first: bool) -> io::Result<OwnedHandle> {
		let mut mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
		if first {
			mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
		}
		let handle = unsafe {
			CreateNamedPipeW(
				path,
				mode,
				PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
				PIPE_UNLIMITED_INSTANCES,
				4096,
				4096,
				0,
				None,
			)
		};
		if handle == INVALID_HANDLE_VALUE {
			return Err(io::Error::last_os_error());
		}
		Ok(OwnedHandle(handle))
	}

	pub(crate) fn accept(&mut self) -> io::Result<PipeStream> {
		let mut ov = OVERLAPPED { hEvent: self.event.0, ..Default::default() };
		match unsafe { ConnectNamedPipe(self.next.0, Some(&mut ov)) } {
			Ok(()) => {}
			Err(e) if is(&e, ERROR_PIPE_CONNECTED) || is(&e, ERROR_NO_DATA) => {}
			Err(e) if is(&e, ERROR_IO_PENDING) => {
				let mut n = 0;
				unsafe { GetOverlappedResult(self.next.0, &ov, &mut n, true) }.map_err(io_error)?;
			}
			Err(e) => {
				self.next = Self::instance(&self.path, false)?;
				return Err(io_error(e));
			}
		}
		let connected = std::mem::replace(&mut self.next, Self::instance(&self.path, false)?);
		PipeStream::new(Arc::new(connected))
	}
}
