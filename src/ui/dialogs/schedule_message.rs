use wxdragon::prelude::*;

use crate::ui::{dialog_look, theme};

/// What the backend works out for WA Web's schedule time picker
/// (WAWebScheduleTimePicker): the days it offers, the time it starts on and
/// the window a time has to fall in, as local "YYYY-MM-DD HH:MM".
pub struct ScheduleOptions {
	pub days: Vec<(String, String)>,
	pub default_date: String,
	pub default_time: String,
	pub earliest: String,
	pub latest: String,
	pub window_error: String,
	pub is_group: bool,
	pub pending: u64,
	pub limit: u64,
}

impl ScheduleOptions {
	pub fn from_json(data: &serde_json::Value) -> Self {
		let text = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
		let days = data
			.get("days")
			.and_then(|d| d.as_array())
			.into_iter()
			.flatten()
			.filter_map(|d| {
				let value = d.get("value")?.as_str()?.to_string();
				let label = d.get("label")?.as_str()?.to_string();
				Some((value, label))
			})
			.collect();
		ScheduleOptions {
			days,
			default_date: text("default_date"),
			default_time: text("default_time"),
			earliest: text("earliest"),
			latest: text("latest"),
			window_error: text("window_error"),
			is_group: data.get("is_group").and_then(|v| v.as_bool()).unwrap_or(false),
			pending: data.get("pending").and_then(|v| v.as_u64()).unwrap_or(0),
			limit: data.get("limit").and_then(|v| v.as_u64()).unwrap_or(30),
		}
	}
}

pub struct ScheduleChoice {
	pub text: String,
	pub date: String,
	pub time: String,
}

/// WA Web's IsScheduledTimeValid, on the local wall-clock strings the
/// backend sends (zero-padded, so they compare in time order).
fn in_window(earliest: &str, latest: &str, date: &str, time: &str) -> bool {
	let picked = format!("{date} {time}");
	picked.as_str() >= earliest && picked.as_str() <= latest
}

fn translate_day_label(label: &str) -> String {
	if !crate::i18n::is_indonesian() {
		return label.to_string();
	}
	let mut out = label.to_string();
	if let Some(rest) = out.strip_prefix("Today, ") {
		out = format!("Hari ini, {rest}");
	} else if let Some(rest) = out.strip_prefix("Tomorrow, ") {
		out = format!("Besok, {rest}");
	}
	for (en, id) in [
		("Sunday", "Minggu"),
		("Monday", "Senin"),
		("Tuesday", "Selasa"),
		("Wednesday", "Rabu"),
		("Thursday", "Kamis"),
		("Friday", "Jumat"),
		("Saturday", "Sabtu"),
		("January", "Januari"),
		("February", "Februari"),
		("March", "Maret"),
		("April", "April"),
		("May", "Mei"),
		("June", "Juni"),
		("July", "Juli"),
		("August", "Agustus"),
		("September", "September"),
		("October", "Oktober"),
		("November", "November"),
		("December", "Desember"),
	] {
		out = out.replace(en, id);
	}
	out
}

fn translate_window_error(err: &str) -> String {
	if !crate::i18n::is_indonesian() {
		return err.to_string();
	}
	if let Some(rest) = err.strip_prefix("Choose a time at least ")
		&& let Some((mins, tail)) = rest.split_once(" minutes from now and no more than ")
		&& let Some(days) = tail.strip_suffix(" days ahead.")
	{
		return format!("Pilih waktu minimal {mins} menit dari sekarang dan paling lambat {days} hari ke depan.");
	}
	crate::i18n::translate_backend_message(err)
}

fn error(parent: &Dialog, message: &str) {
	MessageDialog::builder(parent, message, crate::i18n::tr("Schedule message"))
		.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
		.build()
		.show_modal();
}

