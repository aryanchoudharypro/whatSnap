use wxdragon::prelude::*;

use crate::ui::theme;

#[derive(Clone)]
pub struct CallWindowParts {
	pub frame: Frame,
	pub avatar_bitmap: StaticBitmap,
	pub status_label: StaticText,
	/// Who the call is with: name, phone number, duration, mic/camera status,
	/// and for a call we placed "Calling..."/"Ringing..."/"Connecting...". Read-only,
	/// so a screen reader can move through it with Tab and arrow keys.
	pub caller_info: TextCtrl,
	/// Live region for anything announced while the call window has focus.
	/// The call frame is its own top-level window, and a screen reader only
	/// speaks live-region changes from the FOREGROUND window.
	pub announcer: StaticText,
	pub accept_btn: Button,
	pub reject_btn: Button,
	pub dismiss_btn: Button,
	pub mute_btn: Button,
	pub camera_btn: Button,
	pub screen_btn: Button,
	pub device_settings_btn: Button,
	/// Device selectors shown inline when device settings is expanded. Each
	/// group is a visible label above its combo; show/hide the group, not the combo.
	pub input_device_group: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_group: Panel,
	pub output_device_combo: ComboBox,
	pub camera_device_group: Panel,
	pub camera_device_combo: ComboBox,
	pub add_participant_btn: Button,
	pub raise_hand_btn: Button,
	pub react_btn: Button,
	pub leave_btn: Button,
	pub callback_btn: Button,
	pub close_btn: Button,
	pub people_panel: Panel,
	pub people_list: ListBox,
	pub mute_person_btn: Button,
	pub remove_person_btn: Button,
	pub let_in_btn: Button,
	pub keep_out_btn: Button,
	pub approval_check: CheckBox,
}

