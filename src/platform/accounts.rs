//! The WhatsApp accounts linked on this computer (accounts.json).
//!
//! Every account has its own folder with its own session.db and media cache. The first account keeps the folder whatSnap always used (the
//! app data folder itself), so nothing moves for someone who only has one.
//! Later accounts live in `accounts/<id>`, where the id is 32 hex digits and is
//! checked before it is ever turned into a path.
//!
//! An account is `pending` until its phone links it, `paired` after that, and
//! `deleting` while its files are being removed. A removal that can't finish
//! (a file still in use) stays `deleting` and is finished on the next start.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::paths;

/// The account whose data is in the app data folder itself.
pub const FIRST_ACCOUNT: &str = "default";

const FILE_NAME: &str = "accounts.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountState {
	#[default]
	Pending,
	Paired,
	Deleting,
}

/// How messages to an account that isn't on screen alert.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageAlerts {
	/// Notification and sound, as the chat's own settings allow.
	#[default]
	Full,
	NoSound,
	SoundOnly,
	Off,
}

impl MessageAlerts {
	pub fn sound(self) -> bool {
		matches!(self, Self::Full | Self::SoundOnly)
	}

	pub fn toast(self) -> bool {
		matches!(self, Self::Full | Self::NoSound)
	}
}

/// What a call to an account that isn't on screen does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallAlerts {
	/// Switch to the account and ring, unless a call or recording is on.
	#[default]
	Switch,
	/// A notification that says who is calling, and missed call notifications.
	Notify,
	Off,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
	pub id: String,
	/// The name the user gave the account, if any.
	#[serde(default)]
	pub label: String,
	/// The account's WhatsApp name.
	#[serde(default)]
	pub name: String,
	/// The account's phone number, digits only.
	#[serde(default)]
	pub phone: String,
	#[serde(default)]
	pub state: AccountState,
	/// Message alerts while another account is on screen.
	#[serde(default)]
	pub messages: MessageAlerts,
	/// Call alerts while another account is on screen.
	#[serde(default)]
	pub calls: CallAlerts,
}

