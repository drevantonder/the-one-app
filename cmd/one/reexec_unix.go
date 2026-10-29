//go:build !windows

package main

import (
	"os"
	"syscall"
)

// reexec runs the installed build in this process's place, so whatever
// supervises the node (launchd, systemd, the menu bar app) keeps it.
func reexec(exe string) error {
	return syscall.Exec(exe, os.Args, os.Environ())
}
