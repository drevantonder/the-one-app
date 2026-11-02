package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"time"
)

// Borrowed devices: an agent borrows a device of its own for as long as it
// needs a browser, and gives it back, which deletes it. They come from a pool:
// one spare is kept running, so borrowing is quick. A borrowed device's
// browser starts with a copy of a browser profile (profiles.go), which the
// person signed in to sites in once (sign_in); the copy is thrown away with
// the device, so only sign_in and save_profile change a profile.

const (
	poolPrefix   = "pool-"
	idleBorrowed = 15 * time.Minute // a borrowed device left this long is given back
	signInFor    = 30 * time.Minute // a sign-in is saved and closed after this, by default
	profiles     = home + "/.local/state/cua-driver/browser-profiles/"
	// signInClass marks the browser sign_in opens, which the tools didn't, so
	// closing the tools' leftover browsers leaves it open.
	signInClass = "--class=one-sign-in"
)

// A loan is a borrowed device's: who has it and what for, and the browser
// profile it started with. Its profile's key is kept only in memory, in held;
// after a restart, the app gives it again when the profile is saved.
type loan struct {
	By        string    `json:"by"`
	For       string    `json:"for"`
	Profile   string    `json:"profile,omitempty"` // the profile's name
	Item      string    `json:"item,omitempty"`    // its Vault item
	Since     time.Time `json:"since"`
	SigningIn bool      `json:"signing_in,omitempty"`
	Until     time.Time `json:"until,omitzero"`      // a sign-in's end
	NoScreen  bool      `json:"no_screen,omitempty"` // made without a screen: nobody watches it
}

// caches are left out of saved profiles and backups, each a path in a Chrome
// folder (* is one of its profiles, like Default): they are most of a profile's
// size and rebuild themselves. So are a running browser's locks, which would
// make a copy look in use by another device. Service Worker/ScriptCache isn't
// one: it's the only copy of an extension's background script, and without it
// the extension (1Password) never answers.
var caches = []string{"*/Cache", "*/Code Cache", "*/GPUCache", "*/DawnGraphiteCache", "*/DawnWebGPUCache", "*/Service Worker/CacheStorage",
	"GPUPersistentCache", "GrShaderCache", "ShaderCache", "SingletonLock", "SingletonSocket", "SingletonCookie"}

// excludes are tar's arguments that leave out the caches of the Chrome folders
// at (a path from where tar starts, where * is any one folder) and other: each
// only there, from the start of a path, with * within one folder, never a
// folder of the same name elsewhere, like one in an extension or a person's.
func excludes(at string, other ...string) []string {
	args := []string{"--anchored", "--no-wildcards-match-slash"}
	for _, c := range caches {
		args = append(args, "--exclude="+at+"/"+c)
	}
	for _, o := range other {
		args = append(args, "--exclude="+o)
	}
	return args
}

func (ds *devices) loans() map[string]loan {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	return maps.Clone(ds.lent)
}

// setLoan records or (with nil) forgets a loan, on disk too, so they outlive
// this program.
func (ds *devices) setLoan(name string, l *loan) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if l == nil {
		delete(ds.lent, name)
		delete(ds.held, name)
	} else {
		ds.lent[name] = *l
	}
	b, _ := json.MarshalIndent(ds.lent, "", "  ")
	if err := os.WriteFile(filepath.Join(ds.dir, "lent.json"), b, 0o600); err != nil {
		log.Print(err)
	}
}

func (ds *devices) loadLoans() {
	b, err := os.ReadFile(filepath.Join(ds.dir, "lent.json"))
	if err == nil {
		json.Unmarshal(b, &ds.lent)
	}
	if ds.lent == nil {
		ds.lent = map[string]loan{}
	}
}

// touch notes a call to a device, which keeps it from being given back or
// paused while idle.
func (ds *devices) touch(name string) {
	ds.poolMu.Lock()
	ds.used[name] = time.Now()
	ds.poolMu.Unlock()
}

// idle is how long since the device's last call, or since this program
// first saw it.
func (ds *devices) idle(name string) time.Duration {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	t, ok := ds.used[name]
	if !ok {
		ds.used[name] = time.Now()
		return 0
	}
	return time.Since(t)
}

func (ds *devices) poke() {
	select {
	case ds.kick <- struct{}{}:
	default:
	}
}

// spares are the pool's devices nobody has borrowed.
func spares(all []device, lent map[string]loan) []device {
	var out []device
	for _, d := range all {
		if _, ok := lent[d.Name]; d.pool && !ok && d.State != "stopped" {
			out = append(out, d)
		}
	}
	return out
}

