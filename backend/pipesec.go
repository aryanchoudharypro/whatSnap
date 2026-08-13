package main

import (
	"fmt"

	"golang.org/x/sys/windows"
)

// Protected DACL granting full access to the current user and nobody else.
func currentUserPipeSDDL() (string, error) {
	user, err := windows.GetCurrentProcessToken().GetTokenUser()
	if err != nil {
		return "", fmt.Errorf("resolving current user SID: %w", err)
	}

	sid := user.User.Sid.String()
	if sid == "" {
		return "", fmt.Errorf("current user SID is empty")
	}

	return fmt.Sprintf("D:P(A;;GA;;;%s)", sid), nil
}
