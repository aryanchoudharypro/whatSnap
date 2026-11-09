use std::{
	path::Path,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
};

use ship_shape::{
	InstallKind, UpdateAvailableResult, UpdateChannel, UpdateCheckOutcome, UpdateError, UpdaterConfig,
};
use wx_utils::progress::{Ended, Progress, run_with_progress};
use wxdragon::{ffi, prelude::*, window::WxWidget};

use crate::{UiCommand, ui::theme, ui::wake::UiCommandSender};

const GITHUB_REPO: &str = "aryanchoudharypro/whatSnap";
const MINISIGN_PUBLIC_KEY: &str = "RWRUQ+Jzp5ml2vlGDcLi1g+HxqNGOYXekOc/l1DQ264MgZP8C9EZZSKM";

static UPDATE_CHECK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Proof that this flow owns `UPDATE_CHECK_ACTIVE`.
/// Releases it when dropped, so every exit path frees the next check.
pub struct ActiveCheckGuard;

impl Drop for ActiveCheckGuard {
	fn drop(&mut self) {
		UPDATE_CHECK_ACTIVE.store(false, Ordering::SeqCst);
	}
}

/// A window handle that can cross threads and be dereferenced on the main UI thread.
#[derive(Clone, Copy)]
pub struct ParentWindow(usize);

impl ParentWindow {
	pub fn new(window: &dyn WxWidget) -> Self {
		Self(window.handle_ptr().expose_provenance())
	}
}

unsafe impl Send for ParentWindow {}
unsafe impl Sync for ParentWindow {}

impl WxWidget for ParentWindow {
	fn handle_ptr(&self) -> *mut ffi::wxd_Window_t {
		std::ptr::with_exposed_provenance_mut(self.0)
	}
}

/// Detects whether this running instance was installed by Inno Setup or unpacked as a portable zip.
///
/// Inno Setup always produces an `unins000.dat` file (the compiled log of installed files and registry entries)
/// alongside `unins000.exe` in the application directory. Portable packages never contain `unins*.dat` or `unins*.exe`.
pub fn detect_install_kind() -> InstallKind {
	let Ok(exe) = std::env::current_exe() else {
		return InstallKind::Portable;
	};
	let Some(parent) = exe.parent() else {
		return InstallKind::Portable;
	};

	let has_uninstaller_dat = parent.join("unins000.dat").exists()
		|| (1..=9).any(|i| parent.join(format!("unins{:03}.dat", i)).exists());
	let has_uninstaller_exe = parent.join("unins000.exe").exists()
		|| (1..=9).any(|i| parent.join(format!("unins{:03}.exe", i)).exists());

	if has_uninstaller_dat || has_uninstaller_exe {
		InstallKind::Installer
	} else {
		InstallKind::Portable
	}
}

/// Checks whether the directory is writable by the current process without elevated rights.
pub fn is_dir_writable(path: &Path) -> bool {
	let test_file = path.join(format!(".whatsnap_write_test_{}.tmp", std::process::id()));
	match std::fs::File::create(&test_file) {
		Ok(f) => {
			drop(f);
			let _ = std::fs::remove_file(test_file);
			true
		}
		Err(_) => false,
	}
}

fn announce(announcer: Option<StaticText>, text: &str) {
	if let Some(ann) = announcer {
		crate::platform::announce::announce(ann, text);
	}
}

fn ps_quote(s: &str) -> String {
	format!("'{}'", s.replace('\'', "''"))
}

fn describe_error(err: &UpdateError) -> (String, String) {
	match err {
		UpdateError::Network(_) => (
			"Could not connect to the update server.\n\nPlease check your internet connection and try again.".to_string(),
			"Network Error".to_string(),
		),
		UpdateError::Http(code) if *code == 403 || *code == 429 => (
			"GitHub API rate limit reached.\n\nPlease wait a few minutes before checking for updates again.".to_string(),
			"Update Check Rate Limited".to_string(),
		),
		UpdateError::Http(code) => (
			format!("The update server returned HTTP status {code}.\n\nPlease try again later."),
			"Server Error".to_string(),
		),
		UpdateError::Verification(_) => (
			"The downloaded update could not be verified with the official security signature.\n\nFor security reasons, the update has been cancelled.".to_string(),
			"Security Verification Failed".to_string(),
		),
		UpdateError::Cancelled => (
			"The update check was cancelled.".to_string(),
			"Cancelled".to_string(),
		),
		UpdateError::Io(msg) => (
			format!("An error occurred while preparing the update: {msg}"),
			"File System Error".to_string(),
		),
		_ => (
			format!("Could not complete update check: {err}"),
			"Update Error".to_string(),
		),
	}
}

