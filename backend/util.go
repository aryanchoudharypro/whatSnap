package main

import (
	"encoding/base64"
	"strings"
)

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
