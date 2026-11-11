#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
	pub year: i32,
	pub month: u32,
	pub day: u32,
	pub hour: u32,
	pub minute: u32,
	pub second: u32,
	pub weekday: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i64);

impl Date {
	pub fn add_days(self, days: i64) -> Date {
		Date(self.0 + days)
	}

	pub fn days_since(self, earlier: Date) -> i64 {
		self.0 - earlier.0
	}
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAYS_ID: [&str; 7] = ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"];
const MONTHS: [&str; 12] = [
	"January",
	"February",
	"March",
	"April",
	"May",
	"June",
	"July",
	"August",
	"September",
	"October",
	"November",
	"December",
];
const MONTHS_ID: [&str; 12] = [
	"Januari",
	"Februari",
	"Maret",
	"April",
	"Mei",
	"Juni",
	"Juli",
	"Agustus",
	"September",
	"Oktober",
	"November",
	"Desember",
];

impl LocalTime {
	#[cfg(windows)]
	pub fn from_unix(secs: i64) -> Option<LocalTime> {
		use windows::Win32::{
			Foundation::{FILETIME, SYSTEMTIME},
			System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
		};
		let ticks = secs.checked_add(11_644_473_600)?.checked_mul(10_000_000)?;
		let ticks = u64::try_from(ticks).ok()?;
		let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
		let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
		unsafe {
			FileTimeToSystemTime(&ft, &mut utc).ok()?;
			SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
		}
		Some(LocalTime {
			year: local.wYear as i32,
			month: local.wMonth as u32,
			day: local.wDay as u32,
			hour: local.wHour as u32,
			minute: local.wMinute as u32,
			second: local.wSecond as u32,
			weekday: local.wDayOfWeek as u32,
		})
	}

	#[cfg(not(windows))]
	pub fn from_unix(secs: i64) -> Option<LocalTime> {
		let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
		let (year, month, day) = civil_from_days(days);
		Some(LocalTime {
			year,
			month,
			day,
			hour: (rem / 3600) as u32,
			minute: (rem / 60 % 60) as u32,
			second: (rem % 60) as u32,
			weekday: (days + 4).rem_euclid(7) as u32,
		})
	}

	pub fn now() -> LocalTime {
		let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
		LocalTime::from_unix(secs).unwrap_or(LocalTime {
			year: 1970,
			month: 1,
			day: 1,
			hour: 0,
			minute: 0,
			second: 0,
			weekday: 4,
		})
	}

	pub fn date(&self) -> Date {
		Date(days_from_civil(self.year, self.month, self.day))
	}

	pub fn format(&self, fmt: &str) -> String {
		use std::fmt::Write;
		let mut out = String::with_capacity(fmt.len() + 16);
		let mut chars = fmt.chars();
		let hour12 = match self.hour % 12 {
			0 => 12,
			h => h,
		};
		let is_id = crate::i18n::is_indonesian();
		let weekday = if is_id { WEEKDAYS_ID } else { WEEKDAYS }[self.weekday as usize % 7];
		let month = if is_id { MONTHS_ID } else { MONTHS }[(self.month as usize + 11) % 12];
		while let Some(c) = chars.next() {
			if c != '%' {
				out.push(c);
				continue;
			}
			let _ = match chars.next() {
				Some('Y') => write!(out, "{}", self.year),
				Some('m') => write!(out, "{:02}", self.month),
				Some('d') => write!(out, "{:02}", self.day),
				Some('e') => write!(out, "{:2}", self.day),
				Some('H') => write!(out, "{:02}", self.hour),
				Some('I') => write!(out, "{:02}", hour12),
				Some('l') => write!(out, "{:2}", hour12),
				Some('M') => write!(out, "{:02}", self.minute),
				Some('S') => write!(out, "{:02}", self.second),
				Some('p') => write!(out, "{}", if self.hour < 12 { "AM" } else { "PM" }),
				Some('A') => write!(out, "{weekday}"),
				Some('a') => write!(out, "{}", &weekday[..3]),
				Some('B') => write!(out, "{month}"),
				Some('%') => write!(out, "%"),
				Some('b') => write!(out, "{}", &month[..3]),
				Some(other) => write!(out, "%{other}"),
				None => write!(out, "%"),
			};
		}
		out
	}
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
	let y = year as i64 - (month <= 2) as i64;
	let era = y.div_euclid(400);
	let yoe = y - era * 400;
	let m = month as i64;
	let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day as i64 - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

#[cfg(any(not(windows), test))]
fn civil_from_days(days: i64) -> (i32, u32, u32) {
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
	let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
	((yoe + era * 400 + (month <= 2) as i64) as i32, month, day)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn civil_dates_round_trip() {
		for days in (-800_000..800_000).step_by(37) {
			let (y, m, d) = civil_from_days(days);
			assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
		}
		assert_eq!(civil_from_days(0), (1970, 1, 1));
		assert_eq!(civil_from_days(19_782), (2024, 2, 29));
	}

	#[test]
	fn formats_like_strftime() {
		let t = LocalTime { year: 2024, month: 3, day: 5, hour: 0, minute: 7, second: 9, weekday: 2 };
		assert_eq!(t.format("%A, %B %e, %Y"), "Tuesday, March  5, 2024");
		assert_eq!(t.format("%Y-%m-%d %H:%M:%S"), "2024-03-05 00:07:09");
		assert_eq!(t.format("%I:%M %p"), "12:07 AM");
		assert_eq!(t.format("%l:%M %p"), "12:07 AM");
		assert_eq!(t.format("%b %e at %a %d/%m 100%%"), "Mar  5 at Tue 05/03 100%");
		let t = LocalTime { hour: 13, ..t };
		assert_eq!(t.format("%l:%M %p"), " 1:07 PM");
		assert_eq!(t.format("%I %p"), "01 PM");
	}

	#[test]
	fn dates_count_days() {
		let a = LocalTime { year: 2024, month: 2, day: 28, hour: 0, minute: 0, second: 0, weekday: 3 };
		let b = LocalTime { year: 2024, month: 3, day: 1, ..a };
		assert_eq!(b.date().days_since(a.date()), 2);
		assert_eq!(b.date().add_days(-2), a.date());
	}
}
