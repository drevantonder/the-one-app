//go:build !cgoffi && !windows

package client

import "github.com/ebitengine/purego"

func open(path string) (uintptr, error) {
	return purego.Dlopen(path, purego.RTLD_NOW|purego.RTLD_GLOBAL)
}
