//! Alert 1 to Alert 10, the notification tones WhatsApp for Windows offers
//! next to its default sound. These are that app's own Sounds/Alert-01.wma
//! to Alert-10.wma, turned into small Opus files.

use rodio::buffer::SamplesBuffer;

const ALERTS: [&[u8]; 10] = [
	include_bytes!("../../sounds/alert_01.ogg"),
	include_bytes!("../../sounds/alert_02.ogg"),
	include_bytes!("../../sounds/alert_03.ogg"),
	include_bytes!("../../sounds/alert_04.ogg"),
	include_bytes!("../../sounds/alert_05.ogg"),
	include_bytes!("../../sounds/alert_06.ogg"),
	include_bytes!("../../sounds/alert_07.ogg"),
	include_bytes!("../../sounds/alert_08.ogg"),
	include_bytes!("../../sounds/alert_09.ogg"),
	include_bytes!("../../sounds/alert_10.ogg"),
];

/// The sound of a [`whatsnap::TONE_DEFAULT`] style tone number from Alert 1
/// (2) to Alert 10 (11), or `None` for any other number.
pub(crate) fn alert_tone(tone: u8) -> Option<SamplesBuffer> {
	super::decode_opus(ALERTS.get(usize::from(tone).checked_sub(2)?)?)
}

#[cfg(test)]
mod tests {
	use rodio::Source;

	use super::alert_tone;

	#[test]
	fn only_alerts_one_to_ten_have_a_sound() {
		assert!(alert_tone(whatsnap::TONE_NONE).is_none());
		assert!(alert_tone(whatsnap::TONE_DEFAULT).is_none());
		assert!(alert_tone(whatsnap::TONE_LAST + 1).is_none());
		for tone in 2..=whatsnap::TONE_LAST {
			let alert = alert_tone(tone).unwrap();
			let seconds = alert.total_duration().unwrap().as_secs_f32();
			assert!((1.0..3.0).contains(&seconds), "Alert {} lasts {seconds}s", tone - 1);
		}
	}
}
