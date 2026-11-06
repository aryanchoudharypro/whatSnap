use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::platform::paths;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountEntry {
	pub id: String,
	pub name: String,
	pub phone: String,
	pub push_name: String,
	pub jid: String,
	pub created_at: u64,
	pub last_active_at: u64,
}

impl AccountEntry {
	pub fn display_label(&self) -> String {
		let mut title = if !self.name.is_empty() && !self.push_name.is_empty() && self.name != self.push_name {
			format!("{} ({})", self.name, self.push_name)
		} else if !self.name.is_empty() {
			self.name.clone()
		} else if !self.push_name.is_empty() {
			self.push_name.clone()
		} else {
			format!("Account {}", self.id)
		};

		if !self.phone.is_empty() {
			title = format!("{} • {}", title, self.phone);
		}
		title
	}
}

fn current_unix_time() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountsManager {
	pub active_id: String,
	pub accounts: Vec<AccountEntry>,
}

impl Default for AccountsManager {
	fn default() -> Self {
		let now = current_unix_time();
		Self {
			active_id: "default".to_string(),
			accounts: vec![AccountEntry {
				id: "default".to_string(),
				name: "Personal (Primary)".to_string(),
				phone: String::new(),
				push_name: String::new(),
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
			jid: String::new(),
			created_at: now,
			last_active_at: now,
		};

		let _ = paths::account_dir(&id);
		self.accounts.push(entry.clone());
		self.save();
		entry
	}

	pub fn switch_to(&mut self, id: &str) -> bool {
		if self.accounts.iter().any(|a| a.id == id) {
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

	pub fn update_active_identity(&mut self, jid: Option<&str>, push_name: Option<&str>, phone: Option<&str>) -> bool {
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
			jid: "1234567890@s.whatsapp.net".into(),
			created_at: 100,
			last_active_at: 100,
		};
		assert_eq!(acc.display_label(), "Work (Aryan) • +1234567890");

		acc.push_name = "Work".into();
		assert_eq!(acc.display_label(), "Work • +1234567890");

		acc.phone = "".into();
		assert_eq!(acc.display_label(), "Work");
	}

	#[test]
	fn test_accounts_manager_operations() {
		let mut mgr = AccountsManager::default();
		assert_eq!(mgr.accounts.len(), 1);
		assert_eq!(mgr.active_id, "default");

		let added = mgr.add_account(Some("Work Phone"));
		assert_eq!(mgr.accounts.len(), 2);
		assert_eq!(added.name, "Work Phone");

		assert!(mgr.switch_to(&added.id));
		assert_eq!(mgr.active_id, added.id);

		assert!(mgr.rename_account(&added.id, "Office WhatsApp"));
		assert_eq!(mgr.active_account().unwrap().name, "Office WhatsApp");

		let remove_res = mgr.remove_account(&added.id);
		assert!(remove_res.is_ok());
		assert_eq!(mgr.accounts.len(), 1);
		assert_eq!(mgr.active_id, "default");

		// Cannot remove only remaining account
		assert!(mgr.remove_account("default").is_err());
	}
}
