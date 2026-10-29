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
	/// All chats, Unread, Favorites, Groups, then the user's own lists.
	/// Only shown in Conversations.
	pub chat_filter_label: StaticText,
	pub chat_filter_combo: ComboBox,
	pub new_conv_btn: Button,
	pub locked_chats_btn: Button,
	pub post_media_status_btn: Button,

	pub conversations_label: StaticText,
	pub conversations_list: ListBox,
	pub calls_btn_panel: Panel,
	pub calls_call_btn: Button,
	pub calls_video_btn: Button,

	pub conversation_panel: Panel,
	pub conv_data_btn: Button,
	pub add_attachment_btn: Button,
	/// "Emoji and stickers", just before the message box.
	pub emoji_btn: Button,
	pub search_in_conv_btn: Button,
	pub place_call_btn: Button,
	pub place_video_call_btn: Button,
	pub join_call_btn: Button,
	pub top_btns_panel: Panel,

	pub search_panel: Panel,
	pub search_close_btn: Button,
	pub search_conv_label: StaticText,
	pub search_conv_field: TextCtrl,

	pub messages_label: StaticText,
	pub messages_list: crate::ui::message_list::MessageList,
	pub messages_select_list: crate::ui::message_list::MessageList,

	pub selection_bar_panel: Panel,
	pub selection_count_label: StaticText,
	pub selection_delete_btn: Button,
	pub selection_cancel_btn: Button,

	pub play_voice_btn: Button,
	pub messages_announcer: StaticText,

	pub read_more_btn: Button,

	pub message_label: StaticText,
	pub message_field: TextCtrl,
	/// @mention suggestions, shown above the composer while completing one.
	/// Never focused: the composer keeps focus and announces the highlight.
	pub mention_list: ListBox,
	pub cancel_edit_btn: Button,
	pub remove_quote_btn: Button,
	pub send_message_btn: Button,
	pub record_voice_btn: Button,

	pub discard_voice_btn: Button,
	pub pause_resume_btn: Button,
	/// Sends the recording as view once: it can be played once, as on the phone.
	pub view_once_voice_check: CheckBox,
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
	pub action_retry_btn: Button,
	pub action_open_btn: Button,
	pub action_save_as_btn: Button,
	pub action_download_btn: Button,
}

/// Gives a list a fixed minimum size, so laying out its panel never asks the
/// list for its best size. wxListBox works that out by measuring the text of
/// every row, and any change to the list (a new message, a tick, the unread
/// divider going) throws the answer away. A whole chat is tens of thousands
/// of rows, so every layout after a change - showing or hiding Remove quote
/// when replying, the composer emptying after a send - froze the app for
/// seconds at a time. Both sides must be set: wx falls back to the best size
/// for any side left at -1. The sizer stretches the list as before.
fn skip_best_size(list: &impl WxWidget) {
	list.set_min_size(Size::new(100, 100));
}

/// wx's `wxBORDER_NONE`.
const BORDER_NONE: i64 = 0x0020_0000;

/// Draws the composer's grey bar across the chat, and the round white box
/// the message field sits in. Only painted; the field is the same control.
fn paint_composer(conversation: &Panel, field: &TextCtrl, bar: Colour) {
	if !theme::use_custom_colours() {
		return;
	}
	let (panel_c, field_c) = (*conversation, *field);
	panel_c.set_background_style(BackgroundStyle::Paint);
	panel_c.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		let background = panel_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		if !field_c.is_shown() {
			return;
		}
		let pos = field_c.get_position();
		let fs = field_c.get_size();
		let top = pos.y - 10;
		dc.set_brush(bar, BrushStyle::Solid);
		dc.set_pen(bar, 1, PenStyle::Solid);
		dc.draw_rectangle(0, top, size.width, fs.height + 20);
		let white = theme::Palette::CONTENT_BG;
		dc.set_brush(white, BrushStyle::Solid);
		dc.set_pen(white, 1, PenStyle::Solid);
		dc.draw_rounded_rectangle(pos.x - 8, pos.y - 5, fs.width + 16, fs.height + 10, 8.0);
	});
	panel_c.on_size(move |event| {
		panel_c.refresh(true, None);
		event.skip(true);
	});
}

/// The chat list column's width for a window `width` wide: about a third,
/// as WhatsApp Web has it, within bounds that keep both sides usable.
fn list_column_width(width: i32) -> i32 {
	(width * 3 / 10).clamp(280, 460)
}

