#![cfg(target_os = "windows")]

use std::os::windows::io::AsRawHandle;

use windows::Win32::{
	Foundation::HANDLE,
	System::JobObjects::{
		AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
		JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
		SetInformationJobObject,
	},
};

pub struct KillOnCloseJob {
	handle: HANDLE,
}

impl KillOnCloseJob {
	pub fn new() -> windows::core::Result<Self> {
		let handle = unsafe { CreateJobObjectW(None, None)? };

		let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
			BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
				LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
				..Default::default()
			},
			..Default::default()
		};

		unsafe {
			SetInformationJobObject(
				handle,
				JobObjectExtendedLimitInformation,
				std::ptr::from_mut(&mut info).cast(),
				u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
			)?;
		}

		Ok(Self { handle })
	}

	pub fn assign(&self, child: &std::process::Child) -> windows::core::Result<()> {
		unsafe { AssignProcessToJobObject(self.handle, HANDLE(child.as_raw_handle())) }
	}
}

impl Drop for KillOnCloseJob {
	fn drop(&mut self) {
		unsafe {
			let _ = windows::Win32::Foundation::CloseHandle(self.handle);
		}
	}
}

unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}