pub fn format_release_notes(raw_notes: &str) -> String {
	let plain = ship_shape::ui::markdown_to_text(raw_notes);
	plain
		.lines()
		.map(|line| {
			let trimmed = line.trim_start();
			if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
				format!("• {rest}")
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

fn format_dialog_content(
	channel: UpdateChannel,
	latest_version: &str,
	raw_notes: &str,
) -> (String, String, String, String) {
	let current_version = env!("CARGO_PKG_VERSION");
	let current_commit = env!("WHATSNAP_COMMIT_HASH");
	let short_commit = if current_commit.len() >= 7 { &current_commit[..7] } else { current_commit };

	match channel {
		UpdateChannel::Stable => {
			let title = format!("whatSnap Update Available (v{})", latest_version.trim_start_matches('v'));
			let header = format!(
				"A new stable version of whatSnap is available!\n\nInstalled Version: v{}\nLatest Version: v{}\n\nWhat's new in this release:",
				current_version,
				latest_version.trim_start_matches('v')
			);
			let notes = format_release_notes(raw_notes);
			let notes = if notes.trim().is_empty() {
				"No release notes provided for this version.".to_string()
			} else {
				notes
			};
			(title, header, notes, "&Install Update".to_string())
		}
		UpdateChannel::Dev => {
			let title = format!("whatSnap Development Update ({})", latest_version);
			let header = format!(
				"A new development build of whatSnap is available!\n\nChannel: Development (Rolling Commits)\nInstalled Commit: {}\nLatest Build: {}\n\nRecent changes:",
				if short_commit.is_empty() { "unknown" } else { short_commit },
				latest_version
			);
			let notes = format_release_notes(raw_notes);
			let notes = if notes.trim().is_empty() {
				"Development build contains the latest experimental fixes and improvements.".to_string()
			} else {
				notes
			};
			(title, header, notes, "&Install Dev Update".to_string())
		}
	}
}

pub fn show_update_dialog(
	parent: &dyn WxWidget,
	channel: UpdateChannel,
	latest_version: &str,
	raw_notes: &str,
) -> bool {
	let (title, header, notes, btn_label) = format_dialog_content(channel, latest_version, raw_notes);

	let dialog = Dialog::builder(parent, &title)
		.with_size(540, 420)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);

	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let header_label = StaticText::builder(&panel).with_label(&header).build();
	header_label.set_font(&theme::Fonts::body());
	sizer.add(&header_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 14);

	let changelog_ctrl = TextCtrl::builder(&panel)
		.with_value(&notes)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::Rich2)
		.build();
	changelog_ctrl.set_font(&theme::Fonts::body());
	sizer.add(&changelog_ctrl, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 14);

	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let update_btn = Button::builder(&panel).with_id(ID_OK).with_label(&btn_label).build();
	theme::style_primary_button(&update_btn);
	let later_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label("&Later").build();

	btn_sizer.add_stretch_spacer(1);
	btn_sizer.add(&update_btn, 0, SizerFlag::Right, 10);
	btn_sizer.add(&later_btn, 0, SizerFlag::empty(), 0);
	sizer.add_sizer(&btn_sizer, 0, SizerFlag::Expand | SizerFlag::All, 14);

	panel.set_sizer(sizer, true);

	dialog.set_affirmative_id(ID_OK);
	dialog.set_escape_id(ID_CANCEL);
	dialog.centre();
	dialog.raise();

	// Focus the changelog edit box so users can immediately read the release notes
	// using arrow keys or screen reader commands, while Enter still triggers Install.
	changelog_ctrl.set_focus();

	dialog.show_modal() == ID_OK
}

fn show_up_to_date_dialog(parent: &dyn WxWidget, channel: UpdateChannel, announcer: Option<StaticText>) {
	let current_version = env!("CARGO_PKG_VERSION");
	let current_commit = env!("WHATSNAP_COMMIT_HASH");
	let short_commit = if current_commit.len() >= 7 { &current_commit[..7] } else { current_commit };

	let (title, message, speech) = match channel {
		UpdateChannel::Stable => (
			"whatSnap is Up to Date".to_string(),
			format!(
				"You are running the latest stable version of whatSnap (v{}).\n\nNo newer updates are available at this time.",
				current_version
			),
			format!("whatSnap is up to date. You are running version {}.", current_version),
		),
		UpdateChannel::Dev => (
			"whatSnap Dev Build is Up to Date".to_string(),
			format!(
				"You are running the latest development build.\n\nCurrent commit: {}\nChannel: Development\n\nNo newer commits are available.",
				if short_commit.is_empty() { "latest" } else { short_commit }
			),
			format!(
				"whatSnap development build is up to date. Commit {}.",
				if short_commit.is_empty() { "latest" } else { short_commit }
			),
		),
	};

	announce(announcer, &speech);

	MessageDialog::builder(parent, &message, &title)
		.with_style(MessageDialogStyle::IconInformation)
		.build()
		.show_modal();
}

fn show_error_dialog(parent: &dyn WxWidget, err: &UpdateError, announcer: Option<StaticText>) {
	let (message, title) = describe_error(err);
	announce(announcer, &format!("Update check failed: {}", title));
	MessageDialog::builder(parent, &message, &title)
		.with_style(MessageDialogStyle::IconError)
		.build()
		.show_modal();
}

fn install_and_restart(
	install_kind: InstallKind,
	downloaded_path: &Path,
	ui_tx: &UiCommandSender,
) -> Result<(), String> {
	let current_exe = std::env::current_exe()
		.map_err(|e| format!("Failed to get current executable path: {e}"))?;
	let pid = std::process::id();

	let script = match install_kind {
		InstallKind::Installer => {
			let installer_path = downloaded_path.display().to_string();
			let current_exe_str = current_exe.display().to_string();
			format!(
				"Start-Sleep -Seconds 1; Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 500; Start-Process -FilePath {inst} -ArgumentList '/SILENT','/CLOSEAPPLICATIONS','/SUPPRESSMSGBOXES','/NORESTARTAPPLICATIONS' -Wait; $env:SHIP_SHAPE_RELAUNCHED='1'; Start-Process -FilePath {exe}",
				inst = ps_quote(&installer_path),
				exe = ps_quote(&current_exe_str)
			)
		}
		InstallKind::Portable => {
			let zip_path = downloaded_path.display().to_string();
			let dest_dir = current_exe.parent().unwrap_or(&current_exe).display().to_string();
			let current_exe_str = current_exe.display().to_string();
			format!(
				"Start-Sleep -Seconds 1; Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 500; Expand-Archive -Path {zip} -DestinationPath {dest} -Force; Remove-Item -Path {zip} -Force; $env:SHIP_SHAPE_RELAUNCHED='1'; Start-Process -FilePath {exe}",
				zip = ps_quote(&zip_path),
				dest = ps_quote(&dest_dir),
				exe = ps_quote(&current_exe_str)
			)
		}
	};

	use std::os::windows::process::CommandExt;
	use windows::Win32::System::Threading::CREATE_NO_WINDOW;
	std::process::Command::new("powershell.exe")
		.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
		.creation_flags(CREATE_NO_WINDOW.0)
		.spawn()
		.map_err(|e| format!("Failed to launch updater script: {e}"))?;

	// Spawn safety watchdog in case backend or windows take too long to close
	std::thread::spawn(|| {
		std::thread::sleep(std::time::Duration::from_secs(15));
		std::process::exit(0);
	});

	// Request clean application shutdown
	let _ = ui_tx.send(UiCommand::ExitApp);
	Ok(())
}

fn handle_update_available(
	parent: &ParentWindow,
	ui_tx: &UiCommandSender,
	config: Arc<UpdaterConfig>,
	channel: UpdateChannel,
	result: UpdateAvailableResult,
	announcer: Option<StaticText>,
	guard: ActiveCheckGuard,
) {
	let is_dev = channel == UpdateChannel::Dev;
	if is_dev {
		announce(
			announcer,
			&format!("A new development build is available: {}.", result.latest_version),
		);
	} else {
		announce(
			announcer,
			&format!("A new stable version of whatSnap is available: v{}.", result.latest_version.trim_start_matches('v')),
		);
	}

	let accepted = show_update_dialog(parent, channel, &result.latest_version, &result.release_notes);
	if !accepted {
		drop(guard);
		return;
	}

	if config.install_kind == InstallKind::Portable {
		if let Ok(exe) = std::env::current_exe() {
			if let Some(dir) = exe.parent() {
				if !is_dir_writable(dir) {
					drop(guard);
					announce(announcer, "Update failed: Application folder is write-protected.");
					MessageDialog::builder(
						parent,
						"Cannot update whatSnap Portable: The installation folder is write-protected (administrator permissions required).\n\nPlease run whatSnap as administrator, or move the whatSnap folder to your user profile (e.g. Documents).",
						"Permission Denied",
					)
					.with_style(MessageDialogStyle::IconError)
					.build()
					.show_modal();
					return;
				}
			}
		}
	}

	start_download_and_install(parent, ui_tx, config, channel, result, announcer, guard);
}

fn start_download_and_install(
	parent: &ParentWindow,
	ui_tx: &UiCommandSender,
	config: Arc<UpdaterConfig>,
	channel: UpdateChannel,
	result: UpdateAvailableResult,
	announcer: Option<StaticText>,
	guard: ActiveCheckGuard,
) {
	let is_dev = channel == UpdateChannel::Dev;
	let title = format!("whatSnap {} Update", if is_dev { "Dev" } else { "Stable" });
	let dl_config = Arc::clone(&config);
	let download_url = result.download_url.clone();
	let signature_url = result.signature_url.clone();

	let work = move |progress: &Progress| {
		ship_shape::download_update_file(
			&dl_config,
			&download_url,
			&signature_url,
			progress.cancel_flag(),
			|done, total| {
				if total > 0 {
					let done_mb = done as f64 / (1024.0 * 1024.0);
					let total_mb = total as f64 / (1024.0 * 1024.0);
					let pct = ((done as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
					progress.set_message(&format!("Downloading update: {:.1} MB of {:.1} MB ({:.0}%)...", done_mb, total_mb, pct));
					progress.set(done, Some(total));
				} else {
					let done_mb = done as f64 / (1024.0 * 1024.0);
					progress.set_message(&format!("Downloading update: {:.1} MB...", done_mb));
					progress.set(done, None);
				}
			},
		)
	};

	let parent_win = *parent;
	let ui_tx_clone = ui_tx.clone();
	let install_kind = config.install_kind;

	run_with_progress(
		parent,
		&title,
		"Downloading update...",
		work,
		move |download_res, ended| {
			drop(guard);
			if ended != Ended::Completed {
				announce(announcer, "Update download cancelled.");
				return;
			}

			match download_res {
				Ok(downloaded_path) => {
					if let Err(e) = install_and_restart(install_kind, &downloaded_path, &ui_tx_clone) {
						MessageDialog::builder(&parent_win, &e, "Update Error")
							.with_style(MessageDialogStyle::IconError)
							.build()
							.show_modal();
					}
				}
				Err(err) => {
					let (msg, title) = describe_error(&err);
					announce(announcer, &format!("Download failed: {}", title));
					MessageDialog::builder(&parent_win, &msg, &title)
						.with_style(MessageDialogStyle::IconError)
						.build()
						.show_modal();
				}
			}
		},
	);
}

pub fn run(
	frame: &dyn WxWidget,
	ui_tx: &UiCommandSender,
	channel_str: &str,
	automatic: bool,
	announcer: Option<StaticText>,
) {
	if crate::platform::quit::is_quitting() {
		return;
	}

	if UPDATE_CHECK_ACTIVE.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
		if !automatic {
			if let Some(ann) = announcer {
				crate::platform::announce::announce(ann, "An update check is already in progress.");
			}
		}
		return;
	}

	let is_dev = channel_str == "dev";
	if !automatic {
		if let Some(ann) = announcer {
			let msg = if is_dev {
				"Checking for whatSnap development updates..."
			} else {
				"Checking for whatSnap updates..."
			};
			crate::platform::announce::announce(ann, msg);
		}
	}

	let parent_win = ParentWindow::new(frame);
	let channel_enum = if is_dev { UpdateChannel::Dev } else { UpdateChannel::Stable };
	let install_kind = detect_install_kind();

	let mut config = UpdaterConfig::new(
		GITHUB_REPO,
		"whatSnap",
		"whatSnap",
		MINISIGN_PUBLIC_KEY,
		env!("CARGO_PKG_VERSION"),
	)
	.with_commit(env!("WHATSNAP_COMMIT_HASH"))
	.with_install_kind(install_kind);
	config.installer_args = vec![
		"/SILENT".into(),
		"/CLOSEAPPLICATIONS".into(),
		"/SUPPRESSMSGBOXES".into(),
		"/NORESTARTAPPLICATIONS".into(),
	];
	let config = Arc::new(config);

	let tx = ui_tx.clone();
	std::thread::spawn(move || {
		let outcome = ship_shape::check_for_updates(&config, channel_enum);
		let guard = ActiveCheckGuard;
		wxdragon::call_after(Box::new(move || {
			match outcome {
				Ok(UpdateCheckOutcome::UpdateAvailable(result)) => {
					handle_update_available(
						&parent_win,
						&tx,
						config,
						channel_enum,
						result,
						announcer,
						guard,
					);
				}
				Ok(UpdateCheckOutcome::UpToDate(_)) => {
					drop(guard);
					if !automatic {
						show_up_to_date_dialog(&parent_win, channel_enum, announcer);
					}
				}
				Err(err) => {
					drop(guard);
					if !automatic {
						show_error_dialog(&parent_win, &err, announcer);
					}
				}
			}
		}));
		wxdragon::wake_up_idle();
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_ps_quote_escaping() {
		assert_eq!(ps_quote(r"C:\whatSnap\app.exe"), r"'C:\whatSnap\app.exe'");
		assert_eq!(ps_quote(r"C:\User's Path\app.exe"), r"'C:\User''s Path\app.exe'");
	}

	#[test]
	fn test_format_release_notes_bullets() {
		let input = "- Feature 1\n* Feature 2\nRegular line";
		let formatted = format_release_notes(input);
		assert!(formatted.contains("• Feature 1"));
		assert!(formatted.contains("• Feature 2"));
		assert!(formatted.contains("Regular line"));
	}

	#[test]
	fn test_format_dialog_content_stable() {
		let (title, header, notes, btn) =
			format_dialog_content(UpdateChannel::Stable, "v0.2.0", "### Improvements\n- Faster startup");
		assert!(title.contains("v0.2.0"));
		assert!(header.contains("A new stable version"));
		assert!(header.contains("v0.2.0"));
		assert_eq!(btn, "&Install Update");
		assert!(notes.contains("Faster startup"));
	}

	#[test]
	fn test_format_dialog_content_dev() {
		let (title, header, notes, btn) =
			format_dialog_content(UpdateChannel::Dev, "dev-a1b2c3d", "- Fix call sync\n- Fix notifications");
		assert!(title.contains("dev-a1b2c3d"));
		assert!(header.contains("Development (Rolling Commits)"));
		assert!(header.contains("dev-a1b2c3d"));
		assert_eq!(btn, "&Install Dev Update");
		assert!(notes.contains("Fix call sync"));
	}

	#[test]
	fn test_describe_error_rate_limit() {
		let (msg, title) = describe_error(&UpdateError::Http(429));
		assert_eq!(title, "Update Check Rate Limited");
		assert!(msg.contains("rate limit"));

		let (msg, title) = describe_error(&UpdateError::Http(403));
		assert_eq!(title, "Update Check Rate Limited");
		assert!(msg.contains("rate limit"));
	}

	#[test]
	fn test_describe_error_network() {
		let (msg, title) = describe_error(&UpdateError::Network("connection timeout".into()));
		assert_eq!(title, "Network Error");
		assert!(msg.contains("check your internet connection"));
	}
}
