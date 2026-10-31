//! Update checks, through ship-shape: the stable channel offers the newest
//! numbered release on GitHub, the dev channel the rolling Development Build
//! (the `latest` release CI replaces on every push to master).
//!
//! Releases carry `whatSnap_setup.exe` and `whatSnap.zip`, each with a
//! `.minisig` beside it made with the key below; ship-shape installs nothing
//! whose signature doesn't verify. CI signs with the `MINISIGN_KEY` and
//! `MINISIGN_PASSWORD` secrets (see `xtask`).

use std::sync::Arc;

use ship_shape::{InstallKind, UpdateChannel, UpdaterConfig, ui::CheckTrigger};
use wxdragon::prelude::*;

use crate::{UiCommand, ui_wake::UiCommandSender};

const GITHUB_REPO: &str = "aryanchoudharypro/whatSnap";
const MINISIGN_PUBLIC_KEY: &str = "RWRUQ+Jzp5ml2vlGDcLi1g+HxqNGOYXekOc/l1DQ264MgZP8C9EZZSKM";

/// How this copy was installed. The installer leaves Inno Setup's
/// uninstaller next to the exe; a copy unpacked from the zip has none.
fn install_kind() -> InstallKind {
	let installed = std::env::current_exe()
		.ok()
		.and_then(|exe| exe.parent().map(|dir| dir.join("unins000.exe")))
		.is_some_and(|uninstaller| uninstaller.exists());
	if installed { InstallKind::Installer } else { InstallKind::Portable }
}

/// Checks for an update on `channel` (`"stable"` or `"dev"`). `automatic`
/// is the startup check, which only speaks up when there is an update;
/// otherwise it also says when whatSnap is up to date or the check failed.
///
/// When an update is ready to install, whatSnap quits the way tray Exit
/// does: closing the window alone only hides it to the tray.
pub fn run(frame: &Frame, ui_tx: &UiCommandSender, channel: &str, automatic: bool) {
	let config = Arc::new(
		UpdaterConfig::new(GITHUB_REPO, "whatSnap", "whatSnap", MINISIGN_PUBLIC_KEY, env!("CARGO_PKG_VERSION"))
			.with_commit(env!("WHATSNAP_COMMIT_HASH"))
			.with_install_kind(install_kind()),
	);
	let channel = if channel == "dev" { UpdateChannel::Dev } else { UpdateChannel::Stable };
	let trigger = if automatic { CheckTrigger::Automatic } else { CheckTrigger::Manual };
	let tx = ui_tx.clone();
	ship_shape::ui::run_update_check_with_exit_handler(config, frame, channel, trigger, move || {
		let _ = tx.send(UiCommand::ExitApp);
	});
}