/// One device selector for the call window: a visible label above a read-only
/// combo, in its own panel so both show and hide together. Starts hidden.
fn device_selector(parent: &Panel, sizer: &BoxSizer, label: &str) -> (Panel, ComboBox) {
	let group = Panel::builder(parent).build();
	if theme::use_custom_colours() {
		group.set_background_color(CALL_BG);
	}
	let group_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let text = StaticText::builder(&group).with_label(label).build();
	text.set_font(&theme::Fonts::label());
	if theme::use_custom_colours() {
		text.set_foreground_color(CALL_MUTED_TEXT);
	}
	group_sizer.add(&text, 0, SizerFlag::Left | SizerFlag::Top, 2);
	let combo = ComboBox::builder(&group).with_style(ComboBoxStyle::ReadOnly).build();
	group_sizer.add(&combo, 0, SizerFlag::Expand, 0);
	group.set_sizer(group_sizer, true);
	sizer.add(&group, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	group.show(false);
	(group, combo)
}

/// WhatsApp's call screen colours: the call screen is dark in either theme.
pub const CALL_BG: Colour = Colour::rgb(0x10, 0x1A, 0x20);
pub const CALL_TEXT: Colour = Colour::rgb(0xE9, 0xED, 0xEE);
pub const CALL_MUTED_TEXT: Colour = Colour::rgb(0x86, 0x96, 0xA0);

pub fn empty_avatar_bitmap(side: u32) -> wxdragon::bitmap::Bitmap {
	let side = side.max(1);
	let (cx, cy, r) = (side as f32 / 2.0, side as f32 / 2.0, side as f32 / 2.0);
	let mut rgba = vec![0u8; (side * side * 4) as usize];
	for y in 0..side {
		for x in 0..side {
			let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
			let dist = (dx * dx + dy * dy).sqrt();
			let cover = (r - dist + 0.5).clamp(0.0, 1.0);
			if cover > 0.0 {
				// Anti-aliased WhatsApp contact silhouette: head circle + shoulder curve
				let h_dx = x as f32 + 0.5 - cx;
				let h_dy = y as f32 + 0.5 - (cy - r * 0.20);
				let h_dist = (h_dx * h_dx + h_dy * h_dy).sqrt();
				let head_cover = (r * 0.32 - h_dist + 0.5).clamp(0.0, 1.0);

				let b_dx = x as f32 + 0.5 - cx;
				let b_dy = y as f32 + 0.5 - (cy + r * 0.96);
				let b_dist = (b_dx * b_dx + b_dy * b_dy).sqrt();
				let body_cover = (r * 0.72 - b_dist + 0.5).clamp(0.0, 1.0);

				let sil_factor = (head_cover + body_cover).clamp(0.0, 1.0);

				// Dark circle base: rgb(0x23, 0x31, 0x38), Silhouette: rgb(0x6A, 0x7B, 0x85)
				let red = 0x23 as f32 * (1.0 - sil_factor) + 0x6A as f32 * sil_factor;
				let green = 0x31 as f32 * (1.0 - sil_factor) + 0x7B as f32 * sil_factor;
				let blue = 0x38 as f32 * (1.0 - sil_factor) + 0x85 as f32 * sil_factor;

				let idx = ((y * side + x) * 4) as usize;
				rgba[idx] = red as u8;
				rgba[idx + 1] = green as u8;
				rgba[idx + 2] = blue as u8;
				rgba[idx + 3] = (255.0 * cover) as u8;
			}
		}
	}
	wxdragon::bitmap::Bitmap::from_rgba(&rgba, side, side)
		.unwrap_or_else(|| wxdragon::bitmap::Bitmap::new(1, 1).unwrap())
}

pub fn build_call_window() -> CallWindowParts {
	let frame = Frame::builder().with_title(crate::i18n::tr("Call")).with_size(Size::new(600, 580)).build();
	let panel = Panel::builder(&frame).build();
	if theme::use_custom_colours() {
		panel.set_background_color(CALL_BG);
		panel.set_foreground_color(CALL_TEXT);
	}
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let avatar_bm = empty_avatar_bitmap(112);
	let avatar_bitmap = StaticBitmap::builder(&panel).with_bitmap(Some(avatar_bm)).build();
	sizer.add(&avatar_bitmap, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top | SizerFlag::Bottom, 14);

	let status_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Incoming call...")).build();
	status_label.set_font(&theme::Fonts::heading());
	if theme::use_custom_colours() {
		status_label.set_foreground_color(CALL_MUTED_TEXT);
	}
	sizer.add(
		&status_label,
		0,
		SizerFlag::AlignCenterHorizontal | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		8,
	);

	let caller_info = TextCtrl::builder(&panel)
		.with_style(
			TextCtrlStyle::MultiLine
				| TextCtrlStyle::ReadOnly
				| TextCtrlStyle::Centre
				| TextCtrlStyle::from_bits_retain(0x0020_0000),
		)
		.with_size(Size::new(-1, 80))
		.build();
	caller_info.set_font(&theme::Fonts::body_strong());
	if theme::use_custom_colours() {
		caller_info.set_background_color(CALL_BG);
		caller_info.set_foreground_color(CALL_TEXT);
	}
	caller_info.show(true);
	// Screen readers name the box from the text just above it, so this label
	// is what the box is announced as ("Call info"), not the status heading.
	let caller_info_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Call info")).build();
	caller_info_label.set_font(&theme::Fonts::label());
	if theme::use_custom_colours() {
		caller_info_label.set_foreground_color(CALL_MUTED_TEXT);
	}
	sizer.add(&caller_info_label, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Bottom, 4);
	sizer.add(&caller_info, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Expand | SizerFlag::All, 0);

	let btn_panel = Panel::builder(&panel).build();
	if theme::use_custom_colours() {
		btn_panel.set_background_color(CALL_BG);
	}
	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let accept_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Accept call")).build();
	theme::style_primary_button(&accept_btn);
	btn_sizer.add(&accept_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);

	let reject_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Decline call")).build();
	if theme::use_custom_colours() {
		reject_btn.set_background_color(Colour::rgb(0xEA, 0x00, 0x38));
		reject_btn.set_foreground_color(Colour::rgb(0xFF, 0xFF, 0xFF));
		reject_btn.set_font(&theme::Fonts::body_strong());
	}
	btn_sizer.add(&reject_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);

	let dismiss_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Dismiss")).build();
	crate::ui::owner_draw::install_tile(&dismiss_btn, CALL_BG);
	btn_sizer.add(&dismiss_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	dismiss_btn.show(false);

	let mute_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Mute microphone")).build();
	crate::ui::owner_draw::install_tile(&mute_btn, CALL_BG);
	btn_sizer.add(&mute_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	mute_btn.show(false);

	let camera_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Turn camera on")).build();
	crate::ui::owner_draw::install_tile(&camera_btn, CALL_BG);
	btn_sizer.add(&camera_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	camera_btn.show(false);

	let screen_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Share screen")).build();
	crate::ui::owner_draw::install_tile(&screen_btn, CALL_BG);
	btn_sizer.add(&screen_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	screen_btn.show(false);

	let device_settings_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Device settings")).build();
	crate::ui::owner_draw::install_tile(&device_settings_btn, CALL_BG);
	btn_sizer.add(&device_settings_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	device_settings_btn.show(false);

	// Device selectors shown inline when device settings is expanded. Screen
	// readers name a combo from the static text just before it, so each one
	// sits under a visible label, the way the old devices panel did.
	let (input_device_group, input_device_combo) =
		device_selector(&btn_panel, &btn_sizer, crate::i18n::tr("Microphone"));
	let (output_device_group, output_device_combo) =
		device_selector(&btn_panel, &btn_sizer, crate::i18n::tr("Speaker"));
	let (camera_device_group, camera_device_combo) = device_selector(&btn_panel, &btn_sizer, crate::i18n::tr("Camera"));

	let add_participant_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Add participant")).build();
	crate::ui::owner_draw::install_tile(&add_participant_btn, CALL_BG);
	btn_sizer.add(&add_participant_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	add_participant_btn.show(false);

	let raise_hand_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Raise hand")).build();
	crate::ui::owner_draw::install_tile(&raise_hand_btn, CALL_BG);
	btn_sizer.add(&raise_hand_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	raise_hand_btn.show(false);

	let react_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("React...")).build();
	crate::ui::owner_draw::install_tile(&react_btn, CALL_BG);
	btn_sizer.add(&react_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	react_btn.show(false);

	let leave_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("End call")).build();
	if theme::use_custom_colours() {
		leave_btn.set_background_color(Colour::rgb(0xEA, 0x00, 0x38));
		leave_btn.set_foreground_color(Colour::rgb(0xFF, 0xFF, 0xFF));
		leave_btn.set_font(&theme::Fonts::body_strong());
	}
	btn_sizer.add(&leave_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	leave_btn.show(false);

	let callback_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Call back")).build();
	theme::style_primary_button(&callback_btn);
	crate::ui::owner_draw::install_tile(&callback_btn, CALL_BG);
	btn_sizer.add(&callback_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	callback_btn.show(false);

	let close_btn = Button::builder(&btn_panel).with_label(crate::i18n::tr("Close")).build();
	crate::ui::owner_draw::install_tile(&close_btn, CALL_BG);
	btn_sizer.add(&close_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	close_btn.show(false);

	btn_panel.set_sizer(btn_sizer, true);
	sizer.add(&btn_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let people_panel = Panel::builder(&panel).build();
	theme::style_content(&people_panel);
	let people_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let people_label = StaticText::builder(&people_panel).with_label(crate::i18n::tr("People")).build();
	people_sizer.add(&people_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let people_list = ListBox::builder(&people_panel).build();
	people_sizer.add(&people_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);
	let person_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let mute_person_btn = Button::builder(&people_panel).with_label(crate::i18n::tr("Mute")).build();
	person_sizer.add(&mute_person_btn, 0, SizerFlag::Right, 5);
	let remove_person_btn = Button::builder(&people_panel).with_label(crate::i18n::tr("Remove")).build();
	person_sizer.add(&remove_person_btn, 0, SizerFlag::empty(), 0);
	people_sizer.add_sizer(&person_sizer, 0, SizerFlag::All, 4);
	mute_person_btn.show(false);
	remove_person_btn.show(false);
	let admit_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let let_in_btn = Button::builder(&people_panel).with_label(crate::i18n::tr("Let in")).build();
	admit_sizer.add(&let_in_btn, 0, SizerFlag::Right, 5);
	let keep_out_btn = Button::builder(&people_panel).with_label(crate::i18n::tr("Don't let in")).build();
	admit_sizer.add(&keep_out_btn, 0, SizerFlag::empty(), 0);
	people_sizer.add_sizer(&admit_sizer, 0, SizerFlag::All, 4);
	let approval_check =
		CheckBox::builder(&people_panel).with_label(crate::i18n::tr("Approve new participants")).build();
	people_sizer.add(&approval_check, 0, SizerFlag::All, 4);
	let_in_btn.show(false);
	keep_out_btn.show(false);
	approval_check.show(false);
	people_panel.set_sizer(people_sizer, true);
	sizer.add(&people_panel, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
	people_panel.show(false);

	panel.set_sizer(sizer, true);

	CallWindowParts {
		frame,
		avatar_bitmap,
		status_label,
		caller_info,
		announcer,
		accept_btn,
		reject_btn,
		dismiss_btn,
		mute_btn,
		camera_btn,
		screen_btn,
		device_settings_btn,
		input_device_group,
		input_device_combo,
		output_device_group,
		output_device_combo,
		camera_device_group,
		camera_device_combo,
		add_participant_btn,
		raise_hand_btn,
		react_btn,
		leave_btn,
		callback_btn,
		close_btn,
		people_panel,
		people_list,
		mute_person_btn,
		remove_person_btn,
		let_in_btn,
		keep_out_btn,
		approval_check,
	}
}
