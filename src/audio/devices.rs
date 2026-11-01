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

pub const DEFAULT_LABEL: &str = "System Default";

pub fn input_device_names() -> Vec<String> {
	device_names(true)
}

pub fn output_device_names() -> Vec<String> {
	device_names(false)
}

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
	// cpal panics if a device goes away mid-listing, as a headset's call
	// endpoint does just after a call. Settings must still open.
	std::panic::catch_unwind(|| list_device_names(input)).unwrap_or_default()
}

fn list_device_names(input: bool) -> Vec<String> {
	let host = cpal::default_host();
	let names_by_id = wasapi_friendly_names_by_id(if input { eCapture } else { eRender });
	let devices = if input {
		host.input_devices().map(|d| d.collect::<Vec<_>>())
	} else {
		host.output_devices().map(|d| d.collect::<Vec<_>>())
	};
	devices.unwrap_or_default().iter().filter_map(|d| device_name(d, &names_by_id)).collect()
}

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

fn communications_default_output_name() -> Option<String> {
	use windows::Win32::{
		Media::Audio::{MMDeviceEnumerator, eCommunications, eRender},
		System::Com::{CLSCTX_ALL, CoCreateInstance},
	};
	unsafe {
		let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
		let device = enumerator.GetDefaultAudioEndpoint(eRender, eCommunications).ok()?;
		device_friendly_name(&device)
	}
}

pub fn resolve_output_device_name(name: Option<&str>) -> Option<String> {
	let names_by_id = wasapi_friendly_names_by_id(eRender);
	resolve_output_device(name).and_then(|d| device_name(&d, &names_by_id))
}

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
