package main

import (
	"os"
	"path/filepath"
	"sync"
)

// Must stay in sync with src/paths.rs. On Windows os.UserCacheDir is %LOCALAPPDATA%.
var (
	appDataOnce sync.Once
	appDataPath string
)

func appDataDir() string {
	appDataOnce.Do(func() {
		base, err := os.UserCacheDir()
		if err != nil || base == "" {
			base = "."
		}
		appDataPath = filepath.Join(base, "whaNTV")
		os.MkdirAll(appDataPath, 0o700)
	})
	return appDataPath
}

func defaultDBPath() string {
	return filepath.Join(appDataDir(), "session.db")
}

// Deliberately not derived from -db, so logs stay put when a caller points the
// database elsewhere for testing.
func logFilePath() string {
	dir := filepath.Join(appDataDir(), "logs")
	os.MkdirAll(dir, 0o700)
	return filepath.Join(dir, "backend.log")
}
