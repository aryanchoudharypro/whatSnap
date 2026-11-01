//! Watches for Windows' live "default audio device changed" notifications
//! (headphones plugged/unplugged, a new default picked in Windows Settings)
//! and posts `UiCommand::AudioDeviceChanged` so playback and any active call
//! can react immediately, instead of only noticing on the next unrelated
//! action (a new play, a stream error). This is the mechanism WASAPI expects
//! apps to use for "follow the default device" behavior - a shared-mode
//! stream never migrates to a new device on its own.

use windows::{
	Win32::{
		Foundation::PROPERTYKEY,
		Media::Audio::{
			DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
			MMDeviceEnumerator, eCapture, eCommunications, eConsole, eRender,
		},
		System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
	},
	core::{PCWSTR, Result as WinResult},
};

use crate::{UiCommand, ui::wake::UiCommandSender};

#[windows::core::implement(IMMNotificationClient)]
struct Watcher {
	ui_tx: UiCommandSender,
}

impl IMMNotificationClient_Impl for Watcher_Impl {
	fn OnDeviceStateChanged(&self, _device_id: &PCWSTR, _new_state: DEVICE_STATE) -> WinResult<()> {
		Ok(())
	}

	fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> WinResult<()> {
		Ok(())
	}

	fn OnDeviceRemoved(&self, _device_id: &PCWSTR) -> WinResult<()> {
		Ok(())
	}

	fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _default_device_id: &PCWSTR) -> WinResult<()> {
		// Console and Communications are the two roles this app ever resolves
		// against - playback follows Console (via cpal's default) and calls
		// always use Communications (see audio/devices.rs and calls/audio.rs).
		// Multimedia-role changes are ignored: nothing here reads that role,
		// and Windows fires a change for nearly every role anyway, so this
		// just avoids a redundant extra notification.
		if (role == eConsole || role == eCommunications) && (flow == eRender || flow == eCapture) {
			let _ = self.ui_tx.send(UiCommand::AudioDeviceChanged);
		}
		Ok(())
	}

	fn OnPropertyValueChanged(&self, _device_id: &PCWSTR, _key: &PROPERTYKEY) -> WinResult<()> {
		Ok(())
	}
}

/// Spawns a background thread that registers for live default-device-change
/// notifications and parks for the lifetime of the app. The registration -
/// and the enumerator/client keeping it alive - must outlive the thread that
/// created it, so this thread just blocks forever after registering instead
/// of returning and dropping them (which would unregister the callback).
pub fn start(ui_tx: UiCommandSender) {
	std::thread::spawn(move || unsafe {
		if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
			return;
		}
		let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
			return;
		};
		let client: IMMNotificationClient = Watcher { ui_tx }.into();
		if enumerator.RegisterEndpointNotificationCallback(&client).is_err() {
			return;
		}
		// Notifications arrive on their own MMDevAPI-owned thread; this
		// thread's only remaining job is to keep `enumerator`/`client` alive.
		loop {
			std::thread::park();
		}
	});
}
