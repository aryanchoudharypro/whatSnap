use std::sync::Arc;

use ship_shape::{InstallKind, UpdateChannel, UpdaterConfig, ui::CheckTrigger};
use wxdragon::prelude::*;

use crate::{UiCommand, ui::wake::UiCommandSender};

const GITHUB_REPO: &str = "aryanchoudharypro/whatSnap";
const MINISIGN_PUBLIC_KEY: &str = "RWRUQ+Jzp5ml2vlGDcLi1g+HxqNGOYXekOc/l1DQ264MgZP8C9EZZSKM";

fn install_kind() -> InstallKind {
	let installed = std::env::current_exe()
		.ok()
		.and_then(|exe| exe.parent().map(|dir| dir.join("unins000.exe")))
		.is_some_and(|uninstaller| uninstaller.exists());
	if installed { InstallKind::Installer } else { InstallKind::Portable }
}

pub fn run(frame: &Frame, ui_tx: &UiCommandSender, channel: &str, automatic: bool) {
	let config =
		UpdaterConfig::new(GITHUB_REPO, "whatSnap", "whatSnap", MINISIGN_PUBLIC_KEY, env!("CARGO_PKG_VERSION"))
			.with_commit(env!("WHATSNAP_COMMIT_HASH"))
			.with_install_kind(install_kind());
	// The arm64 packages are whatSnap-arm64.zip and whatSnap_setup-arm64.exe; x64 stays unsuffixed.
	#[cfg(target_arch = "aarch64")]
	let config = config.with_asset_suffix("-arm64");
	let config = Arc::new(config);
	let channel = if channel == "dev" { UpdateChannel::Dev } else { UpdateChannel::Stable };
	let trigger = if automatic { CheckTrigger::Automatic } else { CheckTrigger::Manual };
	let tx = ui_tx.clone();
	ship_shape::ui::run_update_check_with_exit_handler(config, frame, channel, trigger, move || {
		let _ = tx.send(UiCommand::ExitApp);
	});
}
