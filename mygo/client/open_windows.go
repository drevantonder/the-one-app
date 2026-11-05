//go:build !cgoffi

package client

import "syscall"

// open loads the DLL with syscall; purego then calls into it (its Windows calls are syscall.SyscallN, with structs).
func open(path string) (uintptr, error) {
	h, err := syscall.LoadLibrary(path)
	return uintptr(h), err
}
