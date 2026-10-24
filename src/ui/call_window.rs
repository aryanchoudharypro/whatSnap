use wxdragon::prelude::*;

#[derive(Clone)]
pub struct CallWindowParts {
	pub frame: Frame,
	pub status_label: StaticText,
	/// "Call from <name>, <phone>" for an incoming call. Read-only, so a screen
	/// reader can move through it to read the caller's details.
	pub caller_info: TextCtrl,
	/// Live region for anything announced while the call window has focus.
	/// The call frame is its own top-level window, and a screen reader only
	/// speaks live-region changes from the FOREGROUND window - announcing a
	/// call event through the main window's announcer reaches nobody while
	/// this frame is up.
	pub announcer: StaticText,
	pub accept_btn: Button,
	pub reject_btn: Button,
	pub mute_btn: Button,
	pub camera_btn: Button,
	pub screen_btn: Button,
	pub add_participant_btn: Button,
	pub raise_hand_btn: Button,
	pub react_btn: Button,
	pub leave_btn: Button,
	/// Who's in a group call, who's being rung and who's waiting to be let
	/// in through a call link. Hidden on 1:1 calls.
	pub people_panel: Panel,
	pub people_list: ListBox,
	pub let_in_btn: Button,
	pub keep_out_btn: Button,
	/// A call link's "approve new participants", for the link's admin.
	pub approval_check: CheckBox,
	pub devices_panel: Panel,
	pub input_device_combo: ComboBox,
	pub output_device_combo: ComboBox,
	pub camera_device_combo: ComboBox,
}

pub fn build_call_window() -> CallWindowParts {
	let frame = Frame::builder().with_title("Call").with_size(Size::new(400, 420)).build();
	let panel = Panel::builder(&frame).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let status_label = StaticText::builder(&panel).with_label("Incoming call...").build();
	sizer.add(&status_label, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let caller_info = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ReadOnly).build();
	caller_info.show(false);
	sizer.add(&caller_info, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

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

	// Ctrl+Shift+V also toggles this - see keybindings.
	let camera_btn = Button::builder(&btn_panel).with_label("Turn camera on").build();
	btn_sizer.add(&camera_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	camera_btn.show(false);

	// Ctrl+Shift+S also toggles this.
	let screen_btn = Button::builder(&btn_panel).with_label("Share screen").build();
	btn_sizer.add(&screen_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	screen_btn.show(false);

	let add_participant_btn = Button::builder(&btn_panel).with_label("Add participant").build();
	btn_sizer.add(&add_participant_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	add_participant_btn.show(false);

	// Shown once the call connects, like add_participant_btn.
	let raise_hand_btn = Button::builder(&btn_panel).with_label("Raise hand").build();
	btn_sizer.add(&raise_hand_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	raise_hand_btn.show(false);
	let react_btn = Button::builder(&btn_panel).with_label("React...").build();
	btn_sizer.add(&react_btn, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	react_btn.show(false);

	let leave_btn = Button::builder(&btn_panel).with_label("Leave call").build();
	btn_sizer.add(&leave_btn, 1, SizerFlag::Expand | SizerFlag::Left, 5);
	leave_btn.show(false);

	btn_panel.set_sizer(btn_sizer, true);
	sizer.add(&btn_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);

	let people_panel = Panel::builder(&panel).build();
	let people_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let people_label = StaticText::builder(&people_panel).with_label("People").build();
	people_sizer.add(&people_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let people_list = ListBox::builder(&people_panel).build();
	people_sizer.add(&people_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);
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

	let camera_device_label = StaticText::builder(&devices_panel).with_label("Camera").build();
	devices_sizer.add(&camera_device_label, 0, SizerFlag::Left | SizerFlag::Top, 4);
	let camera_device_combo = ComboBox::builder(&devices_panel).with_style(ComboBoxStyle::ReadOnly).build();
	devices_sizer.add(&camera_device_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 4);

	devices_panel.set_sizer(devices_sizer, true);
	sizer.add(&devices_panel, 0, SizerFlag::Expand | SizerFlag::All, 10);
	devices_panel.show(false);

	panel.set_sizer(sizer, true);

	CallWindowParts {
		frame,
		status_label,
		caller_info,
		announcer,
		accept_btn,
		reject_btn,
		mute_btn,
		camera_btn,
		screen_btn,
		add_participant_btn,
		raise_hand_btn,
		react_btn,
		leave_btn,
		people_panel,
		people_list,
		let_in_btn,
		keep_out_btn,
		approval_check,
		devices_panel,
		input_device_combo,
		output_device_combo,
		camera_device_combo,
	}
}
