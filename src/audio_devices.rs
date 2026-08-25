//! Lists audio input/output devices for the Settings "Input & Output" tab, and
//! resolves a saved device name back to a live device at stream-open time.
//!
//! Devices are identified by their friendly name (e.g. "Microphone (Realtek
//! Audio)") rather than a WASAPI endpoint ID, because the two audio backends
//! this app uses - raw WASAPI in `call_audio.rs` for calls, and cpal (via
//! rodio) for voice-message/notification playback - don't share an ID
//! namespace; friendly name is the only identity both can resolve a device
//! from. Duplicate friendly names on one machine are rare enough in practice
//! not to worry about here.

use rodio::cpal::{
	self,
	traits::{DeviceTrait, HostTrait},
};
use windows::{
	Win32::{
		Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
		Media::Audio::{DEVICE_STATE_ACTIVE, EDataFlow, IMMDevice, IMMDeviceEnumerator},
		System::Com::{STGM_READ, StructuredStorage::PropVariantToStringAlloc},
	},
	core::PWSTR,
};

/// "System Default" sentinel shown as the first entry in both device combos;
/// stored in Settings as `None`.
pub const DEFAULT_LABEL: &str = "System Default";

pub fn input_device_names() -> Vec<String> {
	device_names(true)
}

pub fn output_device_names() -> Vec<String> {
	device_names(false)
}

fn device_name(device: &cpal::Device) -> Option<String> {
	device.description().ok().map(|d| d.name().to_string())
}

fn device_names(input: bool) -> Vec<String> {
	let host = cpal::default_host();
	let devices = if input {
		host.input_devices().map(|d| d.collect::<Vec<_>>())
	} else {
		host.output_devices().map(|d| d.collect::<Vec<_>>())
	};
	devices.unwrap_or_default().iter().filter_map(device_name).collect()
}

/// Resolves a saved output device name to a live cpal device, falling back to
/// the system default if `name` is `None` or the named device is no longer
/// present (unplugged, renamed, etc).
pub fn resolve_output_device(name: Option<&str>) -> Option<cpal::Device> {
	let host = cpal::default_host();
	if let Some(name) = name {
		if let Ok(devices) = host.output_devices() {
			if let Some(d) = devices.into_iter().find(|d| device_name(d).as_deref() == Some(name)) {
				return Some(d);
			}
		}
	}
	host.default_output_device()
}

/// Finds the WASAPI endpoint whose friendly name matches `name` among active
/// devices for `dataflow`. Returns `None` for the "use system default"
/// selection, or if the named device is no longer present - callers should
/// fall back to `IMMDeviceEnumerator::GetDefaultAudioEndpoint` in that case.
pub unsafe fn find_wasapi_device_by_name(
	enumerator: &IMMDeviceEnumerator,
	dataflow: EDataFlow,
	name: &str,
) -> Option<IMMDevice> {
	unsafe {
		let collection = enumerator.EnumAudioEndpoints(dataflow, DEVICE_STATE_ACTIVE).ok()?;
		let count = collection.GetCount().ok()?;
		for i in 0..count {
			let device = collection.Item(i).ok()?;
			if device_friendly_name(&device).as_deref() == Some(name) {
				return Some(device);
			}
		}
		None
	}
}

unsafe fn device_friendly_name(device: &IMMDevice) -> Option<String> {
	unsafe {
		let store = device.OpenPropertyStore(STGM_READ).ok()?;
		let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
		let out: PWSTR = PropVariantToStringAlloc(&value).ok()?;
		if out.is_null() {
			return None;
		}
		let name = out.to_string().ok();
		windows::Win32::System::Com::CoTaskMemFree(Some(out.0 as *const core::ffi::c_void));
		name
	}
}
