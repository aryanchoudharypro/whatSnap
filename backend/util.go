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
