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
	pub devices_panel: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_combo: ComboBox,
	pub camera_device_label: StaticText,
	pub camera_device_combo: ComboBox,
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
				let idx = ((y * side + x) * 4) as usize;
				rgba[idx] = 0x23;
				rgba[idx + 1] = 0x31;
				rgba[idx + 2] = 0x38;
				rgba[idx + 3] = (255.0 * cover) as u8;
			}
		}
	}
	wxdragon::bitmap::Bitmap::from_rgba(&rgba, side, side).unwrap_or_else(|| wxdragon::bitmap::Bitmap::new(1, 1).unwrap())
}

pub fn build_call_window() -> CallWindowParts {
	let frame = Frame::builder().with_title("Call").with_size(Size::new(600, 580)).build();
	let panel = Panel::builder(&frame).build();
	if theme::use_custom_colours() {
		panel.set_background_color(CALL_BG);
		panel.set_foreground_color(CALL_TEXT);
	}
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let avatar_bm = empty_avatar_bitmap(96);
	let avatar_bitmap = StaticBitmap::builder(&panel).with_bitmap(Some(avatar_bm)).build();
	sizer.add(&avatar_bitmap, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top | SizerFlag::Bottom, 12);

	let status_label = StaticText::builder(&panel).with_label("Incoming call...").build();
	status_label.set_font(&theme::Fonts::heading());
	if theme::use_custom_colours() {
		status_label.set_foreground_color(CALL_MUTED_TEXT);
	}
	sizer.add(&status_label, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 8);

	let caller_info = TextCtrl::builder(&panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly)
		.with_size(Size::new(-1, 85))
		.build();
	caller_info.set_font(&theme::Fonts::body());
	if theme::use_custom_colours() {
		caller_info.set_background_color(CALL_BG);
		caller_info.set_foreground_color(CALL_TEXT);
	}
	caller_info.show(true);
	sizer.add(&caller_info, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Expand | SizerFlag::All, 0);

	let btn_panel = Panel::builder(&panel).build();
	if theme::use_custom_colours() {
		btn_panel.set_background_color(CALL_BG);
	}
	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let accept_btn = Button::builder(&btn_panel).with_label("Accept call").build();
	crate::ui::owner_draw::install_tile(&accept_btn, CALL_BG);
	btn_sizer.add(&accept_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);

	let reject_btn = Button::builder(&btn_panel).with_label("Decline call").build();
	crate::ui::owner_draw::install_tile(&reject_btn, CALL_BG);
	btn_sizer.add(&reject_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);

	let dismiss_btn = Button::builder(&btn_panel).with_label("Dismiss").build();
	crate::ui::owner_draw::install_tile(&dismiss_btn, CALL_BG);
	btn_sizer.add(&dismiss_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	dismiss_btn.show(false);

	let mute_btn = Button::builder(&btn_panel).with_label("Mute microphone").build();
	crate::ui::owner_draw::install_tile(&mute_btn, CALL_BG);
	btn_sizer.add(&mute_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	mute_btn.show(false);

	let camera_btn = Button::builder(&btn_panel).with_label("Turn camera on").build();
	crate::ui::owner_draw::install_tile(&camera_btn, CALL_BG);
	btn_sizer.add(&camera_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	camera_btn.show(false);

	let screen_btn = Button::builder(&btn_panel).with_label("Share screen").build();
	crate::ui::owner_draw::install_tile(&screen_btn, CALL_BG);
	btn_sizer.add(&screen_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	screen_btn.show(false);

	let add_participant_btn = Button::builder(&btn_panel).with_label("Add participant").build();
	crate::ui::owner_draw::install_tile(&add_participant_btn, CALL_BG);
	btn_sizer.add(&add_participant_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	add_participant_btn.show(false);

	let raise_hand_btn = Button::builder(&btn_panel).with_label("Raise hand").build();
	crate::ui::owner_draw::install_tile(&raise_hand_btn, CALL_BG);
	btn_sizer.add(&raise_hand_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	raise_hand_btn.show(false);

	let react_btn = Button::builder(&btn_panel).with_label("React...").build();
	crate::ui::owner_draw::install_tile(&react_btn, CALL_BG);
	btn_sizer.add(&react_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	react_btn.show(false);

	let leave_btn = Button::builder(&btn_panel).with_label("End call").build();
	crate::ui::owner_draw::install_tile(&leave_btn, CALL_BG);
	btn_sizer.add(&leave_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	leave_btn.show(false);

	let callback_btn = Button::builder(&btn_panel).with_label("Call back").build();
	crate::ui::owner_draw::install_tile(&callback_btn, CALL_BG);
	btn_sizer.add(&callback_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	callback_btn.show(false);

	let close_btn = Button::builder(&btn_panel).with_label("Close").build();
	crate::ui::owner_draw::install_tile(&close_btn, CALL_BG);
	btn_sizer.add(&close_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	close_btn.show(false);

	btn_panel.set_sizer(btn_sizer, true);
	sizer.add(&btn_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let people_panel = Panel::builder(&panel).build();
	theme::style_content(&people_panel);
	let people_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let people_label = StaticText::builder(&people_panel).with_label("People").build();
	people_sizer.add(&people_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let people_list = ListBox::builder(&people_panel).build();
	people_sizer.add(&people_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);
	let person_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let mute_person_btn = Button::builder(&people_panel).with_label("Mute").build();
	person_sizer.add(&mute_person_btn, 0, SizerFlag::Right, 5);
	let remove_person_btn = Button::builder(&people_panel).with_label("Remove").build();
	person_sizer.add(&remove_person_btn, 0, SizerFlag::empty(), 0);
	people_sizer.add_sizer(&person_sizer, 0, SizerFlag::All, 4);
	mute_person_btn.show(false);
	remove_person_btn.show(false);
	let admit_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let let_in_btn = Button::builder(&people_panel).with_label("Let in").build();
	admit_sizer.add(&let_in_btn, 0, SizerFlag::Right, 5);
	let keep_out_btn = Button::builder(&people_panel).with_label("Don't let in").build();
	admit_sizer.add(&keep_out_btn, 0, SizerFlag::empty(), 0);
	people_sizer.add_sizer(&admit_sizer, 0, SizerFlag::All, 4);
	let approval_check = CheckBox::builder(&people_panel).with_label("Approve new participants").build();
	people_sizer.add(&approval_check, 0, SizerFlag::All, 4);
	let_in_btn.show(false);
	keep_out_btn.show(false);
	approval_check.show(false);
	people_panel.set_sizer(people_sizer, true);
	sizer.add(&people_panel, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
	people_panel.show(false);

	let devices_panel = Panel::builder(&panel).build();
	if theme::use_custom_colours() {
		devices_panel.set_background_color(CALL_BG);
	}
	let devices_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let input_device_label = StaticText::builder(&devices_panel).with_label("Microphone").build();
	input_device_label.set_font(&theme::Fonts::label());
	if theme::use_custom_colours() {
		input_device_label.set_foreground_color(CALL_MUTED_TEXT);
	}
	devices_sizer.add(&input_device_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let input_device_combo = ComboBox::builder(&devices_panel).with_style(ComboBoxStyle::ReadOnly).build();
	devices_sizer.add(&input_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);

	let output_device_label = StaticText::builder(&devices_panel).with_label("Speaker").build();
	output_device_label.set_font(&theme::Fonts::label());
	if theme::use_custom_colours() {
		output_device_label.set_foreground_color(CALL_MUTED_TEXT);
	}
	devices_sizer.add(&output_device_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let output_device_combo = ComboBox::builder(&devices_panel).with_style(ComboBoxStyle::ReadOnly).build();
	devices_sizer.add(&output_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);

	let camera_device_label = StaticText::builder(&devices_panel).with_label("Camera").build();
	camera_device_label.set_font(&theme::Fonts::label());
	if theme::use_custom_colours() {
		camera_device_label.set_foreground_color(CALL_MUTED_TEXT);
	}
	devices_sizer.add(&camera_device_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let camera_device_combo = ComboBox::builder(&devices_panel).with_style(ComboBoxStyle::ReadOnly).build();
	devices_sizer.add(&camera_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);

	devices_panel.set_sizer(devices_sizer, true);
	sizer.add(&devices_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);
	devices_panel.show(false);

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
		devices_panel,
		input_device_combo,
		output_device_combo,
		camera_device_label,
		camera_device_combo,
	}
}
