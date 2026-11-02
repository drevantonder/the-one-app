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

// Borrowed computers: an agent borrows a computer of its own for as long as it
// needs a browser, and gives it back, which deletes it. They come from a pool:
// one spare is kept running, so borrowing is quick. A borrowed computer's
// browser starts with a copy of a browser profile (profiles.go), which the
// person signed in to sites in once (sign_in); the copy is thrown away with
// the computer, so only sign_in and save_profile change a profile.

const (
	poolPrefix   = "pool-"
	idleBorrowed = 15 * time.Minute // a borrowed computer left this long is given back
	signInFor    = 30 * time.Minute // a sign-in is saved and closed after this, by default
	profiles     = home + "/.local/state/cua-driver/browser-profiles/"
	// signInClass marks the browser sign_in opens, which the tools didn't, so
	// closing the tools' leftover browsers leaves it open.
	signInClass = "--class=one-sign-in"
)

// A loan is a borrowed computer's: who has it and what for, and the browser
// profile it started with. Its profile's key is kept only in memory, in held.
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

// caches are left out of saved profiles and backups: they are most of a
// browser profile's size and rebuild themselves. So are a running browser's
// locks, which would make a copy look in use by another computer.
var caches = []string{"./.cache", "*/Cache", "*/Code Cache", "*/GPUCache", "*/DawnCache", "*/GrShaderCache", "*/ShaderCache",
	"*/Service Worker/CacheStorage", "*/Service Worker/ScriptCache", "*/SingletonLock", "*/SingletonSocket", "*/SingletonCookie"}

func (ds *computers) loans() map[string]loan {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	return maps.Clone(ds.lent)
}

// setLoan records or (with nil) forgets a loan, on disk too, so they outlive
// this program.
func (ds *computers) setLoan(name string, l *loan) {
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

func (ds *computers) loadLoans() {
	b, err := os.ReadFile(filepath.Join(ds.dir, "lent.json"))
	if err == nil {
		json.Unmarshal(b, &ds.lent)
	}
	if ds.lent == nil {
		ds.lent = map[string]loan{}
	}
}

// touch notes a call to a computer, which keeps it from being given back or
// paused while idle.
func (ds *computers) touch(name string) {
	ds.poolMu.Lock()
	ds.used[name] = time.Now()
	ds.poolMu.Unlock()
}

// idle is how long since the computer's last call, or since this program
// first saw it.
func (ds *computers) idle(name string) time.Duration {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	t, ok := ds.used[name]
	if !ok {
		ds.used[name] = time.Now()
		return 0
	}
	return time.Since(t)
}

func (ds *computers) poke() {
	select {
	case ds.kick <- struct{}{}:
	default:
	}
}

// spares are the pool's computers nobody has borrowed.
func spares(all []computer, lent map[string]loan) []computer {
	var out []computer
	for _, d := range all {
		if _, ok := lent[d.Name]; d.pool && !ok && d.State != "stopped" {
			out = append(out, d)
		}
	}
	return out
}

// borrow lends the spare computer, or one made for it, its browser starting
// with the profile p, if one is given.
func (ds *computers) borrow(ctx context.Context, l loan, p *profile) (computer, error) {
	if p != nil {
		l.Profile, l.Item = p.Name, p.Item
	}
	ds.take.Lock()
	all, err := list(ctx)
	if err != nil {
		ds.take.Unlock()
		return computer{}, err
	}
	var d computer
	if free := spares(all, ds.loans()); len(free) > 0 {
		d = free[0]
	} else if !roomForOne() {
		ds.take.Unlock()
		return computer{}, errors.New("this server has no memory to spare for another computer: give back one you're done with")
	} else if d, err = create(ctx, newPoolName(), sizes[0], true, false); err != nil {
		ds.take.Unlock()
		return computer{}, err
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

// hold keeps a borrowed computer's profile, with its key, while it's lent.
func (ds *computers) hold(name string, p *profile) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if p == nil {
		delete(ds.held, name)
	} else {
		ds.held[name] = p
	}
}

func (ds *computers) heldProfile(name string) *profile {
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
func openPage(ctx context.Context, d computer, name, url string) error {
	if url == "" {
		url = "about:blank"
	}
	return sh(ctx, d, owned(name)+` && (runuser -u cua -- env DISPLAY=:1 HOME=`+home+` setsid google-chrome `+signInClass+
		` --user-data-dir=`+profiles+name+` --password-store=basic --no-first-run --no-default-browser-check --start-maximized `+quote(url)+` >/dev/null 2>&1 </dev/null &)`)
}

func quote(s string) string { return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'" }

// giveBack deletes a borrowed computer; a sign-in's profile is saved first.
func (ds *computers) giveBack(ctx context.Context, name, why string) error {
	l, ok := ds.loans()[name]
	if !ok {
		return fmt.Errorf("computer %s isn't borrowed", name)
	}
	d, err := find(ctx, name)
	if err == nil {
		if p := ds.heldProfile(name); l.SigningIn && p == nil {
			log.Printf("%s: not saving the %s profile: its key is gone, as after this program restarted", name, l.Profile)
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

// saveProfile saves the computer's copy of a profile as its newest save,
// recorded on its Vault item by the app.
func (ds *computers) saveProfile(ctx context.Context, d computer, p *profile) error {
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

// signIn opens a page in a computer on the profile, for the person to sign in
// to sites in, reusing one already signing in to it.
func (ds *computers) signIn(ctx context.Context, p *profile, url string, minutes int, by string) (computer, time.Time, error) {
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

// keep gives back borrowed computers left idle or whose sign-in time is up,
// keeps one spare, pauses idle computers and starts again those that should
// be running, for as long as this runs.
func (ds *computers) keep(ctx context.Context) {
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

func (ds *computers) tend(ctx context.Context) {
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
	free = slices.DeleteFunc(free, func(d computer) bool {
		if d.image == img {
			return false
		}
		ds.drop(d.Name)
		remove(ctx, d)
		return true
	})
	slices.SortFunc(free, func(a, b computer) int { return strings.Compare(a.Name, b.Name) })
	for _, d := range free[min(1, len(free)):] {
		ds.drop(d.Name)
		remove(ctx, d)
	}
	if len(free) == 0 && keepsSpare() {
		d, err := create(ctx, newPoolName(), sizes[0], true, true)
		if err != nil {
			log.Printf("making a spare computer: %v", err)
		} else {
			ds.touch(d.Name)
			ds.refreshIfEmpty(ctx)
		}
	}
	ds.pauseIdle(ctx, all, lent)
}

// keepsSpare reports whether this server keeps a spare computer: a Linux one
// with room for it. A Mac's memory is its person's; it makes computers only
// when asked, and passes borrowing on to a server with a spare.
func keepsSpare() bool { return runtime.GOOS != "darwin" && roomForOne() }

// makesHere reports whether a borrow here would be quick, or this server is
// the one to make a computer for it.
func (ds *computers) makesHere(ctx context.Context) bool {
	all, err := list(ctx)
	return err == nil && len(spares(all, ds.loans())) > 0 || keepsSpare()
}

// roomForOne reports whether another small computer fits, leaving plenty for
// everything else: a third of the memory, and at least 6 GB.
func roomForOne() bool {
	free, total := memory()
	return total > 0 && free-int64(sizes[0].GB)<<30 >= max(total*3/10, 6<<30)
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
