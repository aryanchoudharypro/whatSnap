use wxdragon::prelude::*;

use crate::ui::theme;

/// Every constructed widget is kept here for consistency even if not all of
/// them are read back after construction yet (e.g. static labels).
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ConversationsPanelParts {
	pub panel: Panel,

	pub search_label: StaticText,
	pub search_field: TextCtrl,
	pub new_conv_btn: Button,

	pub conversations_label: StaticText,
	pub conversations_list: ListBox,

	pub conversation_panel: Panel,
	pub conv_data_btn: Button,
	pub add_attachment_btn: Button,
	pub search_in_conv_btn: Button,
	pub place_call_btn: Button,

	pub search_panel: Panel,
	pub search_close_btn: Button,
	pub search_conv_label: StaticText,
	pub search_conv_field: TextCtrl,

	pub messages_label: StaticText,
	pub messages_list: ListBox,

	pub play_voice_btn: Button,
	pub messages_announcer: StaticText,

	pub read_more_btn: Button,

	pub message_label: StaticText,
	pub message_field: TextCtrl,
	pub cancel_edit_btn: Button,
	pub remove_quote_btn: Button,
	pub send_message_btn: Button,
	pub record_voice_btn: Button,

	pub discard_voice_btn: Button,
	pub pause_resume_btn: Button,
	pub send_voice_btn: Button,

	pub attachment_panel: Panel,
	pub caption_label: StaticText,
	pub caption_field: TextCtrl,
	pub send_attachment_btn: Button,
	pub add_more_btn: Button,
	pub attachment_info_label: StaticText,

	pub audio_speed_btn: Button,
	pub audio_progress_label: Button,
	pub audio_slider: Slider,

	pub action_panel: Panel,
	pub action_open_btn: Button,
	pub action_save_as_btn: Button,
	pub action_download_btn: Button,
}

