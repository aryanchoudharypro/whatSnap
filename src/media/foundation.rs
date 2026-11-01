use std::sync::Once;

use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_FULL, MFStartup};

static MF_STARTUP: Once = Once::new();

pub fn ensure_started() {
	MF_STARTUP.call_once(|| unsafe {
		// Intentionally never call MFShutdown: capture/encode sessions come
		// and go for the lifetime of the app, and MFStartup is documented as
		// safe to leave active until process exit.
		let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
	});
}

pub fn pack_u64(high: u32, low: u32) -> u64 {
	((high as u64) << 32) | (low as u64)
}