/// Keeps the chat list column a third of the window as it's resized, and
/// draws WhatsApp's "pick a chat" page where the open chat goes while none
/// is open. It's only painted: a screen reader isn't told about it.
fn fit_columns(panel: &Panel, list: &ListBox, conversation: &Panel) {
	let (panel_c, list_c, conversation_c) = (*panel, *list, *conversation);
	panel.on_size(move |event| {
		let width = panel_c.get_client_size().width;
		// The column is as wide as its widest part, the list.
		list_c.set_min_size(Size::new(list_column_width(width) - 20, 100));
		panel_c.refresh(true, None);
		event.skip(true);
	});
	panel.set_background_style(BackgroundStyle::Paint);
	panel.on_paint(move |_| {
		let dc = AutoBufferedPaintDC::new(&panel_c);
		let size = panel_c.get_client_size();
		let background = panel_c.get_background_color();
		dc.set_brush(background, BrushStyle::Solid);
		dc.set_pen(background, 1, PenStyle::Solid);
		dc.draw_rectangle(0, 0, size.width, size.height);
		let left = list_c.get_position().x + list_c.get_size().width + 10;
		let custom = theme::use_custom_colours();
		let line = if custom { theme::Palette::DIVIDER_SOFT } else { panel_c.get_foreground_color() };
		dc.set_pen(line, 1, PenStyle::Solid);
		dc.draw_line(left, 0, left, size.height);
		if conversation_c.is_shown() {
			return;
		}
		let area = Size::new(size.width - left - 1, size.height);
		if area.width < 80 {
			return;
		}
		if custom {
			dc.set_brush(theme::Palette::PANEL_BG, BrushStyle::Solid);
			dc.set_pen(theme::Palette::PANEL_BG, 1, PenStyle::Solid);
			dc.draw_rectangle(left + 1, 0, area.width, area.height);
		}
		let icon = theme::build_app_icon(96);
		let cx = left + 1 + area.width / 2;
		let mut y = area.height / 2 - 110;
		dc.draw_bitmap(&icon, cx - 48, y, true);
		y += 120;
		dc.set_background_mode(BackgroundMode::Transparent);
		for (text, font, colour) in [
			(
				"whatSnap for Windows",
				theme::Fonts::heading(),
				if custom { theme::Palette::TEXT_PRIMARY } else { panel_c.get_foreground_color() },
			),
			(
				"Pick a chat on the left to start messaging.",
				theme::Fonts::body(),
				if custom { theme::Palette::TEXT_MUTED } else { panel_c.get_foreground_color() },
			),
			(
				"Your personal messages are end-to-end encrypted.",
				theme::Fonts::label(),
				if custom { theme::Palette::TEXT_MUTED } else { panel_c.get_foreground_color() },
			),
		] {
			dc.set_font(&font);
			dc.set_text_foreground(colour);
			let (w, h) = dc.get_text_extent(text);
			dc.draw_text(text, cx - w / 2, y);
			y += h + 10;
		}
	});
}

