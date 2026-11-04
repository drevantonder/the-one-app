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
// profile it started with, with its key, kept only in memory: after a
// restart, the app gives it again when the profile is saved.
type loan struct {
	By        string    `json:"by"`
	For       string    `json:"for"`
	Profile   string    `json:"profile,omitempty"` // the profile's name
	Item      string    `json:"item,omitempty"`    // its Vault item
	Since     time.Time `json:"since"`
	SigningIn bool      `json:"signing_in,omitempty"`
	Until     time.Time `json:"until,omitzero"`      // a sign-in's end
	NoScreen  bool      `json:"no_screen,omitempty"` // made without a screen: nobody watches it
	State     loanState `json:"state,omitempty"`
	key       *profile
}

// A loan's state. It starts preparing, its device chosen and being readied;
// once ready it's lent, and the borrower's; giving it back, it's returning,
// until the device is deleted and the loan forgotten, or back to lent if that
// failed. Each step is one of the methods below, all under poolMu, and each
// is saved in lent.json, so a restart carries on: a loan preparing when this
// program stopped never got ready, and is given back.
type loanState string

const (
	loanLent      loanState = ""
	loanPreparing loanState = "preparing"
	loanReturning loanState = "returning"
)

// someone borrowed what a caller with no name did, which never carries on another's loan.
const someone = "someone"

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

// reserve starts a loan of the device named, preparing it.
func (ds *devices) reserve(name string, l loan) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	l.State = loanPreparing
	ds.lent[name] = l
	ds.saveLoans()
}

// lend makes a loan the borrower's, once its device is ready.
func (ds *devices) lend(name string) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if l, ok := ds.lent[name]; ok && l.State == loanPreparing {
		l.State = loanLent
		ds.lent[name] = l
		ds.saveLoans()
	}
}

// change changes a lent loan, reporting false if it isn't lent.
func (ds *devices) change(name string, f func(*loan)) bool {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	l, ok := ds.lent[name]
	if !ok || l.State != loanLent {
		return false
	}
	f(&l)
	ds.lent[name] = l
	ds.saveLoans()
	return true
}

// returning starts giving back a loan, preparing or lent: it's the loan as it
// was, which only one caller gets.
func (ds *devices) returning(name string) (loan, error) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	l, ok := ds.lent[name]
	switch {
	case !ok:
		return l, fmt.Errorf("device %s isn't borrowed", name)
	case l.State == loanReturning:
		return l, fmt.Errorf("device %s is being given back already", name)
	}
	r := l
	r.State = loanReturning
	ds.lent[name] = r
	ds.saveLoans()
	return l, nil
}

// returnFailed makes a loan whose giving back failed lent again.
func (ds *devices) returnFailed(name string) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if l, ok := ds.lent[name]; ok {
		l.State = loanLent
		ds.lent[name] = l
		ds.saveLoans()
	}
}

// endLoan forgets a loan, with its profile's key.
func (ds *devices) endLoan(name string) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if _, ok := ds.lent[name]; ok {
		delete(ds.lent, name)
		ds.saveLoans()
	}
}