/// Asks for the message and when to send it. `text` is what was in the
/// message box.
pub fn show_schedule_dialog(parent: &impl WxWidget, options: &ScheduleOptions, text: &str) -> Option<ScheduleChoice> {
	let dialog = Dialog::builder(parent, crate::i18n::tr("Schedule message"))
		.with_size(460, 520)
		.with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
		.build();
	theme::style_content(&dialog);
	let panel = Panel::builder(&dialog).build();
	theme::style_content(&panel);
	let sizer = BoxSizer::builder(Orientation::Vertical).build();

	if options.is_group {
		let note = StaticText::builder(&panel)
			.with_label(crate::i18n::tr("Members who join after you schedule a message will not receive it."))
			.build();
		theme::style_muted_label(&note);
		sizer.add(&note, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	}

	let message_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Message")).build();
	message_label.set_font(&theme::Fonts::body());
	sizer.add(&message_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let message_field = TextCtrl::builder(&panel).with_style(TextCtrlStyle::MultiLine).build();
	message_field.set_font(&theme::Fonts::body());
	message_field.set_value(text);
	sizer.add(&message_field, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let date_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Date")).build();
	date_label.set_font(&theme::Fonts::body());
	sizer.add(&date_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let day_labels: Vec<String> = options.days.iter().map(|(_, label)| translate_day_label(label)).collect();
	let day_label_refs: Vec<&str> = day_labels.iter().map(String::as_str).collect();
	let date_combo =
		ComboBox::builder(&panel).with_string_choices(&day_label_refs).with_style(ComboBoxStyle::ReadOnly).build();
	date_combo.set_font(&theme::Fonts::body());
	let day_at = options.days.iter().position(|(value, _)| *value == options.default_date).unwrap_or(0);
	date_combo.set_selection(day_at as u32);
	sizer.add(&date_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let (default_hour, default_minute) = options.default_time.split_once(':').unwrap_or(("09", "00"));
	let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
	let minutes: Vec<String> = (0..60).map(|m| format!("{m:02}")).collect();

	let hour_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Hour")).build();
	hour_label.set_font(&theme::Fonts::body());
	sizer.add(&hour_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let hour_refs: Vec<&str> = hours.iter().map(String::as_str).collect();
	let hour_combo =
		ComboBox::builder(&panel).with_string_choices(&hour_refs).with_style(ComboBoxStyle::ReadOnly).build();
	hour_combo.set_font(&theme::Fonts::body());
	hour_combo.set_selection(hours.iter().position(|h| h == default_hour).unwrap_or(9) as u32);
	sizer.add(&hour_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let minute_label = StaticText::builder(&panel).with_label(crate::i18n::tr("Minute")).build();
	minute_label.set_font(&theme::Fonts::body());
	sizer.add(&minute_label, 0, SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);
	let minute_refs: Vec<&str> = minutes.iter().map(String::as_str).collect();
	let minute_combo =
		ComboBox::builder(&panel).with_string_choices(&minute_refs).with_style(ComboBoxStyle::ReadOnly).build();
	minute_combo.set_font(&theme::Fonts::body());
	minute_combo.set_selection(minutes.iter().position(|m| m == default_minute).unwrap_or(0) as u32);
	sizer.add(&minute_combo, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let info_text = if crate::i18n::is_indonesian() {
		let rule = translate_window_error(&options.window_error).replacen("Pilih waktu", "Waktu harus", 1);
		format!("{} {} dari {} pesan dijadwalkan di chat ini.", rule, options.pending, options.limit)
	} else {
		format!(
			"{} {} of {} messages are scheduled in this chat.",
			options.window_error.replacen("Choose a time", "The time has to be", 1),
			options.pending,
			options.limit
		)
	};
	let info = StaticText::builder(&panel).with_label(&info_text).build();
	theme::style_muted_label(&info);
	sizer.add(&info, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right | SizerFlag::Top, 12);

	let schedule_btn = Button::builder(&panel).with_label(crate::i18n::tr("Schedule message")).build();
	dialog_look::pill_button(&schedule_btn, true, theme::Palette::CONTENT_BG());
	let cancel_btn = Button::builder(&panel).with_id(ID_CANCEL).with_label(crate::i18n::tr("Cancel")).build();
	dialog_look::pill_button(&cancel_btn, false, theme::Palette::CONTENT_BG());
	dialog_look::button_row(&sizer, &[&schedule_btn, &cancel_btn]);
	panel.set_sizer(sizer, true);

	let outer = BoxSizer::builder(Orientation::Vertical).build();
	outer.add(&panel, 1, SizerFlag::Expand, 0);
	dialog.set_sizer(outer, true);
	dialog.center();
	crate::ui::keys::escape_cancels_anywhere(dialog);

	let picked_time = move || {
		let hour = hour_combo.get_selection().unwrap_or(0);
		let minute = minute_combo.get_selection().unwrap_or(0);
		format!("{hour:02}:{minute:02}")
	};
	let days = options.days.clone();
	let picked_date = move || {
		date_combo
			.get_selection()
			.and_then(|i| days.get(i as usize))
			.map(|(value, _)| value.clone())
			.unwrap_or_default()
	};

	let window_error = translate_window_error(&options.window_error);
	let (earliest, latest) = (options.earliest.clone(), options.latest.clone());
	let (date_check, time_check) = (picked_date.clone(), picked_time.clone());
	schedule_btn.on_click(move |_| {
		if message_field.get_value().trim().is_empty() {
			error(&dialog, crate::i18n::tr("Type a message to schedule."));
			message_field.set_focus();
			return;
		}
		if !in_window(&earliest, &latest, &date_check(), &time_check()) {
			error(&dialog, &window_error);
			hour_combo.set_focus();
			return;
		}
		dialog.end_modal(ID_OK);
	});
	cancel_btn.on_click(move |_| dialog.end_modal(ID_CANCEL));

	if text.trim().is_empty() {
		message_field.set_focus();
	} else {
		date_combo.set_focus();
	}

	let scheduled = dialog.show_modal() == ID_OK;
	let choice = ScheduleChoice { text: message_field.get_value(), date: picked_date(), time: picked_time() };
	dialog.destroy();
	scheduled.then_some(choice)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn times_outside_the_window_are_refused() {
		let (earliest, latest) = ("2026-11-09 10:25", "2026-11-23 10:15");
		assert!(in_window(earliest, latest, "2026-11-09", "10:25"));
		assert!(!in_window(earliest, latest, "2026-11-09", "10:24"));
		assert!(in_window(earliest, latest, "2026-11-23", "10:15"));
		assert!(!in_window(earliest, latest, "2026-11-23", "10:16"));
		assert!(in_window(earliest, latest, "2026-11-15", "00:00"));
	}

	#[test]
	fn options_are_read_from_the_backend() {
		let data = serde_json::json!({
			"days": [{"value": "2026-11-09", "label": "Today, Monday 9 November"}],
			"default_date": "2026-11-09",
			"default_time": "17:00",
			"earliest": "2026-11-09 10:25",
			"latest": "2026-11-23 10:15",
			"is_group": true,
			"pending": 2,
			"limit": 30
		});
		let options = ScheduleOptions::from_json(&data);
		assert_eq!(options.days.len(), 1);
		assert_eq!(options.default_time, "17:00");
		assert!(options.is_group);
		assert_eq!(options.pending, 2);
	}
}
