package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
)

var (
	commitRE  = regexp.MustCompile(`^[0-9a-f]{40}$`)
	versionRE = regexp.MustCompile(`^[0-9A-Za-z][0-9A-Za-z.+-]*$`)
)

// errRestart ends serve so main can run the new build in its place.
var errRestart = errors.New("restart")

// nodeBuild is what a node runs, so `one update` knows what to send it.
type nodeBuild struct {
	OS      string `json:"os"`
	Arch    string `json:"arch"`
	Version string `json:"version"`
	App     bool   `json:"app"` // runs from TheOneApp.app, so takes the signed app, not a bare binary
}

// self is the running executable, found before anything replaces it.
func self() string {
	p, _ := os.Executable()
	if r, err := filepath.EvalSymlinks(p); err == nil {
		p = r
	}
	return p
}

// appBundle is the .app this binary runs from on a Mac, if any.
func appBundle(exe string) string {
	if runtime.GOOS != "darwin" {
		return ""
	}
	if i := strings.Index(exe, ".app/Contents/MacOS/"); i >= 0 {
		return exe[:i+len(".app")]
	}
	return ""
}

// updateRoutes lets the owner's devices replace this node's build: PUT a
// binary (or, for an app, a zip of TheOneApp.app) with X-One-Version set,
// and the node installs it and restarts. A Mac's node also builds the app.
func updateRoutes(mux *http.ServeMux, exe string, restart func()) {
	mux.HandleFunc("GET /v1/update", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, nodeBuild{OS: runtime.GOOS, Arch: runtime.GOARCH, Version: version, App: appBundle(exe) != ""})
	})
	var building sync.Mutex
	mux.HandleFunc("POST /v1/update/app", func(w http.ResponseWriter, r *http.Request) {
		if runtime.GOOS != "darwin" {
			http.Error(w, "only a Mac builds TheOneApp.app", http.StatusNotFound)
			return
		}
		var in appRequest
		if err := json.NewDecoder(r.Body).Decode(&in); err != nil || !commitRE.MatchString(in.Commit) || !versionRE.MatchString(in.Version) {
			http.Error(w, "want a full commit hash and a version", http.StatusBadRequest)
			return
		}
		building.Lock()
		defer building.Unlock()
		f, err := os.CreateTemp("", "one-app-*.zip")
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		defer os.Remove(f.Name())
		defer f.Close()
		if err := buildAppHere(r.Context(), in.Commit, in.Version, in.Notarize, f); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		_, _ = f.Seek(0, io.SeekStart)
		w.Header().Set("Content-Type", "application/zip")
		_, _ = io.Copy(w, f)
	})
	mux.HandleFunc("PUT /v1/update", func(w http.ResponseWriter, r *http.Request) {
		want := r.Header.Get("X-One-Version")
		if want == "" {
			http.Error(w, "X-One-Version is required", http.StatusBadRequest)
			return
		}
		if err := install(exe, want, http.MaxBytesReader(w, r.Body, 1<<30)); err != nil {
			http.Error(w, err.Error(), http.StatusUnprocessableEntity)
			return
		}
		writeJSON(w, map[string]string{"installed": want})
		go restart()
	})
}

func install(exe, want string, body io.Reader) error {
	dir := filepath.Dir(exe)
	if app := appBundle(exe); app != "" {
		dir = filepath.Dir(app)
	}
	tmp, err := os.MkdirTemp(dir, ".one-update-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmp)
	f, err := os.Create(filepath.Join(tmp, "upload"))
	if err != nil {
		return err
	}
	_, err = io.Copy(f, body)
	f.Close()
	if err != nil {
		return err
	}

	from, to, newExe := filepath.Join(tmp, "upload"), exe, filepath.Join(tmp, "upload")
	if app := appBundle(exe); app != "" {
		if err := unzip(from, tmp); err != nil {
			return err
		}
		from, to = filepath.Join(tmp, filepath.Base(app)), app
		newExe = filepath.Join(from, strings.TrimPrefix(exe, app))
	} else if err := os.Chmod(from, 0o755); err != nil {
		return err
	}
	out, err := exec.Command(newExe, "version").Output()
	if got := strings.TrimSpace(string(out)); err != nil || got != want {
		return fmt.Errorf("the upload doesn't run here as %s (got %q, %v)", want, got, err)
	}
	if appBundle(exe) != "" {
		if out, err := exec.Command("codesign", "--verify", "--strict", from).CombinedOutput(); err != nil {
			return fmt.Errorf("the app's signature is invalid: %s", out)
		}
	}
	// A running program can't be overwritten everywhere, but it can be moved.
	old := to + ".old"
	_ = os.RemoveAll(old)
	if err := os.Rename(to, old); err != nil {
		return err
	}
	if err := os.Rename(from, to); err != nil {
		_ = os.Rename(old, to)
		return err
	}
	if runtime.GOOS != "windows" {
		_ = os.RemoveAll(old)
	}
	return nil
}

// unzip extracts a zip of an app bundle with ditto, which keeps the
// symlinks and extended attributes its signature covers.
func unzip(path, dest string) error {
	if out, err := exec.Command("ditto", "-x", "-k", path, dest).CombinedOutput(); err != nil {
		return fmt.Errorf("unzip: %s", out)
	}
	return nil
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}