// borrow lends the spare device, or one made for it, its browser starting
// with the profile p, if one is given.
func (ds *devices) borrow(ctx context.Context, l loan, p *profile) (device, error) {
	if p != nil {
		l.Profile, l.Item = p.Name, p.Item
	}
	ds.take.Lock()
	all, err := list(ctx)
	if err != nil {
		ds.take.Unlock()
		return device{}, err
	}
	var d device
	if free := spares(all, ds.loans()); len(free) > 0 {
		d = free[0]
	} else if !roomForOne() {
		ds.take.Unlock()
		return device{}, errors.New("this server has no memory to spare for another device: give back one you're done with")
	} else if d, err = create(ctx, newPoolName(), sizes[0], true, false); err != nil {
		ds.take.Unlock()
		return device{}, err
	}
	l.Since = time.Now().UTC()
	ds.setLoan(d.Name, &l)
	ds.hold(d.Name, p)
	ds.touch(d.Name)
	ds.take.Unlock()
	log.Printf("%s borrowed %s for %q, profile %q", l.By, d.Name, l.For, l.Profile)
	ds.poke() // for another spare
	if d.State == "paused" {
		if err := unpause(ctx, d); err != nil {
			return d, err
		}
	}
	if err := ready(ctx, d); err != nil {
		return d, err
	}
	if p != nil {
		if err := ds.unpack(ctx, d, p); err != nil {
			ds.giveBack(context.WithoutCancel(ctx), d.Name, "couldn't prepare it")
			return d, err
		}
	}
	return d, nil
}

// hold keeps a borrowed device's profile, with its key, while it's lent.
func (ds *devices) hold(name string, p *profile) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if p == nil {
		delete(ds.held, name)
	} else {
		ds.held[name] = p
	}
}

func (ds *devices) heldProfile(name string) *profile {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	return ds.held[name]
}

func newPoolName() string {
	b := make([]byte, 3)
	rand.Read(b)
	return poolPrefix + hex.EncodeToString(b)
}

// owned makes the profile one the browser tools will open: they only open
// profiles they made, as a file in it says.
func owned(name string) string {
	dir := profiles + name
	return fmt.Sprintf(`mkdir -p %s && printf '%%s' '{"schema":"cua-driver-browser-profile-v1","mode":"isolated_named","name":%q}' > %s/.cua-driver-owned-profile.json && chmod 700 %s && chown -R cua:cua %s/.local`,
		dir, name, dir, dir, home)
}

// openPage opens a page in a browser on the profile, the person's to use
// through view.
func openPage(ctx context.Context, d device, name, url string) error {
	if url == "" {
		url = "about:blank"
	}
	return sh(ctx, d, owned(name)+` && (runuser -u cua -- env DISPLAY=:1 HOME=`+home+` setsid google-chrome `+signInClass+
		` --user-data-dir=`+profiles+name+` --password-store=basic --no-first-run --no-default-browser-check --start-maximized `+quote(url)+` >/dev/null 2>&1 </dev/null &)`)
}

