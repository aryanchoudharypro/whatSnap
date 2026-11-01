//! Screen reader announcements. They go out at Medium priority: NVDA finishes
//! what it's saying, then speaks the newest one, instead of being cut off.
//! Call notifications are the exception and interrupt (see
//! `call_helpers::announce_call`).

use wxdragon::prelude::StaticText;

pub(crate) fn announce(label: StaticText, message: &str) {
	live_region::announce_with_priority(label, message, live_region::Priority::Medium);
}
