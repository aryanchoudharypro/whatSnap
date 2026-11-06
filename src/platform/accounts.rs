use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::platform::paths;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountEntry {
	pub id: String,
	pub name: String,
	#[serde(default)]
	pub phone: String,
	#[serde(default)]
	pub push_name: String,
	#[serde(default)]
	pub about: String,
	#[serde(default)]
	pub jid: String,
	pub created_at: u64,
	pub last_active_at: u64,
}

impl AccountEntry {
	pub fn display_label(&self) -> String {
		// Priority 1: If WhatsApp profile name is known
		if !self.push_name.is_empty() {
			let is_default_name =
				self.name.is_empty() || self.name == "Personal (Primary)" || self.name.starts_with("Account ");
			if !is_default_name && self.name != self.push_name {
				format!("{} ({})", self.name, self.push_name)
			} else {
				self.push_name.clone()
			}
		} else if !self.name.is_empty() {
			self.name.clone()
		} else if !self.about.is_empty() {
			self.about.clone()
		} else {
			format!("Account {}", self.id)
		}
	}
}

fn current_unix_time() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountsManager {
	pub active_id: String,
	#[serde(default)]
	pub previous_active_id: Option<String>,
	pub accounts: Vec<AccountEntry>,
}

impl Default for AccountsManager {
	fn default() -> Self {
		let now = current_unix_time();
		Self {
			active_id: "default".to_string(),
			previous_active_id: None,
			accounts: vec![AccountEntry {
				id: "default".to_string(),
				name: "Personal (Primary)".to_string(),
				phone: String::new(),
				push_name: String::new(),
				about: String::new(),
				jid: String::new(),
				created_at: now,
				last_active_at: now,
			}],
		}
	}
}

impl AccountsManager {
	pub fn load() -> Self {
		let path = paths::accounts_json_path();
		if path.exists() {
			if let Ok(content) = std::fs::read_to_string(&path) {
				if let Ok(mut manager) = serde_json::from_str::<AccountsManager>(&content) {
					if manager.accounts.is_empty() {
						manager = AccountsManager::default();
						manager.save();
					} else if !manager.accounts.iter().any(|a| a.id == manager.active_id) {
						manager.active_id = manager.accounts[0].id.clone();
						manager.save();
					}
					return manager;
				}
			}
		}

		let manager = AccountsManager::default();
		manager.save();
		manager
	}

	pub fn save(&self) {
		let path = paths::accounts_json_path();
		if let Ok(data) = serde_json::to_string_pretty(self) {
			let _ = std::fs::write(path, data);
		}
	}

	pub fn active_account(&self) -> Option<&AccountEntry> {
		self.accounts.iter().find(|a| a.id == self.active_id)
	}

	pub fn active_account_mut(&mut self) -> Option<&mut AccountEntry> {
		let target = self.active_id.clone();
		self.accounts.iter_mut().find(|a| a.id == target)
	}

	pub fn db_path_for(&self, id: &str) -> PathBuf {
		if id == "default" { paths::default_db_path() } else { paths::account_dir(id).join("session.db") }
	}

	pub fn active_db_path(&self) -> PathBuf {
		self.db_path_for(&self.active_id)
	}

	pub fn add_account(&mut self, custom_name: Option<&str>) -> AccountEntry {
		let hex = crate::util::random_hex();
		let id = format!("account_{}", &hex[..8]);
		let now = current_unix_time();
		let count = self.accounts.len() + 1;
		let name = match custom_name {
			Some(n) if !n.trim().is_empty() => n.trim().to_string(),
			_ => format!("Account {}", count),
		};

		let entry = AccountEntry {
			id: id.clone(),
			name,
			phone: String::new(),
			push_name: String::new(),
			about: String::new(),
			jid: String::new(),
			created_at: now,
			last_active_at: now,
		};

		let _ = paths::account_dir(&id);
		self.previous_active_id = Some(self.active_id.clone());
		self.accounts.push(entry.clone());
		self.save();
		entry
	}

	pub fn switch_to(&mut self, id: &str) -> bool {
		if self.accounts.iter().any(|a| a.id == id) {
			if self.active_id != id {
				self.previous_active_id = Some(self.active_id.clone());
			}
			self.active_id = id.to_string();
			let now = current_unix_time();
			if let Some(acc) = self.active_account_mut() {
				acc.last_active_at = now;
			}
			self.save();
			true
		} else {
			false
		}
	}

