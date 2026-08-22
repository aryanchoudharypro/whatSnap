package main

import (
	"encoding/base64"
	"path/filepath"
	"strings"
)

// boolToInt converts a bool to SQLite's 0/1 integer representation.
func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}

// decodeBase64Loose accepts either a bare base64 string or a data: URL
// (e.g. "data:image/png;base64,iVBORw0...") and returns the decoded bytes.
func decodeBase64Loose(s string) ([]byte, error) {
	if idx := strings.Index(s, ","); idx != -1 && strings.HasPrefix(s, "data:") {
		s = s[idx+1:]
	}
	if data, err := base64.StdEncoding.DecodeString(s); err == nil {
		return data, nil
	}
	return base64.RawStdEncoding.DecodeString(s)
}

// safeFilenameForFS reduces an untrusted filename (e.g. a document filename
// supplied by whoever sent us a message) to a single safe path component,
// so it can't escape the intended directory (via "..\" traversal) or break
// out of a quoted shell argument (via embedded quote characters) when we
// later join it into a local cache/temp path.
func safeFilenameForFS(name string) string {
	name = filepath.Base(strings.ReplaceAll(name, "\x00", ""))
	name = sanitizeFilenameComponent(name)
	name = strings.ReplaceAll(name, "'", "_")
	if name == "" || name == "." || name == ".." {
		name = "file"
	}
	return name
}

// sanitizeFilename removes mangled MIME extensions (e.g. .x-python) and replaces them with standard ones
func sanitizeFilename(filename string) string {
	filename = strings.ReplaceAll(filename, ".x-python", ".py")
	filename = strings.ReplaceAll(filename, ".x-script.python", ".py")
	filename = strings.ReplaceAll(filename, ".x-shellscript", ".sh")
	filename = strings.ReplaceAll(filename, ".x-csrc", ".c")
	filename = strings.ReplaceAll(filename, ".x-c++src", ".cpp")
	filename = strings.ReplaceAll(filename, ".x-perl", ".pl")
	filename = strings.ReplaceAll(filename, ".x-ruby", ".rb")
	filename = strings.ReplaceAll(filename, ".x-php", ".php")
	return filename
}
