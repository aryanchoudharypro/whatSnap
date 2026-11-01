use windows::{
	Win32::Media::MediaFoundation::{
		IMFActivate, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
		MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
		MFCreateAttributes, MFEnumDeviceSources,
	},
	core::PWSTR,
};

use crate::media::foundation;

pub struct CameraDevice {
	pub name: String,
	pub symbolic_link: String,
}

fn read_allocated_string(activate: &IMFActivate, key: &windows::core::GUID) -> Option<String> {
	unsafe {
		let mut ptr = PWSTR::null();
		let mut len: u32 = 0;
		activate.GetAllocatedString(key, &mut ptr, &mut len).ok()?;
		if ptr.is_null() {
			return None;
		}
		let s = ptr.to_string().ok();
		windows::Win32::System::Com::CoTaskMemFree(Some(ptr.0 as *const core::ffi::c_void));
		s
	}
}

pub fn list_cameras() -> Vec<CameraDevice> {
	foundation::ensure_started();
	unsafe {
		let mut attributes: Option<windows::Win32::Media::MediaFoundation::IMFAttributes> = None;
		if MFCreateAttributes(&mut attributes, 1).is_err() {
			return Vec::new();
		}
		let Some(attributes) = attributes else { return Vec::new() };
		if attributes
			.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)
			.is_err()
		{
			return Vec::new();
		}

		let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
		let mut count: u32 = 0;
		if MFEnumDeviceSources(&attributes, &mut activates, &mut count).is_err() || activates.is_null() {
			return Vec::new();
		}

		let slice = std::slice::from_raw_parts(activates, count as usize);
		let mut devices = Vec::with_capacity(count as usize);
		for entry in slice {
			let Some(activate) = entry else { continue };
			let Some(name) = read_allocated_string(activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME) else { continue };
			let Some(symbolic_link) =
				read_allocated_string(activate, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)
			else {
				continue;
			};
			devices.push(CameraDevice { name, symbolic_link });
		}

		windows::Win32::System::Com::CoTaskMemFree(Some(activates as *const core::ffi::c_void));
		devices
	}
}

pub fn camera_names() -> Vec<String> {
	list_cameras().into_iter().map(|d| d.name).collect()
}

pub fn resolve_camera_symbolic_link(name: Option<&str>) -> Option<String> {
	let devices = list_cameras();
	if let Some(name) = name
		&& let Some(d) = devices.iter().find(|d| d.name == name)
	{
		return Some(d.symbolic_link.clone());
	}
	devices.into_iter().next().map(|d| d.symbolic_link)
}
