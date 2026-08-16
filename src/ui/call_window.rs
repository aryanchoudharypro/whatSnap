use wxdragon::prelude::*;

#[derive(Clone)]
pub struct CallWindowParts {
	pub frame: Frame,
	pub status_label: StaticText,
	pub accept_btn: Button,
	pub reject_btn: Button,
	pub mute_btn: Button,
	pub add_participant_btn: Button,
	pub leave_btn: Button,
}

pub fn build_call_window() -> CallWindowParts {
	let frame = Frame::builder().with_title("Call").with_size(Size::new(400, 300)).build();
	let panel = Panel::builder(&frame).build();
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	let status_label = StaticText::builder(&panel).with_label("Incoming call...").build();
	sizer.add(&status_label, 0, SizerFlag::Expand | SizerFlag::All, 10);

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
	panel.set_sizer(sizer, true);

	CallWindowParts {
		frame,
		status_label,
		accept_btn,
		reject_btn,
		mute_btn,
		add_participant_btn,
		leave_btn,
	}
}
