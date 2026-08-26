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
