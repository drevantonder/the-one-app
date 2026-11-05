//go:build !cgoffi

package client

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"github.com/ebitengine/purego"
)

// libName is the library as Cargo names it on this platform.
func libName() string {
	switch runtime.GOOS {
	case "darwin":
		return "libone_client_ffi.dylib"
	case "windows":
		return "one_client_ffi.dll"
	}
	return "libone_client_ffi.so"
}

// load opens the library with purego (dlopen, or LoadLibrary on Windows) and binds each function by its C signature.
// purego passes and returns uniffi's 24-byte RustBuffer by value itself on amd64 and arm64, on every desktop platform.
func load(l *lib, path string) error {
	if path == "" {
		path = os.Getenv("ONE_CLIENT_LIB")
	}
	if path == "" {
		for _, dir := range candidates() {
			p := filepath.Join(dir, libName())
			if _, err := os.Stat(p); err == nil {
				path = p
				break
			}
		}
	}
	if path == "" {
		return fmt.Errorf("%s isn't beside the program (set ONE_CLIENT_LIB)", libName())
	}
	h, err := open(path)
	if err != nil {
		return fmt.Errorf("opening %s: %w", path, err)
	}
	for name, fn := range l.symbols() {
		if err := bind(h, fn, name); err != nil {
			return err
		}
	}
	return nil
}

func bind(h uintptr, fn any, name string) (err error) {
	defer func() {
		if r := recover(); r != nil {
			err = fmt.Errorf("binding %s: %v", name, r)
		}
	}()
	purego.RegisterLibFunc(fn, h, name)
	return nil
}

// candidates are where the library may be: beside the program (and a macOS app's Frameworks), then up the tree to a
// checkout's target/debug.
func candidates() []string {
	var out []string
	if exe, err := os.Executable(); err == nil {
		dir := filepath.Dir(exe)
		out = append(out, dir, filepath.Join(dir, "..", "Frameworks"))
	}
	if wd, err := os.Getwd(); err == nil {
		for d := wd; ; d = filepath.Dir(d) {
			out = append(out, filepath.Join(d, "target", "debug"))
			if filepath.Dir(d) == d {
				break
			}
		}
	}
	return out
}
