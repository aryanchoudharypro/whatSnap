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
