use wxdragon::prelude::*;

#[derive(Clone)]
pub struct CallWindowParts {
	pub frame: Frame,
	pub status_label: StaticText,
	/// Live region for anything announced while the call window has focus.
	/// The call frame is its own top-level window, and a screen reader only
	/// speaks live-region changes from the FOREGROUND window - announcing a
	/// call event through the main window's announcer reaches nobody while
	/// this frame is up.
	pub announcer: StaticText,
	pub accept_btn: Button,
	pub reject_btn: Button,
	pub mute_btn: Button,
	pub add_participant_btn: Button,
	pub leave_btn: Button,
	pub devices_panel: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_combo: ComboBox,
}

pub fn build_call_window() -> CallWindowParts {
	let frame = Frame::builder().with_title("Call").with_size(Size::new(400, 300)).build();
	let panel = Panel::builder(&frame).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let status_label = StaticText::builder(&panel).with_label("Incoming call...").build();
	sizer.add(&status_label, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let announcer = StaticText::builder(&panel).with_label("").build();
	live_region::set_live_region(&announcer);
	sizer.add(&announcer, 0, SizerFlag::Expand | SizerFlag::All, 0);

	let btn_panel = Panel::builder(&panel).build();
	let btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let accept_btn = Button::builder(&btn_panel).with_label("Accept call").build();
	btn_sizer.add(&accept_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);

	let reject_btn = Button::builder(&btn_panel).with_label("Reject call").build();
	btn_sizer.add(&reject_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);

	let mute_btn = Button::builder(&btn_panel).with_label("Mute microphone").build();
	btn_sizer.add(&mute_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	mute_btn.show(false);

	let add_participant_btn = Button::builder(&btn_panel).with_label("Add participant").build();
	btn_sizer.add(&add_participant_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	add_participant_btn.show(false);

	let leave_btn = Button::builder(&btn_panel).with_label("Leave call").build();
	btn_sizer.add(&leave_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	leave_btn.show(false);

	btn_panel.set_sizer(btn_sizer, true);
	sizer.add(&btn_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);

	// Lets the mic/speaker be switched without ending the call - hidden until
	// the call connects, same as mute_btn/leave_btn above.
	let devices_panel = Panel::builder(&panel).build();
	let devices_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let input_device_label = StaticText::builder(&devices_panel).with_label("Microphone").build();
	devices_sizer.add(&input_device_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let input_device_combo = ComboBox::builder(&devices_panel).with_style(ComboBoxStyle::ReadOnly).build();
	devices_sizer.add(&input_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);

	let output_device_label = StaticText::builder(&devices_panel).with_label("Speaker").build();
	devices_sizer.add(&output_device_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let output_device_combo = ComboBox::builder(&devices_panel).with_style(ComboBoxStyle::ReadOnly).build();
	devices_sizer.add(&output_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);

	devices_panel.set_sizer(devices_sizer, true);
	sizer.add(&devices_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);
	devices_panel.show(false);

	panel.set_sizer(sizer, true);

	CallWindowParts {
		frame,
		status_label,
		announcer,
		accept_btn,
		reject_btn,
		mute_btn,
		add_participant_btn,
		leave_btn,
		devices_panel,
		input_device_combo,
		output_device_combo,
	}
}
