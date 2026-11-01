package main

import (
	"encoding/base64"
	"path/filepath"
	"strings"
)

func boolToInt(b bool) int {
	if b {
		return 1
	}
	return 0
}

func decodeBase64Loose(s string) ([]byte, error) {
	if idx := strings.Index(s, ","); idx != -1 && strings.HasPrefix(s, "data:") {
		s = s[idx+1:]
	}
	if data, err := base64.StdEncoding.DecodeString(s); err == nil {
		return data, nil
	}
	return base64.RawStdEncoding.DecodeString(s)
}

func safeFilenameForFS(name string) string {
	name = filepath.Base(strings.ReplaceAll(name, "\x00", ""))
	name = sanitizeFilenameComponent(name)
	name = strings.ReplaceAll(name, "'", "_")
	if name == "" || name == "." || name == ".." {
		name = "file"
	}
	return name
}

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
