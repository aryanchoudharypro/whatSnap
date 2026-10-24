package main

import (
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// maxLogBytes is where a log file gets rotated (to <name>.1, replacing the
// previous one) at startup. Nothing ever trimmed these, so they only grew.
const maxLogBytes = 10 << 20

// rotateLog moves path aside to path+".1" if it has grown past maxLogBytes.
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

// housekeepLogs rotates the backend's log files and prunes old dumps. Run
// once at startup, before any of them are opened.
func housekeepLogs() {
	logs := filepath.Join(appDataDir(), "logs")
	os.MkdirAll(logs, 0o700)
	for _, name := range []string{"backend.log", "call.log", "backend_stderr.log"} {
		rotateLog(filepath.Join(logs, name))
	}
	pruneOldFiles(filepath.Join(logs, "call-diag"), 7*24*time.Hour, nil)
	pruneOldFiles(logs, 7*24*time.Hour, func(n string) bool { return strings.HasPrefix(n, "stall-") })
}

// debugf logs only with -verbose-logging: per-event lines that are useful when
// troubleshooting but just noise (and log growth) the rest of the time.
func debugf(format string, args ...interface{}) {
	if *verboseLogFlag {
		log.Printf(format, args...)
	}
}

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
		appDataPath = filepath.Join(base, "whatSnap")
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
