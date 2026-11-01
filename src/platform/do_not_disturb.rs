use std::{
	sync::Mutex,
	time::{Duration, Instant},
};

use windows::Win32::UI::Shell::SHQueryUserNotificationState;

/// `WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED`: the state the shell updates
/// when the active Do Not Disturb / Focus Assist profile changes. Its DWORD is
/// 0 when unrestricted, non-zero for a restrictive profile. Undocumented, but
/// stable across Windows 10 1803 through 11, and unlike the legacy API it
/// actually tracks the modern Do Not Disturb switch.
const WNF_QUIET_HOURS_ACTIVE_PROFILE: u64 = 0x0D83_063E_A3BF_1C75;

/// Windows' Do Not Disturb state doesn't change on a sub-second scale, and the
/// probes run on the UI thread for every background message, so a burst of
/// messages reuses one reading.
const CACHE_TTL: Duration = Duration::from_secs(1);

static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

#[link(name = "ntdll")]
unsafe extern "system" {
	fn NtQueryWnfStateData(
		state_name: *const u64,
		type_id: *const core::ffi::c_void,
		explicit_scope: *const core::ffi::c_void,
		change_stamp: *mut u32,
		buffer: *mut core::ffi::c_void,
		buffer_size: *mut u32,
	) -> i32;
}

fn query_focus_assist() -> Option<bool> {
	let mut change_stamp = 0u32;
	let mut value = 0u32;
	let mut size = size_of::<u32>() as u32;
	let status = unsafe {
		NtQueryWnfStateData(
			&WNF_QUIET_HOURS_ACTIVE_PROFILE,
			std::ptr::null(),
			std::ptr::null(),
			&mut change_stamp,
			(&mut value as *mut u32).cast(),
			&mut size,
		)
	};
	(status == 0 && size as usize >= size_of::<u32>()).then_some(value != 0)
}

fn query_notification_state() -> Option<i32> {
	unsafe { SHQueryUserNotificationState() }.ok().map(|state| state.0)
}

pub fn should_silence(settings: &crate::settings::Settings) -> bool {
	settings.follow_windows_do_not_disturb && windows_is_quieting()
}

fn windows_is_quieting() -> bool {
	let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
	if let Some((at, answer)) = *cache
		&& at.elapsed() < CACHE_TTL
	{
		return answer;
	}
	let answer = whatsnap::windows_is_quieting(query_focus_assist(), query_notification_state());
	*cache = Some((Instant::now(), answer));
	answer
}