// saveLoans writes the loans to disk, so they outlive this program; poolMu is held.
func (ds *devices) saveLoans() {
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
	for name, l := range ds.lent {
		if l.State == loanReturning {
			l.State = loanLent // given back again, as it would be
			ds.lent[name] = l
		}
	}
	if b, err := os.ReadFile(filepath.Join(ds.dir, "used.json")); err == nil {
		json.Unmarshal(b, &ds.used)
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

// saveUsed keeps each device's last call on disk, so a restart doesn't make
// every device look just used, forgetting the devices not kept.
func (ds *devices) saveUsed(kept func(string) bool) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	maps.DeleteFunc(ds.used, func(name string, _ time.Time) bool { return !kept(name) })
	b, _ := json.Marshal(ds.used)
	if err := os.WriteFile(filepath.Join(ds.dir, "used.json"), b, 0o600); err != nil {
		log.Print(err)
	}
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
	ds.take.Lock()
	return ds.borrowTaken(ctx, l, p)
}

// borrowTaken is borrow with take held, which it lets go once it has chosen
// the device. The loan is the borrower's only once the device is ready: on
// any failure before then it's given back.
func (ds *devices) borrowTaken(ctx context.Context, l loan, p *profile) (device, error) {
	if p != nil {
		l.Profile, l.Item, l.key = p.Name, p.Item, p
	}
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
		return device{}, errors.New("this device has no memory to spare for another agent device: give back one you're done with")
	} else if d, err = create(ctx, newPoolName(), sizes[0], true, false); err != nil {
		ds.take.Unlock()
		return device{}, err
	}
	l.Since = time.Now().UTC()
	ds.reserve(d.Name, l)
	ds.touch(d.Name)
	ds.take.Unlock()
	ds.poke() // for another spare
	err = nil
	if d.State == "paused" {
		err = unpause(ctx, d)
	}
	if err == nil {
		err = ready(ctx, d)
	}
	if err == nil && p != nil {
		err = ds.unpack(ctx, d, p)
	}
	if err != nil {
		if e := ds.giveBack(context.WithoutCancel(ctx), d.Name, "couldn't prepare it: "+err.Error()); e != nil {
			log.Printf("giving back %s: %v", d.Name, e)
		}
		return d, err
	}
	ds.lend(d.Name)
	log.Printf("%s borrowed %s for %q, profile %q", l.By, d.Name, l.For, l.Profile)
	return d, nil
}

