package main

import (
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

const maxLogBytes = 10 << 20

func rotateLog(path string) {
	if info, err := os.Stat(path); err == nil && info.Size() > maxLogBytes {
		os.Remove(path + ".1")
		os.Rename(path, path+".1")
	}
}

// pruneOldFiles deletes files in dir older than maxAge (per-call diagnostic
// dumps, stall dumps) so they don't pile up forever.
func pruneOldFiles(dir string, maxAge time.Duration, match func(name string) bool) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-maxAge)
	for _, e := range entries {
		if match != nil && !match(e.Name()) {
			continue
		}
		if info, err := e.Info(); err == nil && info.ModTime().Before(cutoff) {
			os.RemoveAll(filepath.Join(dir, e.Name()))
		}
	}
}

func housekeepLogs() {
	logs := logsDir()
	for _, name := range []string{"backend.log", "call.log", "backend_stderr.log"} {
		rotateLog(filepath.Join(logs, name))
	}
	pruneOldFiles(filepath.Join(logs, "call-diag"), 7*24*time.Hour, nil)
	pruneOldFiles(logs, 7*24*time.Hour, func(n string) bool { return strings.HasPrefix(n, "stall-") })
}

func debugf(format string, args ...interface{}) {
	if *verboseLogFlag {
		log.Printf(format, args...)
	}
}

// Must stay in sync with src/platform/paths.rs. On Windows os.UserCacheDir is %LOCALAPPDATA%.
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
		appDataPath = filepath.Join(base, "whatSnap")
		os.MkdirAll(appDataPath, 0o700)
	})
	return appDataPath
}

// logsDir holds the backend's logs, shared by every account.
func logsDir() string {
	dir := filepath.Join(appDataDir(), "logs")
	os.MkdirAll(dir, 0o700)
	return dir
}

func logFilePath() string {
	return filepath.Join(logsDir(), "backend.log")
}