func quote(s string) string { return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'" }

// giveBack deletes a borrowed device; a sign-in's profile is saved first.
func (ds *devices) giveBack(ctx context.Context, name, why string) error {
	l, ok := ds.loans()[name]
	if !ok {
		return fmt.Errorf("device %s isn't borrowed", name)
	}
	d, err := find(ctx, name)
	if err == nil {
		if p := ds.heldProfile(name); l.SigningIn && p == nil {
			// Its key went when this program restarted: the app's save_profile gives it again, and gives the
			// device back once it's saved.
			if _, err := ds.callApp(ctx, "save_profile", map[string]any{"item": l.Item}); err != nil {
				return fmt.Errorf("not given back: saving the %s profile failed: %w", l.Profile, err)
			}
			return nil
		} else if l.SigningIn {
			if err := ds.saveProfile(ctx, d, p); err != nil {
				return fmt.Errorf("not given back: saving the %s profile failed: %w", l.Profile, err)
			}
		}
		ds.drop(name)
		if err := remove(ctx, d); err != nil {
			return err
		}
	}
	log.Printf("%s: %s", name, why)
	ds.setLoan(name, nil)
	ds.poke()
	return nil
}

// saveProfile saves the device's copy of a profile as its newest save,
// recorded on its Vault item by the app.
func (ds *devices) saveProfile(ctx context.Context, d device, p *profile) error {
	if p == nil {
		return errors.New("its profile's key is gone, as after this server restarted: save it with save_profile")
	}
	if d.State == "paused" {
		unpause(ctx, d)
	}
	s, err := ds.pack(ctx, d, p)
	if err != nil {
		return err
	}
	if err := ds.saved(ctx, s); err != nil {
		return err
	}
	// The next save starts from this one.
	p.Blob, p.Salt = s.Blob, s.Salt
	return nil
}

// signIn opens a page in a device on the profile, for the person to sign in
// to sites in, reusing one already signing in to it.
func (ds *devices) signIn(ctx context.Context, p *profile, url string, minutes int, by string) (device, time.Time, error) {
	until := time.Now().Add(time.Duration(minutes) * time.Minute).UTC()
	for name, l := range ds.loans() {
		if l.SigningIn && l.Item == p.Item {
			d, err := ds.running(ctx, name)
			if err != nil {
				return d, until, err
			}
			l.Until, l.Profile = until, p.Name
			ds.setLoan(name, &l)
			if ds.heldProfile(name) == nil {
				ds.hold(name, p)
			}
			return d, until, openPage(ctx, d, p.Name, url)
		}
	}
	d, err := ds.borrow(ctx, loan{By: by, For: "Signing in to sites", SigningIn: true, Until: until}, p)
	if err != nil {
		return d, until, err
	}
	return d, until, openPage(ctx, d, p.Name, url)
}

// keep gives back borrowed devices left idle or whose sign-in time is up,
// keeps one spare, pauses idle devices and starts again those that should
// be running, for as long as this runs.
func (ds *devices) keep(ctx context.Context) {
	revive(ctx)
	for {
		ds.tend(ctx)
		select {
		case <-ctx.Done():
			return
		case <-ds.kick:
		case <-time.After(30 * time.Second):
		}
	}
}

func (ds *devices) tend(ctx context.Context) {
	all, err := list(ctx)
	if err != nil {
		log.Print(err)
		return
	}
	here := map[string]bool{}
	for _, d := range all {
		here[d.Name] = true
	}
	for name, l := range ds.loans() {
		why := ""
		switch {
		case !here[name]:
			why = "gone"
		case l.SigningIn && time.Now().After(l.Until):
			why = "sign-in time up"
		case !l.SigningIn && ds.idle(name) > idleBorrowed:
			why = "idle for 15 minutes"
		}
		if why != "" {
			if err := ds.giveBack(ctx, name, why); err != nil {
				log.Printf("giving back %s: %v", name, err)
			}
		}
	}

	ds.take.Lock()
	defer ds.take.Unlock()
	if all, err = list(ctx); err != nil {
		return
	}
	lent := ds.loans()
	for _, d := range all {
		if _, ok := lent[d.Name]; d.pool && !ok && d.State == "stopped" {
			ds.drop(d.Name)
			remove(ctx, d)
		}
	}
	free := spares(all, lent)
	// A spare on an older image makes way for one on this week's.
	img := imageFor(time.Now())
	free = slices.DeleteFunc(free, func(d device) bool {
		if d.image == img {
			return false
		}
		ds.drop(d.Name)
		remove(ctx, d)
		return true
	})
	slices.SortFunc(free, func(a, b device) int { return strings.Compare(a.Name, b.Name) })
	for _, d := range free[min(1, len(free)):] {
		ds.drop(d.Name)
		remove(ctx, d)
	}
	if len(free) == 0 && keepsSpare() {
		d, err := create(ctx, newPoolName(), sizes[0], true, true)
		if err != nil {
			log.Printf("making a spare device: %v", err)
		} else {
			ds.touch(d.Name)
			ds.refreshIfEmpty(ctx)
		}
	}
	ds.pauseIdle(ctx, all, lent)
}

// keepsSpare reports whether this server keeps a spare device: a Linux one
// with room for it. A Mac's memory is its person's; it makes devices only
// when asked, and passes borrowing on to a server with a spare.
func keepsSpare() bool { return runtime.GOOS != "darwin" && roomForOne() }

// makesHere reports whether a borrow here would be quick, or this server is
// the one to make a device for it.
func (ds *devices) makesHere(ctx context.Context) bool {
	all, err := list(ctx)
	return err == nil && len(spares(all, ds.loans())) > 0 || keepsSpare()
}

// roomForOne reports whether another small device fits: as much memory
// available as it may use.
func roomForOne() bool {
	free, _ := memory()
	return free >= int64(sizes[0].GB)<<30
}

// memory is this server's available and total memory, in bytes.
func memory() (free, total int64) {
	if runtime.GOOS == "darwin" {
		return macMemory()
	}
	b, err := os.ReadFile("/proc/meminfo")
	if err != nil {
		return 0, 0
	}
	for line := range strings.Lines(string(b)) {
		k, v, _ := strings.Cut(line, ":")
		var n int64
		fmt.Sscan(strings.TrimSuffix(strings.TrimSpace(v), " kB"), &n)
		switch k {
		case "MemAvailable":
			free = n << 10
		case "MemTotal":
			total = n << 10
		}
	}
	return free, total
}

// macMemory reads vm_stat: free, inactive and speculative pages are
// available.
func macMemory() (free, total int64) {
	out, err := exec.Command("sysctl", "-n", "hw.memsize").Output()
	if err != nil {
		return 0, 0
	}
	fmt.Sscan(strings.TrimSpace(string(out)), &total)
	if out, err = exec.Command("vm_stat").Output(); err != nil {
		return 0, 0
	}
	page := int64(16384)
	for line := range strings.Lines(string(out)) {
		if _, rest, ok := strings.Cut(line, "page size of "); ok {
			fmt.Sscan(rest, &page)
		}
		for _, k := range []string{"Pages free:", "Pages inactive:", "Pages speculative:"} {
			if v, ok := strings.CutPrefix(line, k); ok {
				var n int64
				fmt.Sscan(strings.TrimSuffix(strings.TrimSpace(v), "."), &n)
				free += n * page
			}
		}
	}
	return free, total
}