pub fn build_conversations_panel(parent: &impl WxWidget) -> ConversationsPanelParts {
	let panel = Panel::builder(parent).build();
	theme::style_content(&panel);
	// The chat list on the left and the open chat beside it, as on WhatsApp
	// Web. The left column is as wide as its list (see `fit_columns`).
	let outer_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let left_sizer = BoxSizer::builder(Orientation::Vertical).build();

	let search_label = StaticText::builder(&panel).with_label("Search chats").build();
	theme::style_muted_label(&search_label);
	left_sizer.add(&search_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	let search_field = TextCtrl::builder(&panel).build();
	search_field.set_font(&theme::Fonts::body());
	left_sizer.add(&search_field, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let chat_filter_label = StaticText::builder(&panel).with_label("Filters").build();
	theme::style_muted_label(&chat_filter_label);
	left_sizer.add(&chat_filter_label, 0, SizerFlag::Left | SizerFlag::Top, 10);
	let chat_filter_combo = ComboBox::builder(&panel)
		.with_string_choices(&crate::message_display::BUILT_IN_CHAT_FILTERS)
		.with_style(ComboBoxStyle::ReadOnly)
		.build();
	chat_filter_combo.set_font(&theme::Fonts::body());
	chat_filter_combo.set_selection(0);
	left_sizer.add(&chat_filter_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let new_conv_btn = Button::builder(&panel).with_label("New Chat").build();
	theme::style_primary_button(&new_conv_btn);
	left_sizer.add(&new_conv_btn, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);

	let locked_chats_btn = Button::builder(&panel).with_label("Locked Chats").build();
	left_sizer.add(
		&locked_chats_btn,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top | SizerFlag::Bottom,
		10,
	);
	locked_chats_btn.show(false);

	// Only shown in the Status nav view, alongside new_conv_btn (which posts
	// a text status there instead of opening the new-conversation dialog).
	let post_media_status_btn = Button::builder(&panel).with_label("Post Photo/Video Status...").build();
	left_sizer.add(
		&post_media_status_btn,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		10,
	);
	post_media_status_btn.show(false);

	let list_divider = StaticLine::builder(&panel).build();
	theme::style_divider(&list_divider);
	left_sizer.add(&list_divider, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

	let conversations_label = StaticText::builder(&panel).with_label("Chats").build();
	theme::style_heading(&conversations_label);
	left_sizer.add(&conversations_label, 0, SizerFlag::Left | SizerFlag::Top, 10);

	// Owner-drawn so its rows can show pictures and icons; a screen reader
	// still reads each row's text, as before (see ui::chat_rows).
	let conversations_list =
		ListBox::builder(&panel).with_style(ListBoxStyle::from_bits_retain(crate::ui::chat_rows::OWNER_DRAW)).build();
	conversations_list.set_font(&theme::Fonts::body());
	skip_best_size(&conversations_list);
	left_sizer.add(&conversations_list, 1, SizerFlag::Expand | SizerFlag::All, 10);

	let calls_btn_panel = Panel::builder(&panel).build();
	let calls_btn_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let calls_call_btn = Button::builder(&calls_btn_panel).with_label("Call").build();
	calls_call_btn.set_font(&theme::Fonts::body());
	theme::style_primary_button(&calls_call_btn);
	calls_btn_sizer.add(&calls_call_btn, 0, SizerFlag::Right, 5);

	let calls_video_btn = Button::builder(&calls_btn_panel).with_label("Video call").build();
	calls_video_btn.set_font(&theme::Fonts::body());
	calls_btn_sizer.add(&calls_video_btn, 0, SizerFlag::Right, 5);

	calls_btn_panel.set_sizer(calls_btn_sizer, true);
	calls_btn_panel.show(false);
	left_sizer.add(&calls_btn_panel, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom, 10);

	let conversation_panel = Panel::builder(&panel).build();
	theme::style_chat_area(&conversation_panel);
	let conv_sizer = BoxSizer::builder(Orientation::Vertical).build();

	// WhatsApp's header bar: the chat's picture, name and status, then its
	// buttons as icons. Each keeps its label for the screen reader.
	let top_btns_panel = Panel::builder(&conversation_panel).build();
	let header_bg = theme::Palette::PANEL_BG;
	if theme::use_custom_colours() {
		top_btns_panel.set_background_color(header_bg);
	}
	let top_btns_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let conv_data_btn = Button::builder(&top_btns_panel).with_label("Conversation data").build();
	conv_data_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_header(&conv_data_btn, header_bg);
	top_btns_sizer.add(&conv_data_btn, 1, SizerFlag::Right | SizerFlag::Expand, 5);

	let search_in_conv_btn = Button::builder(&top_btns_panel).with_label("Search in conversation").build();
	search_in_conv_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&search_in_conv_btn, header_bg);
	top_btns_sizer.add(&search_in_conv_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);

	let place_call_btn = Button::builder(&top_btns_panel).with_label("Call").build();
	place_call_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&place_call_btn, header_bg);
	top_btns_sizer.add(&place_call_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);

	let place_video_call_btn = Button::builder(&top_btns_panel).with_label("Video call").build();
	place_video_call_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&place_video_call_btn, header_bg);
	top_btns_sizer.add(&place_video_call_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);

	let join_call_btn = Button::builder(&top_btns_panel).with_label("Join call").build();
	join_call_btn.set_font(&theme::Fonts::body());
	theme::style_primary_button(&join_call_btn);
	top_btns_sizer.add(&join_call_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	join_call_btn.show(false);

	top_btns_panel.set_sizer(top_btns_sizer, true);
	conv_sizer.add(&top_btns_panel, 0, SizerFlag::Expand, 0);

	let search_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&search_panel);
	let search_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let search_close_btn = Button::builder(&search_panel).with_label("Close search").build();
	crate::ui::owner_draw::install_icon_button(&search_close_btn, theme::Palette::CONTENT_BG);
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

	// Kept for anything that reads the window's texts, but not shown: the
	// list says what it is itself, and WhatsApp has no such heading.
	let messages_label = StaticText::builder(&conversation_panel).with_label("Messages").build();
	theme::style_muted_label(&messages_label);
	messages_label.set_min_size(Size::new(1, 1));
	conv_sizer.add(&messages_label, 0, SizerFlag::Left, 0);

	let messages_list = crate::ui::message_list::MessageList::new(&conversation_panel, "Messages");
	messages_list.set_font(&theme::Fonts::body());
	skip_best_size(&messages_list);
	theme::style_chat_area(&messages_list);
	conv_sizer.add(&messages_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);

	// Shown in place of `messages_list` while selection mode is active - kept
	// as a separate widget rather than swapping messages_list's own type so
	// every existing single-selection call site (context menu, search jump,
	// keyboard nav) is unaffected by selection mode.
	let messages_select_list = crate::ui::message_list::MessageList::new(&conversation_panel, "Select messages");
	messages_select_list.set_checkable(true);
	messages_select_list.set_font(&theme::Fonts::body());
	skip_best_size(&messages_select_list);
	theme::style_chat_area(&messages_select_list);
	conv_sizer.add(&messages_select_list, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
	messages_select_list.show(false);

	let selection_bar_panel = Panel::builder(&conversation_panel).build();
	theme::style_content(&selection_bar_panel);
	let selection_bar_sizer = BoxSizer::builder(Orientation::Horizontal).build();
	let selection_count_label = StaticText::builder(&selection_bar_panel).with_label("0 selected").build();
	theme::style_muted_label(&selection_count_label);
	selection_bar_sizer.add(&selection_count_label, 0, SizerFlag::Left | SizerFlag::AlignCenterVertical, 5);
	selection_bar_sizer.add_stretch_spacer(1);
	let selection_delete_btn = Button::builder(&selection_bar_panel).with_label("Delete").build();
	selection_delete_btn.set_font(&theme::Fonts::body());
	selection_bar_sizer.add(&selection_delete_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	let selection_cancel_btn = Button::builder(&selection_bar_panel).with_label("Cancel").build();
	selection_cancel_btn.set_font(&theme::Fonts::body());
	selection_bar_sizer.add(&selection_cancel_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	selection_bar_panel.set_sizer(selection_bar_sizer, true);
	conv_sizer.add(
		&selection_bar_panel,
		0,
		SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Bottom,
		10,
	);
	selection_bar_panel.show(false);

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

	let action_retry_btn = Button::builder(&action_panel).with_label("Retry send").build();
	action_sizer.add(&action_retry_btn, 0, SizerFlag::Left | SizerFlag::Bottom, 5);
	action_retry_btn.show(false);

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

	// The emoji and stickers panel's button (WhatsApp Web's sits beside its
	// message box), then the label, then the box itself. Created in that
	// order: Windows names the box from the text just before it, so the
	// button must come ahead of the label, and Shift+Tab from the box still
	// reaches the button.
	// WhatsApp's composer: one grey bar with the emoji button, the message
	// box (drawn white and round, see `paint_composer`), then the attach,
	// send and mic buttons as icons. Creation order - which decides tab
	// order and names the box from the label just before it - is the same as
	// it always was, and the row is laid out in that same order, so what a
	// sighted user reads left to right is what a keyboard user tabs through:
	// emoji, label, box, attach, cancel edit, remove quote, send, record.
	let composer_bg = theme::Palette::PANEL_BG;
	let message_row = BoxSizer::builder(Orientation::Horizontal).build();
	let emoji_btn = Button::builder(&conversation_panel).with_label("Emoji and stickers").build();
	emoji_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&emoji_btn, composer_bg);
	message_row.add(&emoji_btn, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 4);
	let message_label = StaticText::builder(&conversation_panel).with_label("Type a message").build();
	theme::style_muted_label(&message_label);
	if theme::use_custom_colours() {
		message_label.set_background_color(composer_bg);
	}
	message_row.add(&message_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
	// No border of its own: it sits in the round white box drawn behind it.
	let message_field = TextCtrl::builder(&conversation_panel)
		.with_style(
			TextCtrlStyle::MultiLine | TextCtrlStyle::ProcessEnter | TextCtrlStyle::from_bits_retain(BORDER_NONE),
		)
		.build();
	message_field.set_font(&theme::Fonts::body());
	message_field.set_min_size(Size::new(-1, message_field.get_char_height() * 2 + 6));
	message_row.add(&message_field, 1, SizerFlag::AlignCenterVertical | SizerFlag::Right, 12);
	// Created after the field, so it's never ahead of it in tab order.
	let mention_list = ListBox::builder(&conversation_panel).build();
	mention_list.set_font(&theme::Fonts::body());
	mention_list.set_min_size(Size::new(-1, 120));
	conv_sizer.add(&mention_list, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 10);
	mention_list.show(false);
	let add_attachment_btn = Button::builder(&conversation_panel).with_label("Add attachment").build();
	add_attachment_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&add_attachment_btn, composer_bg);
	message_row.add(&add_attachment_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	let cancel_edit_btn = Button::builder(&conversation_panel).with_label("Cancel edit").build();
	cancel_edit_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&cancel_edit_btn, composer_bg);
	message_row.add(&cancel_edit_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	cancel_edit_btn.show(false);
	let remove_quote_btn = Button::builder(&conversation_panel).with_label("Remove quote").build();
	remove_quote_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&remove_quote_btn, composer_bg);
	message_row.add(&remove_quote_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	remove_quote_btn.show(false);
	let send_message_btn = Button::builder(&conversation_panel).with_label("Send message").build();
	send_message_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&send_message_btn, composer_bg);
	message_row.add(&send_message_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 2);
	let record_voice_btn = Button::builder(&conversation_panel).with_label("Record voice message").build();
	record_voice_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&record_voice_btn, composer_bg);
	message_row.add(&record_voice_btn, 0, SizerFlag::AlignCenterVertical, 0);
	conv_sizer.add_sizer(&message_row, 0, SizerFlag::Expand | SizerFlag::All, 10);
	paint_composer(&conversation_panel, &message_field, composer_bg);
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
	crate::ui::owner_draw::install_icon_button(&discard_voice_btn, theme::Palette::CHAT_BG);
	voice_toolbar_sizer.add(&discard_voice_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	discard_voice_btn.show(false);

	let pause_resume_btn = Button::builder(&conversation_panel).with_label("Pause recording").build();
	pause_resume_btn.set_font(&theme::Fonts::body());
	crate::ui::owner_draw::install_icon_button(&pause_resume_btn, theme::Palette::CHAT_BG);
	voice_toolbar_sizer.add(&pause_resume_btn, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	pause_resume_btn.show(false);

	let view_once_voice_check = CheckBox::builder(&conversation_panel).with_label("View once").build();
	view_once_voice_check.set_font(&theme::Fonts::body());
	voice_toolbar_sizer.add(&view_once_voice_check, 0, SizerFlag::Right | SizerFlag::AlignCenterVertical, 5);
	view_once_voice_check.show(false);

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

	outer_sizer.add_sizer(&left_sizer, 0, SizerFlag::Expand, 0);
	outer_sizer.add(&conversation_panel, 1, SizerFlag::Expand, 0);
	panel.set_sizer(outer_sizer, true);
	fit_columns(&panel, &conversations_list, &conversation_panel);

	ConversationsPanelParts {
		panel,
		search_label,
		search_field,
		chat_filter_label,
		chat_filter_combo,
		new_conv_btn,
		locked_chats_btn,
		post_media_status_btn,
		conversations_label,
		conversations_list,
		calls_btn_panel,
		calls_call_btn,
		calls_video_btn,
		conversation_panel,
		conv_data_btn,
		add_attachment_btn,
		emoji_btn,
		search_in_conv_btn,
		place_call_btn,
		place_video_call_btn,
		join_call_btn,
		top_btns_panel,
		search_panel,
		search_close_btn,
		search_conv_label,
		search_conv_field,
		messages_label,
		messages_list,
		messages_select_list,
		selection_bar_panel,
		selection_count_label,
		selection_delete_btn,
		selection_cancel_btn,
		play_voice_btn,
		messages_announcer,
		read_more_btn,
		message_label,
		message_field,
		mention_list,
		cancel_edit_btn,
		remove_quote_btn,
		send_message_btn,
		record_voice_btn,

		discard_voice_btn,
		pause_resume_btn,
		view_once_voice_check,
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
		action_retry_btn,
		action_open_btn,
		action_save_as_btn,
		action_download_btn,
	}
}
