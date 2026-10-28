package spaces

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"slices"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// A Space can be backed up: its home folder, without caches, goes to the
// bucket when it stops and every few hours while it runs, encrypted, and
// the last few backups are kept. Creating a Space with the name of a
// backed-up one restores it, on any device, so one device at a time holds
// it: the log records which, and a Space is backed up while this device
// holds it.

const (
	backupEvery = 6 * time.Hour
	keepBackups = 3
)

// caches are left out of backups: they are most of a browser profile's size
// and rebuild themselves. So are a running browser's locks, which would make
// a restored profile look in use by another computer.
var caches = []string{"./.cache", "*/Cache", "*/Code Cache", "*/GPUCache", "*/DawnCache", "*/GrShaderCache", "*/ShaderCache", "*/Service Worker/CacheStorage", "*/Service Worker/ScriptCache", "*/SingletonLock", "*/SingletonSocket", "*/SingletonCookie"}

type hosted struct {
	Name string `json:"name"`
	Host string `json:"host"` // the device's tailnet name
}

type released struct {
	Name string `json:"name"`
}

// backups are a Space's stored backups, newest first.
func backups(call *sdk.Call, name string) ([]sdk.FileInfo, error) {
	files, err := call.Files(name + "/")
	if err != nil {
		return nil, err
	}
	slices.SortFunc(files, func(a, b sdk.FileInfo) int { return strings.Compare(b.Name, a.Name) })
	return files, nil
}

// backup stores the running Space's home folder and drops all but the
// newest backups.
func backup(call *sdk.Call, s space) (sdk.FileInfo, error) {
	tmp, err := os.CreateTemp("", "one-space-*.tar.gz")
	if err != nil {
		return sdk.FileInfo{}, err
	}
	defer os.Remove(tmp.Name())
	args := []string{"tar", "-C", home, "--warning=no-file-changed"}
	for _, c := range caches {
		args = append(args, "--exclude="+c)
	}
	err = rt.exec(call.Context, s.Name, nil, tmp, append(args, "-czf", "-", ".")...)
	tmp.Close()
	var exit *exec.ExitError
	if err != nil && !(errors.As(err, &exit) && exit.ExitCode() == 1) { // 1: a file changed while it was read
		return sdk.FileInfo{}, fmt.Errorf("packing %s: %w", s.Name, err)
	}
	name := fmt.Sprintf("%s/%s.tar.gz", s.Name, time.Now().UTC().Format("20060102T150405Z"))
	if err := call.PutFile(name, tmp.Name()); err != nil {
		return sdk.FileInfo{}, err
	}
	all, err := backups(call, s.Name)
	if err != nil {
		return sdk.FileInfo{}, err
	}
	for _, old := range all[min(keepBackups, len(all)):] {
		if err := call.DeleteFile(old.Name); err != nil {
			call.Logf("dropping old backup %s: %v", old.Name, err)
		}
	}
	for _, f := range all {
		if f.Name == name {
			return f, nil
		}
	}
	return sdk.FileInfo{Name: name}, nil
}

// noBucket reports whether err is the core saying there is no bucket to
// store files in, so no backups.
func noBucket(err error) bool {
	return strings.Contains(err.Error(), "needs a bucket") || strings.Contains(err.Error(), "no bucket configured")
}

// dropBackups deletes all of a Space's backups.
func dropBackups(call *sdk.Call, name string) error {
	all, err := backups(call, name)
	if err != nil {
		if noBucket(err) {
			return nil
		}
		return err
	}
	for _, f := range all {
		if err := call.DeleteFile(f.Name); err != nil {
			return err
		}
	}
	return nil
}

// restore unpacks a backup into a freshly made Space and restarts it.
func restore(call *sdk.Call, s space, f sdk.FileInfo) error {
	tmp, err := os.CreateTemp("", "one-space-*.tar.gz")
	if err != nil {
		return err
	}
	tmp.Close()
	defer os.Remove(tmp.Name())
	if err := call.GetFile(f.Name, tmp.Name()); err != nil {
		return err
	}
	in, err := os.Open(tmp.Name())
	if err != nil {
		return err
	}
	defer in.Close()
	if err := rt.exec(call.Context, s.Name, in, nil, "tar", "-C", home, "-xzf", "-"); err != nil {
		return fmt.Errorf("unpacking %s: %w", f.Name, err)
	}
	if err := rt.stop(call.Context, s.Name); err != nil {
		return err
	}
	if err := rt.start(call.Context, s.Name); err != nil {
		return err
	}
	return ready(call.Context, s)
}

// keepBackedUp backs up every running, backed-up Space whose newest backup
// is older than backupEvery. It runs for as long as the plugin does.
func keepBackedUp(call *sdk.Call, held func(string) bool) {
	for {
		spaces, err := list(call.Context)
		if err == nil {
			for _, s := range spaces {
				if s.State != "running" || !held(s.Name) {
					continue
				}
				all, err := backups(call, s.Name)
				if err != nil {
					call.Logf("backups of %s: %v", s.Name, err)
					continue
				}
				if len(all) > 0 && time.Since(all[0].Modified) < backupEvery {
					continue
				}
				if _, err := backup(call, s); err != nil {
					call.Logf("backing up %s: %v", s.Name, err)
				}
			}
		}
		select {
		case <-call.Context.Done():
			return
		case <-time.After(15 * time.Minute):
		}
	}
}

func size(n int64) string {
	switch {
	case n >= 1<<30:
		return fmt.Sprintf("%.1f GB", float64(n)/(1<<30))
	case n >= 1<<20:
		return fmt.Sprintf("%.0f MB", float64(n)/(1<<20))
	default:
		return fmt.Sprintf("%d KB", n>>10)
	}
}