pub fn build_conversations_panel(parent: &impl WxWidget) -> ConversationsPanelParts {
	let panel = Panel::builder(parent).build();
	theme::style_content(&panel);
	let outer_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label("Search conversations").build();
	theme::style_muted_label(&search_label);
	outer_sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_field = TextCtrl::builder(&panel).build();
	search_field.set_font(&theme::Fonts::body());
	outer_sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let new_conv_btn = Button::builder(&panel).with_label("New Conversation").build();
	theme::style_primary_button(&new_conv_btn);
	outer_sizer.add(
		&new_conv_btn,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top | SizerFlag::Bottom,
		10,
	);

	let list_divider = StaticLine::builder(&panel).build();
	theme::style_divider(&list_divider);
	outer_sizer.add(&list_divider, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

	let conversations_label = StaticText::builder(&panel).with_label("Conversations").build();
	theme::style_heading(&conversations_label);
	outer_sizer.add(&conversations_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let conversations_list = ListBox::builder(&panel).build();
	conversations_list.set_font(&theme::Fonts::body());
	outer_sizer.add(&conversations_list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let conversation_panel = Panel::builder(&panel).build();
	theme::style_chat_area(&conversation_panel);
	let conv_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let top_btns_panel = Panel::builder(&conversation_panel).build();
	let top_btns_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let conv_data_btn = Button::builder(&top_btns_panel).with_label("Conversation data").build();
	conv_data_btn.set_font(&theme::Fonts::body());
	top_btns_sizer.add(&conv_data_btn, 0, SizerFlag::Right, 5);

	let search_in_conv_btn = Button::builder(&top_btns_panel).with_label("Search in conversation").build();
	search_in_conv_btn.set_font(&theme::Fonts::body());
	top_btns_sizer.add(&search_in_conv_btn, 0, SizerFlag::Right, 5);

	let place_call_btn = Button::builder(&top_btns_panel).with_label("Call").build();
	place_call_btn.set_font(&theme::Fonts::body());
	top_btns_sizer.add(&place_call_btn, 0, SizerFlag::Right, 5);

	top_btns_panel.set_sizer(top_btns_sizer, true);
	conv_sizer.add(&top_btns_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top | SizerFlag::Bottom, 5);

	let search_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&search_panel);
	let search_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let search_close_btn = Button::builder(&search_panel).with_label("Close search").build();
	search_sizer.add(&search_close_btn, 0, SizerFlag::Right, 5);
	let search_conv_label = StaticText::builder(&search_panel).with_label("Search conversation").build();
	theme::style_muted_label(&search_conv_label);
	search_sizer.add(&search_conv_label, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	let search_conv_field = TextCtrl::builder(&search_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	search_conv_field.set_font(&theme::Fonts::body());
	search_sizer.add(&search_conv_field, 1, SizerFlag::Expand | SizerFlag::Right, 5);
	search_panel.set_sizer(search_sizer, true);
	search_panel.show(false);
	conv_sizer.add(&search_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	let messages_label = StaticText::builder(&conversation_panel).with_label("Messages").build();
	theme::style_muted_label(&messages_label);
	conv_sizer.add(&messages_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let messages_list = ListBox::builder(&conversation_panel).build();
	messages_list.set_font(&theme::Fonts::body());
	theme::style_chat_area(&messages_list);
	conv_sizer.add(&messages_list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let action_panel = Panel::builder(&conversation_panel).build();
	theme::style_chat_area(&action_panel);
	let action_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let action_spacer = StaticText::builder(&action_panel).with_size(Size::new(1, 90)).with_label("").build();
	action_sizer.add(&action_spacer, 0, SizerFlag::Left, 0);

	let play_voice_btn = Button::builder(&action_panel).with_label("Play voice message").build();
	action_sizer.add(&play_voice_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	play_voice_btn.show(false);

	let messages_announcer = StaticText::builder(&action_panel).with_label("").build();
	live_region::set_live_region(&messages_announcer);
	action_sizer.add(&messages_announcer, 0, SizerFlag::Left, 0);
	messages_announcer.show(false);

	let read_more_btn = Button::builder(&action_panel).with_label("Read more").build();
	action_sizer.add(&read_more_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	read_more_btn.show(false);

	let action_open_btn = Button::builder(&action_panel).with_label("Open").build();
	action_sizer.add(&action_open_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_open_btn.show(false);

	let action_save_as_btn = Button::builder(&action_panel).with_label("Save as").build();
	action_sizer.add(&action_save_as_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_save_as_btn.show(false);

	let action_download_btn = Button::builder(&action_panel).with_label("Download").build();
	action_sizer.add(&action_download_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_download_btn.show(false);

	let audio_speed_btn = Button::builder(&action_panel).with_label("1.0x").build();
	action_sizer.add(&audio_speed_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	audio_speed_btn.show(false);

	let audio_progress_label = Button::builder(&action_panel).with_label("0:00 / 0:00").build();
	action_sizer.add(&audio_progress_label, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	audio_progress_label.show(false);

	let audio_slider = Slider::builder(&action_panel).with_min_value(0).with_max_value(1000).build();
	action_sizer.add(&audio_slider, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 5);
	audio_slider.show(false);

	action_panel.set_sizer(action_sizer, true);
	action_panel.show(false);
	conv_sizer.add(&action_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 0);

	let message_label = StaticText::builder(&conversation_panel).with_label("Type a message").build();
	theme::style_muted_label(&message_label);
	conv_sizer.add(&message_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let message_field = TextCtrl::builder(&conversation_panel)
		.with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ProcessEnter)
		.build();
	message_field.set_font(&theme::Fonts::body());
	message_field.set_min_size(Size::new(-1, 60));
	conv_sizer.add(&message_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	// Composer toolbar: attach / edit-reply state / send / record, laid out as one
	// row instead of a tall stack of buttons. Creation order — which is what
	// decides tab order — is deliberately identical to the old stacked layout,
	// and the buttons are added to the row in that same order so that what a
	// sighted user reads left to right is what a keyboard user tabs through.
	// Reordering these is a behaviour change for anyone driving the composer by
	// keyboard, not a layout detail.
	let composer_toolbar_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let add_attachment_btn = Button::builder(&conversation_panel).with_label("Add attachment").build();
	add_attachment_btn.set_font(&theme::Fonts::body());
	composer_toolbar_sizer.add(&add_attachment_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);

	let cancel_edit_btn = Button::builder(&conversation_panel).with_label("Cancel edit").build();
	cancel_edit_btn.set_font(&theme::Fonts::body());
	composer_toolbar_sizer.add(&cancel_edit_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	cancel_edit_btn.show(false);

	let remove_quote_btn = Button::builder(&conversation_panel).with_label("Remove quote").build();
	remove_quote_btn.set_font(&theme::Fonts::body());
	composer_toolbar_sizer.add(&remove_quote_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	remove_quote_btn.show(false);

	composer_toolbar_sizer.add_stretch_spacer(1);

	let send_message_btn = Button::builder(&conversation_panel).with_label("Send message").build();
	theme::style_primary_button(&send_message_btn);
	composer_toolbar_sizer.add(&send_message_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);

	let record_voice_btn = Button::builder(&conversation_panel).with_label("Record voice message").build();
	record_voice_btn.set_font(&theme::Fonts::body());
	composer_toolbar_sizer.add(&record_voice_btn, 0, SizerFlag::AlignCenterVertical, 0);

	conv_sizer.add_sizer(
		&composer_toolbar_sizer,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top | SizerFlag::Bottom,
		10,
	);

	let attachment_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&attachment_panel);
	let attach_sizer = BoxSizer::builder(Orientation::Vertical).build();
	let caption_label = StaticText::builder(&attachment_panel).with_label("Caption").build();
	theme::style_muted_label(&caption_label);
	attach_sizer.add(&caption_label, 0, SizerFlag::Left | SizerFlag::Top, 5);
	let caption_field = TextCtrl::builder(&attachment_panel).with_style(TextCtrlStyle::ProcessEnter).build();
	caption_field.set_font(&theme::Fonts::body());
	attach_sizer.add(&caption_field, 0, SizerFlag::Expand | SizerFlag::All, 5);
	let attachment_info_label = StaticText::builder(&attachment_panel).with_label("").build();
	theme::style_muted_label(&attachment_info_label);
	attach_sizer.add(&attachment_info_label, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 5);
	let add_more_btn = Button::builder(&attachment_panel).with_label("Add more files").build();
	add_more_btn.set_font(&theme::Fonts::body());
	attach_sizer.add(&add_more_btn, 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Bottom, 5);
	let send_attachment_btn = Button::builder(&attachment_panel).with_label("Send attachment").build();
	theme::style_primary_button(&send_attachment_btn);
	attach_sizer.add(&send_attachment_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	attachment_panel.set_sizer(attach_sizer, true);
	attachment_panel.show(false);
	conv_sizer.add(&attachment_panel, 0, SizerFlag::Expand | SizerFlag::All, 5);

	// Voice recording toolbar: same "row, not stack" treatment as the composer.
	let voice_toolbar_sizer = BoxSizer::builder(Orientation::Horizontal).build();

	let discard_voice_btn = Button::builder(&conversation_panel).with_label("Discard voice message").build();
	discard_voice_btn.set_font(&theme::Fonts::body());
	voice_toolbar_sizer.add(&discard_voice_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	discard_voice_btn.show(false);

	let pause_resume_btn = Button::builder(&conversation_panel).with_label("Pause recording").build();
	pause_resume_btn.set_font(&theme::Fonts::body());
	voice_toolbar_sizer.add(&pause_resume_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	pause_resume_btn.show(false);

	let send_voice_btn = Button::builder(&conversation_panel).with_label("Send voice message").build();
	theme::style_primary_button(&send_voice_btn);
	voice_toolbar_sizer.add(&send_voice_btn, 0, SizerFlag::AlignCenterVertical, 0);
	send_voice_btn.show(false);

	conv_sizer.add_sizer(
		&voice_toolbar_sizer,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		10,
	);

	conversation_panel.set_sizer(conv_sizer, true);
	conversation_panel.show(false);

	outer_sizer.add(&conversation_panel, 2, SizerFlag::Expand | SizerFlag::All, 5);
	panel.set_sizer(outer_sizer, true);

	ConversationsPanelParts {
		panel,
		search_label,
		search_field,
		new_conv_btn,
		conversations_label,
		conversations_list,
		conversation_panel,
		conv_data_btn,
		add_attachment_btn,
		search_in_conv_btn,
		place_call_btn,
		search_panel,
		search_close_btn,
		search_conv_label,
		search_conv_field,
		messages_label,
		messages_list,
		play_voice_btn,
		messages_announcer,
		read_more_btn,
		message_label,
		message_field,
		cancel_edit_btn,
		remove_quote_btn,
		send_message_btn,
		record_voice_btn,

		discard_voice_btn,
		pause_resume_btn,
		send_voice_btn,
		attachment_panel,
		caption_label,
		caption_field,
		send_attachment_btn,
		add_more_btn,
		attachment_info_label,
		audio_speed_btn,
		audio_progress_label,
		audio_slider,
		action_panel,
		action_open_btn,
		action_save_as_btn,
		action_download_btn,
	}
}
