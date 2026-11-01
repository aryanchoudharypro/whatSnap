use wxdragon::prelude::StaticText;

pub(crate) fn announce(label: StaticText, message: &str) {
	live_region::announce_with_priority(label, message, live_region::Priority::Medium);
}