	pub fn cancel_pending_account(&mut self, id: &str) -> Option<String> {
		let is_unlinked = self.accounts.iter().find(|a| a.id == id).map(|a| a.jid.is_empty()).unwrap_or(false);
		if is_unlinked && self.accounts.len() > 1 {
			if let Some(idx) = self.accounts.iter().position(|a| a.id == id) {
				self.accounts.remove(idx);
				if id != "default" {
					let dir = paths::account_dir(id);
					let _ = std::fs::remove_dir_all(&dir);
				}
			}
		}

		let target_id = if let Some(prev) = &self.previous_active_id {
			if self.accounts.iter().any(|a| &a.id == prev) {
				prev.clone()
			} else {
				self.accounts.first().map(|a| a.id.clone()).unwrap_or_else(|| "default".to_string())
			}
		} else if let Some(first_other) = self.accounts.iter().find(|a| a.id != id) {
			first_other.id.clone()
		} else {
			self.accounts.first().map(|a| a.id.clone()).unwrap_or_else(|| "default".to_string())
		};

		self.active_id = target_id.clone();
		self.previous_active_id = None;
		self.save();
		Some(target_id)
	}

	pub fn previous_account(&self) -> Option<&AccountEntry> {
		if let Some(prev) = &self.previous_active_id { self.accounts.iter().find(|a| &a.id == prev) } else { None }
	}

	pub fn rename_account(&mut self, id: &str, new_name: &str) -> bool {
		if let Some(acc) = self.accounts.iter_mut().find(|a| a.id == id) {
			acc.name = new_name.trim().to_string();
			self.save();
			true
		} else {
			false
		}
	}

	pub fn remove_account(&mut self, id: &str) -> Result<String, String> {
		if self.accounts.len() <= 1 {
			return Err("Cannot remove the only configured account.".to_string());
		}

		let idx = self.accounts.iter().position(|a| a.id == id).ok_or_else(|| "Account not found.".to_string())?;

		self.accounts.remove(idx);

		if self.active_id == id {
			self.active_id = self.accounts[0].id.clone();
		}

		if id != "default" {
			let dir = paths::account_dir(id);
			let _ = std::fs::remove_dir_all(&dir);
		}

		self.save();
		Ok(self.active_id.clone())
	}

	pub fn update_active_identity(
		&mut self,
		jid: Option<&str>,
		push_name: Option<&str>,
		about: Option<&str>,
		phone: Option<&str>,
	) -> bool {
		let mut changed = false;
		if let Some(acc) = self.active_account_mut() {
			if let Some(j) = jid {
				if !j.is_empty() && acc.jid != j {
					acc.jid = j.to_string();
					changed = true;
				}
			}
			if let Some(p) = push_name {
				if !p.is_empty() && acc.push_name != p {
					acc.push_name = p.to_string();
					changed = true;
				}
			}
			if let Some(a) = about {
				if !a.is_empty() && acc.about != a {
					acc.about = a.to_string();
					changed = true;
				}
			}
			if let Some(ph) = phone {
				if !ph.is_empty() && acc.phone != ph {
					acc.phone = ph.to_string();
					changed = true;
				}
			}
		}
		if changed {
			self.save();
		}
		changed
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_account_entry_display_label() {
		let mut acc = AccountEntry {
			id: "acc1".into(),
			name: "Work".into(),
			phone: "+1234567890".into(),
			push_name: "Aryan".into(),
			about: "Available".into(),
			jid: "1234567890@s.whatsapp.net".into(),
			created_at: 100,
			last_active_at: 100,
		};
		// Custom name with distinct WhatsApp push name
		assert_eq!(acc.display_label(), "Work (Aryan)");

		// Default name gets replaced directly by WhatsApp profile name
		acc.name = "Personal (Primary)".into();
		assert_eq!(acc.display_label(), "Aryan");

		acc.name = "Account 1".into();
		assert_eq!(acc.display_label(), "Aryan");

		// Identical custom name and push name
		acc.name = "Work".into();
		acc.push_name = "Work".into();
		assert_eq!(acc.display_label(), "Work");

		// When push_name is empty, fallback to custom name
		acc.push_name = "".into();
		assert_eq!(acc.display_label(), "Work");

		// Never contains any raw phone digits
		assert!(!acc.display_label().contains("+1234567890"));
	}

	#[test]
	fn test_accounts_manager_operations() {
		let mut mgr = AccountsManager::default();
		assert_eq!(mgr.accounts.len(), 1);
		assert_eq!(mgr.active_id, "default");

		let added = mgr.add_account(Some("Work Profile"));
		assert_eq!(mgr.accounts.len(), 2);
		assert_eq!(added.name, "Work Profile");
		assert_eq!(mgr.previous_active_id, Some("default".to_string()));

		assert!(mgr.switch_to(&added.id));
		assert_eq!(mgr.active_id, added.id);
		assert_eq!(mgr.previous_active_id, Some("default".to_string()));

		assert!(mgr.rename_account(&added.id, "Office WhatsApp"));
		assert_eq!(mgr.active_account().unwrap().name, "Office WhatsApp");

		// Cancel pending unlinked account returns to previous account
		let target = mgr.cancel_pending_account(&added.id);
		assert_eq!(target, Some("default".to_string()));
		assert_eq!(mgr.active_id, "default");
		assert_eq!(mgr.accounts.len(), 1);

		// Cannot remove only remaining account
		assert!(mgr.remove_account("default").is_err());
	}
}
