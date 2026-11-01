//! Lists audio input/output devices for the Settings "Input & Output" tab, and
//! resolves a saved device name back to a live device at stream-open time.
//!
//! Devices are identified by their WASAPI friendly name (e.g. "Microphone
//! (Realtek(R) Audio)") everywhere - both audio backends this app uses (raw
//! WASAPI in `calls/audio.rs`/`audio/voice_recorder.rs` for calls and voice-message
//! recording, and cpal via rodio for voice-message/notification playback)
//! must agree on it, or a device picked in a combo silently fails to match
//! and the call/recording side falls back to the default device instead.
//! cpal's own `Device::description()`/`name()` are NOT used for this: on
//! Windows they prefer the driver's terse `DeviceDesc` property ("Microphone")
//! over `FriendlyName`, which doesn't match what `find_wasapi_device_by_name`
//! searches for. Duplicate friendly names on one machine are rare enough in
//! practice not to worry about here.

use std::collections::HashMap;

use rodio::cpal::{
	self,
	traits::{DeviceTrait, HostTrait},
};
use windows::{
	Win32::{
		Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
		Media::Audio::{DEVICE_STATE_ACTIVE, EDataFlow, IMMDevice, IMMDeviceEnumerator, eCapture, eRender},
		System::Com::{
			CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ, StructuredStorage::PropVariantToStringAlloc,
		},
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

/// Maps every active WASAPI endpoint's stable device ID (what
/// `cpal::Device::id()` also returns on the WASAPI backend - both call the
/// same underlying `IMMDevice::GetId`) to its friendly name, so a cpal
/// device can be named the same way `find_wasapi_device_by_name` searches.
fn wasapi_friendly_names_by_id(dataflow: EDataFlow) -> HashMap<String, String> {
	let mut map = HashMap::new();
	unsafe {
		use windows::Win32::Media::Audio::MMDeviceEnumerator;
		let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
			return map;
		};
		let Ok(collection) = enumerator.EnumAudioEndpoints(dataflow, DEVICE_STATE_ACTIVE) else {
			return map;
		};
		let Ok(count) = collection.GetCount() else {
			return map;
		};
		for i in 0..count {
			let Ok(device) = collection.Item(i) else { continue };
			let Ok(id_pwstr) = device.GetId() else { continue };
			let id_str = id_pwstr.to_string().ok();
			CoTaskMemFree(Some(id_pwstr.0 as *const core::ffi::c_void));
			let (Some(id_str), Some(name)) = (id_str, device_friendly_name(&device)) else { continue };
			map.insert(id_str, name);
		}
	}
	map
}

fn device_name(device: &cpal::Device, names_by_id: &HashMap<String, String>) -> Option<String> {
	let id = device.id().ok()?.1;
	names_by_id.get(&id).cloned()
}

fn device_names(input: bool) -> Vec<String> {
	let host = cpal::default_host();
	let names_by_id = wasapi_friendly_names_by_id(if input { eCapture } else { eRender });
	let devices = if input {
		host.input_devices().map(|d| d.collect::<Vec<_>>())
	} else {
		host.output_devices().map(|d| d.collect::<Vec<_>>())
	};
	devices.unwrap_or_default().iter().filter_map(|d| device_name(d, &names_by_id)).collect()
}

/// Resolves a saved output device name to a live cpal device, falling back to
/// the system default if `name` is `None` or the named device is no longer
/// present (unplugged, renamed, etc).
///
/// "System default" is resolved via the WASAPI **Communications**-role
/// default rather than cpal's own `default_output_device()` (which follows
/// the Console/Multimedia role). Windows tracks these roles independently -
/// plugging in a headset commonly repoints only the Communications default,
/// leaving Console pointed at the speakers - and calls (`calls/audio.rs`)
/// already use the Communications role. Resolving playback the same way
/// keeps voice-message/notification audio on the same device calls use,
/// instead of silently falling back to the speakers.
pub fn resolve_output_device(name: Option<&str>) -> Option<cpal::Device> {
	let host = cpal::default_host();
	let names_by_id = wasapi_friendly_names_by_id(eRender);
	if let Some(name) = name
		&& let Ok(devices) = host.output_devices()
		&& let Some(d) = devices.into_iter().find(|d| device_name(d, &names_by_id).as_deref() == Some(name))
	{
		return Some(d);
	}
	if let Some(comms_name) = communications_default_output_name()
		&& let Ok(devices) = host.output_devices()
		&& let Some(d) =
			devices.into_iter().find(|d| device_name(d, &names_by_id).as_deref() == Some(comms_name.as_str()))
	{
		return Some(d);
	}
	host.default_output_device()
}

/// The friendly name of the WASAPI Communications-role default render
/// device, or `None` if it can't be determined (COM not initialized on this
/// thread, no active render endpoint, etc) - callers should fall back to
/// cpal's own default in that case.
fn communications_default_output_name() -> Option<String> {
	use windows::Win32::{
		Media::Audio::{MMDeviceEnumerator, eCommunications, eRender},
		System::Com::{CLSCTX_ALL, CoCreateInstance},
	};
	unsafe {
		// CoCreateInstance works whether or not this thread has already
		// CoInitializeEx'd (it will if it's the wx UI thread); if COM truly
		// isn't initialized here this just fails harmlessly and the caller
		// falls back to cpal's default.
		let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
		let device = enumerator.GetDefaultAudioEndpoint(eRender, eCommunications).ok()?;
		device_friendly_name(&device)
	}
}

/// The friendly name `resolve_output_device(name)` would currently resolve
/// to - cheap enough to call before every playback to detect whether a
/// cached output stream needs reopening (e.g. the "System Default" device
/// changed because headphones were unplugged/replugged; a stream opened
/// against one concrete endpoint doesn't follow that change on its own).
pub fn resolve_output_device_name(name: Option<&str>) -> Option<String> {
	let names_by_id = wasapi_friendly_names_by_id(eRender);
	resolve_output_device(name).and_then(|d| device_name(&d, &names_by_id))
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
