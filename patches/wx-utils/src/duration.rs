//! Spoken-form duration formatting, for durations a screen reader has to read out.

use patois::nt;

/// Formats a duration in whole seconds as a localized, comma-joined list of its non-zero
/// segments, such as `"1 hour, 5 minutes, 3 seconds"`.
///
/// Written out in words rather than as `1:05:03` because this is text a screen reader reads
/// aloud, and a colon-separated form is liable to be read as a time of day.
///
/// A zero segment is left out, so an exact hour is `"1 hour"` rather than `"1 hour, 0 minutes,
/// 0 seconds"`. A zero duration is the one exception, and is `"0 seconds"` rather than nothing
/// at all.
#[must_use]
pub fn format_duration_seconds(total_seconds: u64) -> String {
	let hours = total_seconds / 3600;
	let minutes = (total_seconds % 3600) / 60;
	let seconds = total_seconds % 60;
	let mut parts: Vec<String> = Vec::new();
	if hours >= 1 {
		// TRANSLATORS: Duration segment for hours (e.g. "1 hour" / "5 hours"). The %d placeholder is replaced with the count.
		parts.push(nt("%d hour", "%d hours", hours).replacen("%d", &hours.to_string(), 1));
	}
	if minutes >= 1 {
		// TRANSLATORS: Duration segment for minutes (e.g. "1 minute" / "5 minutes"). The %d placeholder is replaced with the count.
		parts.push(nt("%d minute", "%d minutes", minutes).replacen("%d", &minutes.to_string(), 1));
	}
	if seconds >= 1 || total_seconds == 0 {
		// TRANSLATORS: Duration segment for seconds (e.g. "1 second" / "5 seconds"). The %d placeholder is replaced with the count.
		parts.push(nt("%d second", "%d seconds", seconds).replacen("%d", &seconds.to_string(), 1));
	}
	parts.join(", ")
}

/// [`format_duration_seconds`] for a duration given in milliseconds, rounded down to the second.
#[must_use]
pub fn format_duration_ms(total_ms: u64) -> String {
	format_duration_seconds(total_ms / 1000)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn joins_nonzero_segments() {
		assert_eq!(format_duration_seconds(3665), "1 hour, 1 minute, 5 seconds");
	}

	#[test]
	fn omits_zero_segments() {
		assert_eq!(format_duration_seconds(3600), "1 hour");
		assert_eq!(format_duration_seconds(60), "1 minute");
	}

	#[test]
	fn zero_shows_zero_seconds() {
		assert_eq!(format_duration_seconds(0), "0 seconds");
	}

	#[test]
	fn pluralizes_each_segment_independently() {
		assert_eq!(format_duration_seconds(2 * 3600 + 5 * 60 + 1), "2 hours, 5 minutes, 1 second");
	}

	#[test]
	fn milliseconds_floor_to_the_nearest_second() {
		assert_eq!(format_duration_ms(2 * 60 * 60 * 1000 + 999), "2 hours");
	}
}
