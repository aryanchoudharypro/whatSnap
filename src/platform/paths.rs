use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
	let dir = local_app_data_base().join("whatSnap");
	let _ = std::fs::create_dir_all(&dir);
	dir
}

fn local_app_data_base() -> PathBuf {
	std::env::var_os("LOCALAPPDATA")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")))
		.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

const LEGACY_FILES: [&str; 4] = ["session.db", "session.db-wal", "session.db-shm", "settings.json"];
const LEGACY_MEDIA_DIR: &str = "media_cache";

/// On the first run after the rename, copies the session database (with its WAL
/// and shared-memory files), the settings and the media cache into the new
/// folder. The old `whaNTV` folder is left exactly as it was, as a fallback.
/// Does nothing once the new folder has a session, or if there is no old session
/// to copy. Files already in the new folder are never overwritten.
///
/// Must run before anything opens the database.
pub fn migrate_legacy_data() {
	let new_dir = app_data_dir();
	if new_dir.join("session.db").exists() {
		return;
	}
	let legacy_dir = local_app_data_base().join("whaNTV");
	if !legacy_dir.join("session.db").exists() {
		return;
	}
	let mut complete = true;
	for name in LEGACY_FILES {
		complete &= copy_if_missing(&legacy_dir.join(name), &new_dir.join(name));
	}
	complete &= copy_tree_if_missing(&legacy_dir.join(LEGACY_MEDIA_DIR), &new_dir.join(LEGACY_MEDIA_DIR));
	// Only a complete copy is marked, so the old folder is never removed for a partial one.
	if complete {
		let _ = std::fs::write(new_dir.join(MIGRATED_MARKER), legacy_dir.display().to_string());
	}
}

const MIGRATED_MARKER: &str = ".migrated-from-whaNTV";

fn copy_if_missing(from: &std::path::Path, to: &std::path::Path) -> bool {
	if !from.is_file() || to.exists() {
		return true;
	}
	match std::fs::copy(from, to) {
		Ok(_) => true,
		Err(err) => {
			eprintln!("[MIGRATE] couldn't copy {} to the new data folder: {err}", from.display());
			false
		}
	}
}

fn copy_tree_if_missing(from: &std::path::Path, to: &std::path::Path) -> bool {
	let Ok(entries) = std::fs::read_dir(from) else { return true };
	if std::fs::create_dir_all(to).is_err() {
		return false;
	}
	let mut complete = true;
	for entry in entries.flatten() {
		let source = entry.path();
		let target = to.join(entry.file_name());
		complete &=
			if source.is_dir() { copy_tree_if_missing(&source, &target) } else { copy_if_missing(&source, &target) };
	}
	complete
}

pub fn remove_legacy_data_if_running(running: bool) {
	use std::sync::atomic::{AtomicBool, Ordering};
	static DONE: AtomicBool = AtomicBool::new(false);
	if !running || DONE.load(Ordering::Relaxed) {
		return;
	}
	let marker = app_data_dir().join(MIGRATED_MARKER);
	if !marker.exists() {
		DONE.store(true, Ordering::Relaxed);
		return;
	}
	static LAST_TRY: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
	{
		let mut last = LAST_TRY.lock().unwrap_or_else(|e| e.into_inner());
		if last.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(30)) {
			return;
		}
		*last = Some(std::time::Instant::now());
	}
	let legacy_dir = local_app_data_base().join("whaNTV");
	if !legacy_dir.exists() {
		let _ = std::fs::remove_file(&marker);
		DONE.store(true, Ordering::Relaxed);
		return;
	}
	match std::fs::remove_dir_all(&legacy_dir) {
		Ok(()) => {
			let _ = std::fs::remove_file(&marker);
			DONE.store(true, Ordering::Relaxed);
			eprintln!("[MIGRATE] removed the old data folder {}", legacy_dir.display());
		}
		Err(err) => eprintln!("[MIGRATE] couldn't remove {}: {err}", legacy_dir.display()),
	}
}

pub fn accounts_dir() -> PathBuf {
	let dir = app_data_dir().join("accounts");
	let _ = std::fs::create_dir_all(&dir);
	dir
}

pub fn account_dir(id: &str) -> PathBuf {
	let dir = accounts_dir().join(id);
	let _ = std::fs::create_dir_all(&dir);
	dir
}

pub fn accounts_json_path() -> PathBuf {
	app_data_dir().join("accounts.json")
}

pub fn default_db_path() -> PathBuf {
	app_data_dir().join("session.db")
}

pub fn db_path() -> PathBuf {
	default_db_path()
}

pub fn log_dir() -> PathBuf {
	let dir = app_data_dir().join("logs");
	let _ = std::fs::create_dir_all(&dir);
	dir
}

pub fn db_and_wal_size() -> u64 {
	let db = db_path();
	let mut wal = db.clone().into_os_string();
	wal.push("-wal");
	std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0)
		+ std::fs::metadata(PathBuf::from(wal)).map(|m| m.len()).unwrap_or(0)
}

const MAX_LOG_BYTES: u64 = 10 << 20;

pub fn rotate_log(name: &str) {
	let path = log_dir().join(name);
	if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
		let rotated = log_dir().join(format!("{name}.1"));
		let _ = std::fs::remove_file(&rotated);
		let _ = std::fs::rename(&path, rotated);
	}
}