// holdKey gives a lent device's loan its profile, with its key.
func (ds *devices) holdKey(name string, p *profile) {
	ds.change(name, func(l *loan) { l.key = p })
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

// A sign-in whose profile can't be saved is given back unsaved this long
// after its time is up, as when the app never gives its key back after a
// restart, or its device won't start.
const signInGrace = 10 * time.Minute

// giveBack deletes a borrowed device; a sign-in's profile is saved first.
func (ds *devices) giveBack(ctx context.Context, name, why string) error {
	l, ok := ds.loans()[name]
	if !ok {
		return fmt.Errorf("device %s isn't borrowed", name)
	}
	late := time.Now().After(l.Until.Add(signInGrace))
	if l.State == loanLent && l.SigningIn && l.key == nil && !late {
		// Its key went when this program restarted: the app's save_profile gives it again, and gives the
		// device back once it's saved.
		if _, err := ds.callApp(ctx, "save_profile", map[string]any{"item": l.Item}); err != nil {
			return fmt.Errorf("not given back: saving the %s profile failed: %w", l.Profile, err)
		}
		return nil
	}
	l, err := ds.returning(name)
	if err != nil {
		return err
	}
	d, err := find(ctx, name)
	if err == nil {
		if l.State == loanLent && l.SigningIn {
			if err := ds.saveProfile(ctx, d, l.key); err != nil && !late {
				ds.returnFailed(name)
				return fmt.Errorf("not given back: saving the %s profile failed: %w", l.Profile, err)
			} else if err != nil {
				log.Printf("%s: giving it back without saving the %s profile: %v", name, l.Profile, err)
			}
		}
		ds.drop(name)
		if err := remove(ctx, d); err != nil {
			ds.returnFailed(name)
			return err
		}
	}
	log.Printf("%s: %s", name, why)
	ds.endLoan(name)
	ds.poke()
	return nil
}

// saveProfile saves the device's copy of a profile as its newest save,
// recorded on its Vault item by the app.
func (ds *devices) saveProfile(ctx context.Context, d device, p *profile) error {
	if p == nil {
		return errors.New("its profile's key is gone, as after this device restarted: save it with save_profile")
	}
	switch d.State {
	case "paused":
		if err := unpause(ctx, d); err != nil {
			return err
		}
	case "stopped":
		if err := start(ctx, d); err != nil {
			return err
		}
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
// to sites in, reusing one already signing in to it. take is held while it
// looks for one, so two sign-ins to a profile share one device.
func (ds *devices) signIn(ctx context.Context, p *profile, url string, minutes int, by string) (device, time.Time, error) {
	until := time.Now().Add(time.Duration(minutes) * time.Minute).UTC()
	ds.take.Lock()
	for name, l := range ds.loans() {
		if !l.SigningIn || l.Item != p.Item {
			continue
		}
		ds.take.Unlock()
		if l.State != loanLent {
			return device{}, until, fmt.Errorf("the %s profile's sign-in is starting or closing: try again in a moment", p.Name)
		}
		d, err := ds.running(ctx, name)
		if err != nil {
			return d, until, err
		}
		ds.change(name, func(l *loan) {
			l.Until, l.Profile = until, p.Name
			if l.key == nil {
				l.key = p
			}
		})
		return d, until, openPage(ctx, d, p.Name, url)
	}
	d, err := ds.borrowTaken(ctx, loan{By: by, For: "Signing in to sites", SigningIn: true, Until: until}, p)
	if err != nil {
		return d, until, err
	}
	return d, until, openPage(ctx, d, p.Name, url)
}

// keep gives back borrowed devices left idle or whose sign-in time is up,
// keeps one spare, pauses idle devices and starts again those that should
// be running, for as long as this runs.
func (ds *devices) keep(ctx context.Context) {
	ds.revive(ctx)
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
	ds.tendVMs(ctx)
	ds.tendMoved(ctx, all)
	here := map[string]bool{}
	for _, d := range all {
		here[d.Name] = true
	}
	ds.saveUsed(func(name string) bool { return here[name] || ds.vmNamed(name) != nil })
	for name, l := range ds.loans() {
		if ds.vmNamed(name) != nil || l.State == loanReturning || l.State == loanPreparing && l.Since.After(ds.started) {
			continue
		}
		why := ""
		switch {
		case l.State == loanPreparing:
			why = "never got ready before this program restarted"
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

// keepsSpare reports whether this device keeps a spare agent device: a Linux one
// with room for it. A Mac's memory is its person's; it makes devices only
// when asked.
func keepsSpare() bool { return runtime.GOOS != "darwin" && roomForOne() }

// roomForOne reports whether another small device fits: as much memory
// available as it may use.
func roomForOne() bool {
	free, _ := memory()
	return free >= int64(sizes[0].GB)<<30
}

// memory is this device's available and total memory, in bytes: available less the swap in use.
func memory() (free, total int64) {
	if runtime.GOOS == "darwin" {
		return macMemory()
	}
	b, err := os.ReadFile("/proc/meminfo")
	if err != nil {
		return 0, 0
	}
	var swapTotal, swapFree int64
	for line := range strings.Lines(string(b)) {
		k, v, _ := strings.Cut(line, ":")
		var n int64
		fmt.Sscan(strings.TrimSuffix(strings.TrimSpace(v), " kB"), &n)
		switch k {
		case "MemAvailable":
			free = n << 10
		case "MemTotal":
			total = n << 10
		case "SwapTotal":
			swapTotal = n << 10
		case "SwapFree":
			swapFree = n << 10
		}
	}
	// Swap in use is memory already short: what's available counts only once it's paid back.
	return max(free-(swapTotal-swapFree), 0), total
}

// macMemory is macOS's own measure of the memory free, compressed pages and swap counted: memory_pressure's free
// percentage of the Mac's memory.
func macMemory() (free, total int64) {
	out, err := exec.Command("sysctl", "-n", "hw.memsize").Output()
	if err != nil {
		return 0, 0
	}
	fmt.Sscan(strings.TrimSpace(string(out)), &total)
	if out, err = exec.Command("memory_pressure", "-Q").Output(); err != nil {
		return 0, total
	}
	_, pct, _ := strings.Cut(string(out), "free percentage:")
	var p int64
	fmt.Sscan(strings.TrimSuffix(strings.TrimSpace(pct), "%"), &p)
	return total * p / 100, total
}
