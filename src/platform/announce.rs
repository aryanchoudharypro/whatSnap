use wxdragon::prelude::StaticText;

pub(crate) fn announce(label: StaticText, message: &str) {
	let localized = crate::i18n::tr_str(message);
	live_region::announce_with_priority(label, &localized, live_region::Priority::Medium);
}
