//! Process-wide Media Foundation startup, shared by camera enumeration
//! (`calls/video_devices.rs`) and capture/encode (`calls/video_send.rs`). MFStartup
//! is reference-counted internally by MF itself, but callers still need to
//! call it before touching any MF API and are expected to pair it with
//! MFShutdown - `std::sync::Once` here just makes "call it once, from
//! whichever of those two modules gets there first" safe without every
//! caller having to coordinate.

use std::sync::Once;

use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_FULL, MFStartup};

static MF_STARTUP: Once = Once::new();

/// Ensures Media Foundation is started on this process. Safe to call from
/// any thread, any number of times.
pub fn ensure_started() {
	MF_STARTUP.call_once(|| unsafe {
		// Intentionally never call MFShutdown: capture/encode sessions come
		// and go for the lifetime of the app, and MFStartup is documented as
		// safe to leave active until process exit.
		let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
	});
}

/// Packs two u32 halves into the u64 MF uses for `MF_MT_FRAME_SIZE`
/// (width/height) and `MF_MT_FRAME_RATE` (numerator/denominator).
pub fn pack_u64(high: u32, low: u32) -> u64 {
	((high as u64) << 32) | (low as u64)
}
