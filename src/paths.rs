//! Runtime write locations. Must stay in sync with `backend/paths.go`; the
//! frontend passes the resolved db path to the backend via `-db` so only one
//! side derives it.

use std::path::PathBuf;

pub fn app_data_dir() -> PathBuf {
	let base = std::env::var_os("LOCALAPPDATA")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")))
		.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

	let dir = base.join("whaNTV");
	let _ = std::fs::create_dir_all(&dir);
	dir
}

pub fn db_path() -> PathBuf {
	app_data_dir().join("session.db")
}

pub fn log_dir() -> PathBuf {
	let dir = app_data_dir().join("logs");
	let _ = std::fs::create_dir_all(&dir);
	dir
}

/// Combined size of the session database and its WAL sidecar file. In WAL
/// mode, writes land in `-wal` first and only fold back into the main file
/// at a checkpoint, so the main file alone can look static while the app is
/// actively writing - summing both is what actually tracks live write
/// activity.
pub fn db_and_wal_size() -> u64 {
	let db = db_path();
	let mut wal = db.clone().into_os_string();
	wal.push("-wal");
	std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0)
		+ std::fs::metadata(PathBuf::from(wal)).map(|m| m.len()).unwrap_or(0)
}

/// Where a frontend log file is rotated (to `<name>.1`) when it's opened for a
/// new session - matching the backend's own `rotateLog`.
const MAX_LOG_BYTES: u64 = 10 << 20;

/// Moves a log file aside once it has grown past `MAX_LOG_BYTES`, so the
/// append-only logs (a call's audio log gets a health line every 2 seconds)
/// can't grow without bound.
pub fn rotate_log(name: &str) {
	let path = log_dir().join(name);
	if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
		let rotated = log_dir().join(format!("{name}.1"));
		let _ = std::fs::remove_file(&rotated);
		let _ = std::fs::rename(&path, rotated);
	}
}