impl Account {
	/// What the account is called in menus, titles and notifications: the
	/// user's own label, else the WhatsApp name, else the phone number.
	pub fn display_name(&self) -> String {
		if !self.label.trim().is_empty() {
			self.label.trim().to_string()
		} else if !self.name.trim().is_empty() {
			self.name.trim().to_string()
		} else if !self.phone.is_empty() {
			format!("+{}", self.phone)
		} else {
			crate::i18n::tr("New account").to_string()
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
	pub active: String,
	pub accounts: Vec<Account>,
}

pub fn is_valid_id(id: &str) -> bool {
	id == FIRST_ACCOUNT || (id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// The folder holding an account's database, media cache and logs.
pub fn data_dir(id: &str) -> PathBuf {
	data_dir_in(&paths::app_data_dir(), id)
}

fn data_dir_in(base: &Path, id: &str) -> PathBuf {
	assert!(is_valid_id(id), "invalid account id");
	if id == FIRST_ACCOUNT { base.to_path_buf() } else { base.join("accounts").join(id) }
}

pub fn db_path(id: &str) -> PathBuf {
	data_dir(id).join("session.db")
}

/// Phone number digits from an account's own JID ("91981...:12@s.whatsapp.net").
pub fn phone_from_jid(jid: &str) -> String {
	let user = jid.split('@').next().unwrap_or("");
	let user = user.split(':').next().unwrap_or("");
	if jid.ends_with("@s.whatsapp.net") && user.bytes().all(|b| b.is_ascii_digit()) {
		user.to_string()
	} else {
		String::new()
	}
}

impl Registry {
	fn first_run(base: &Path) -> Self {
		let state = if base.join("session.db").exists() { AccountState::Paired } else { AccountState::Pending };
		Self {
			active: FIRST_ACCOUNT.to_string(),
			accounts: vec![Account { id: FIRST_ACCOUNT.to_string(), state, ..Default::default() }],
		}
	}

	/// Reads accounts.json. A missing file means a single account in the old
	/// place. A file that can't be read is kept aside, never overwritten, and
	/// the accounts are found again from their folders, so a damaged file can't
	/// lose a linked account.
	pub fn load() -> Self {
		Self::load_from(&paths::app_data_dir())
	}

	fn load_from(base: &Path) -> Self {
		let path = base.join(FILE_NAME);
		let text = match std::fs::read_to_string(&path) {
			Ok(text) => text,
			Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::first_run(base),
			Err(_) => return Self::recover(base, &path),
		};
		match serde_json::from_str::<Registry>(&text) {
			Ok(mut registry) => {
				registry.tidy();
				registry
			}
			Err(_) => Self::recover(base, &path),
		}
	}

	fn recover(base: &Path, path: &Path) -> Self {
		let stamp =
			std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
		let _ = std::fs::rename(path, base.join(format!("{FILE_NAME}.corrupt-{stamp}")));
		let mut registry = Self::first_run(base);
		if let Ok(entries) = std::fs::read_dir(base.join("accounts")) {
			let mut found: Vec<String> = entries
				.flatten()
				.filter_map(|e| e.file_name().into_string().ok())
				.filter(|id| id != FIRST_ACCOUNT && is_valid_id(id))
				.filter(|id| base.join("accounts").join(id).join("session.db").exists())
				.collect();
			found.sort();
			for id in found {
				registry.accounts.push(Account { id, state: AccountState::Paired, ..Default::default() });
			}
		}
		let _ = registry.save_to(base);
		registry
	}

	/// Drops entries that can't be used (bad or repeated ids) and makes sure
	/// the active account is one that can be shown.
	fn tidy(&mut self) {
		let mut seen = std::collections::HashSet::new();
		self.accounts.retain(|a| is_valid_id(&a.id) && seen.insert(a.id.clone()));
		if self.accounts.iter().all(|a| a.state == AccountState::Deleting) {
			if let Some(first) = self.accounts.iter_mut().find(|a| a.id == FIRST_ACCOUNT) {
				first.state = AccountState::Pending;
			} else {
				self.accounts.push(Account { id: FIRST_ACCOUNT.to_string(), ..Default::default() });
			}
		}
		if !self.live().any(|a| a.id == self.active) {
			let first = self.live().next().map(|a| a.id.clone());
			self.active = first.unwrap_or_else(|| FIRST_ACCOUNT.to_string());
		}
	}

	pub fn save(&self) -> std::io::Result<()> {
		self.save_to(&paths::app_data_dir())
	}

	/// Written to a temporary file and moved over the old one, so a crash
	/// mid-write leaves either the old list or the new one, never half of one.
	fn save_to(&self, base: &Path) -> std::io::Result<()> {
		let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
		let tmp = base.join(format!("{FILE_NAME}.tmp"));
		std::fs::write(&tmp, text)?;
		std::fs::rename(&tmp, base.join(FILE_NAME))
	}

	/// Accounts that aren't being removed, in the order they were added.
	pub fn live(&self) -> impl Iterator<Item = &Account> {
		self.accounts.iter().filter(|a| a.state != AccountState::Deleting)
	}

	pub fn live_count(&self) -> usize {
		self.live().count()
	}

	pub fn get(&self, id: &str) -> Option<&Account> {
		self.accounts.iter().find(|a| a.id == id)
	}

	pub fn get_mut(&mut self, id: &str) -> Option<&mut Account> {
		self.accounts.iter_mut().find(|a| a.id == id)
	}

	pub fn display_name(&self, id: &str) -> String {
		self.get(id).map(Account::display_name).unwrap_or_else(|| crate::i18n::tr("New account").to_string())
	}

	/// A new account waiting to be linked, with its folder made.
	pub fn add(&mut self, label: &str) -> std::io::Result<String> {
		self.add_in(&paths::app_data_dir(), label)
	}

	fn add_in(&mut self, base: &Path, label: &str) -> std::io::Result<String> {
		let id = loop {
			let id = crate::util::random_hex();
			if self.get(&id).is_none() {
				break id;
			}
		};
		std::fs::create_dir_all(data_dir_in(base, &id))?;
		self.accounts.push(Account { id: id.clone(), label: label.trim().to_string(), ..Default::default() });
		Ok(id)
	}

	/// Records who the account turned out to be once it connected. Returns
	/// true when something changed.
	pub fn set_identity(&mut self, id: &str, jid: Option<&str>, name: Option<&str>) -> bool {
		let Some(account) = self.get_mut(id) else { return false };
		let before = account.clone();
		if let Some(jid) = jid {
			let phone = phone_from_jid(jid);
			if !phone.is_empty() {
				account.phone = phone;
			}
			if account.state == AccountState::Pending {
				account.state = AccountState::Paired;
			}
		}
		if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
			account.name = name.to_string();
		}
		*account != before
	}

	/// Another linked account with the same phone number, if any: linking one
	/// number twice would give two folders for one WhatsApp account.
	pub fn duplicate_of(&self, id: &str) -> Option<&Account> {
		let phone = &self.get(id)?.phone;
		if phone.is_empty() {
			return None;
		}
		self.live().find(|a| a.id != id && &a.phone == phone)
	}
}

/// Deletes an account's files. The first account shares its folder with the
/// app's own settings and logs, so only its database and media go. True when
/// nothing was left behind.
pub fn purge_files(id: &str) -> bool {
	purge_files_in(&paths::app_data_dir(), id)
}

fn purge_files_in(base: &Path, id: &str) -> bool {
	if !is_valid_id(id) {
		return false;
	}
	let gone = |r: std::io::Result<()>| match r {
		Ok(()) => true,
		Err(err) => err.kind() == std::io::ErrorKind::NotFound,
	};
	let dir = data_dir_in(base, id);
	if id != FIRST_ACCOUNT {
		return gone(std::fs::remove_dir_all(&dir));
	}
	let mut complete = true;
	for name in ["session.db", "session.db-wal", "session.db-shm"] {
		complete &= gone(std::fs::remove_file(dir.join(name)));
	}
	complete & gone(std::fs::remove_dir_all(dir.join("media_cache")))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn alert_choices_default_and_round_trip() {
		let old: Account = serde_json::from_str(r#"{"id":"default","state":"paired"}"#).unwrap();
		assert_eq!((old.messages, old.calls), (MessageAlerts::Full, CallAlerts::Switch));
		let quiet = Account { messages: MessageAlerts::NoSound, calls: CallAlerts::Notify, ..old };
		let text = serde_json::to_string(&quiet).unwrap();
		assert!(text.contains(r#""messages":"no_sound""#) && text.contains(r#""calls":"notify""#));
		assert_eq!(serde_json::from_str::<Account>(&text).unwrap(), quiet);
		assert!(!MessageAlerts::NoSound.sound() && MessageAlerts::NoSound.toast());
		assert!(MessageAlerts::SoundOnly.sound() && !MessageAlerts::SoundOnly.toast());
	}

	fn temp_base(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("whatsnap-accounts-{name}-{}", crate::util::random_hex()));
		std::fs::create_dir_all(&dir).unwrap();
		dir
	}

	#[test]
	fn a_missing_file_is_one_account_in_the_old_place() {
		let base = temp_base("first");
		assert_eq!(Registry::load_from(&base).accounts[0].state, AccountState::Pending);
		std::fs::write(base.join("session.db"), b"x").unwrap();
		let registry = Registry::load_from(&base);
		assert_eq!(registry.active, FIRST_ACCOUNT);
		assert_eq!(registry.accounts.len(), 1);
		assert_eq!(registry.accounts[0].state, AccountState::Paired);
		assert_eq!(data_dir_in(&base, FIRST_ACCOUNT), base);
		let _ = std::fs::remove_dir_all(base);
	}

	#[test]
	fn ids_that_could_leave_the_folder_are_refused() {
		assert!(is_valid_id(FIRST_ACCOUNT));
		assert!(is_valid_id("0123456789abcdef0123456789abcdef"));
		assert!(!is_valid_id("../../windows"));
		assert!(!is_valid_id("0123456789ABCDEF0123456789ABCDEF"));
		assert!(!is_valid_id("0123456789abcdef0123456789abcde"));
	}

	#[test]
	fn a_damaged_file_is_kept_and_linked_accounts_are_found_again() {
		let base = temp_base("corrupt");
		std::fs::write(base.join("session.db"), b"x").unwrap();
		let id = "0123456789abcdef0123456789abcdef";
		std::fs::create_dir_all(base.join("accounts").join(id)).unwrap();
		std::fs::write(base.join("accounts").join(id).join("session.db"), b"x").unwrap();
		std::fs::write(base.join(FILE_NAME), b"{ not json").unwrap();
		let registry = Registry::load_from(&base);
		let ids: Vec<&str> = registry.accounts.iter().map(|a| a.id.as_str()).collect();
		assert_eq!(ids, [FIRST_ACCOUNT, id]);
		assert!(registry.accounts.iter().all(|a| a.state == AccountState::Paired));
		let kept = std::fs::read_dir(&base)
			.unwrap()
			.flatten()
			.any(|e| e.file_name().to_string_lossy().starts_with("accounts.json.corrupt-"));
		assert!(kept);
		let _ = std::fs::remove_dir_all(base);
	}

	#[test]
	fn add_save_and_load_round_trip() {
		let base = temp_base("roundtrip");
		let mut registry = Registry::load_from(&base);
		let id = registry.add_in(&base, " Work ").unwrap();
		assert!(is_valid_id(&id));
		assert!(data_dir_in(&base, &id).is_dir());
		registry.active = id.clone();
		registry.save_to(&base).unwrap();
		let loaded = Registry::load_from(&base);
		assert_eq!(loaded.active, id);
		assert_eq!(loaded.display_name(&id), "Work");
		let _ = std::fs::remove_dir_all(base);
	}

	#[test]
	fn a_deleting_active_account_hands_over_to_a_live_one() {
		let base = temp_base("tidy");
		let mut registry = Registry::load_from(&base);
		let id = registry.add_in(&base, "").unwrap();
		registry.get_mut(&id).unwrap().state = AccountState::Paired;
		registry.get_mut(FIRST_ACCOUNT).unwrap().state = AccountState::Deleting;
		registry.active = FIRST_ACCOUNT.to_string();
		registry.tidy();
		assert_eq!(registry.active, id);
		let _ = std::fs::remove_dir_all(base);
	}

	#[test]
	fn identity_fills_in_the_phone_and_pairs_the_account() {
		let mut registry = Registry::first_run(Path::new("/nonexistent"));
		assert!(registry.set_identity(FIRST_ACCOUNT, Some("919810012345:7@s.whatsapp.net"), Some("Aryan")));
		let account = registry.get(FIRST_ACCOUNT).unwrap();
		assert_eq!(account.phone, "919810012345");
		assert_eq!(account.state, AccountState::Paired);
		assert_eq!(account.display_name(), "Aryan");
		assert!(!registry.set_identity(FIRST_ACCOUNT, Some("919810012345:7@s.whatsapp.net"), Some("Aryan")));
	}

	#[test]
	fn display_name_prefers_the_label_then_the_name_then_the_number() {
		let mut account = Account { phone: "919810012345".into(), ..Default::default() };
		assert_eq!(account.display_name(), "+919810012345");
		account.name = "Aryan".into();
		assert_eq!(account.display_name(), "Aryan");
		account.label = "Work".into();
		assert_eq!(account.display_name(), "Work");
	}

	#[test]
	fn purging_the_first_account_keeps_the_app_settings() {
		let base = temp_base("purge");
		std::fs::write(base.join("session.db"), b"x").unwrap();
		std::fs::write(base.join("settings.json"), b"{}").unwrap();
		std::fs::create_dir_all(base.join("media_cache").join("media")).unwrap();
		assert!(purge_files_in(&base, FIRST_ACCOUNT));
		assert!(!base.join("session.db").exists());
		assert!(!base.join("media_cache").exists());
		assert!(base.join("settings.json").exists());
		let _ = std::fs::remove_dir_all(base);
	}
}
