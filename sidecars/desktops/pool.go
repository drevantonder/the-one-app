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

// Borrowed desktops: an agent borrows a desktop of its own for as long as it
// needs a browser, and gives it back, which deletes it. They come from a pool:
// one spare is kept running, so borrowing is quick. A borrowed desktop's
// browser starts with a copy of a saved login, a browser profile the person
// signed in to sites in once (sign_in); the copy is thrown away with the
// desktop, so only sign_in and save_login change a saved login. Saved logins
// are files on this server, the last few of each kept.

const (
	poolPrefix   = "pool-"
	idleBorrowed = 15 * time.Minute // a borrowed desktop left this long is given back
	signInFor    = 30 * time.Minute // a sign-in is saved and closed after this, by default
	profiles     = home + "/.local/state/cua-driver/browser-profiles/"
	keepLogins   = 3
	// signInClass marks the browser sign_in opens, which the tools didn't, so
	// closing the tools' leftover browsers leaves it open.
	signInClass = "--class=one-sign-in"
)

// A loan is a borrowed desktop's: who has it and what for.
type loan struct {
	By        string    `json:"by"`
	For       string    `json:"for"`
	Login     string    `json:"login,omitempty"`
	Since     time.Time `json:"since"`
	SigningIn bool      `json:"signing_in,omitempty"`
	Until     time.Time `json:"until,omitzero"` // a sign-in's end
}

// caches are left out of saved logins and backups: they are most of a
// browser profile's size and rebuild themselves. So are a running browser's
// locks, which would make a copy look in use by another computer.
var caches = []string{"./.cache", "*/Cache", "*/Code Cache", "*/GPUCache", "*/DawnCache", "*/GrShaderCache", "*/ShaderCache",
	"*/Service Worker/CacheStorage", "*/Service Worker/ScriptCache", "*/SingletonLock", "*/SingletonSocket", "*/SingletonCookie"}

func (ds *desktops) loans() map[string]loan {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	return maps.Clone(ds.lent)
}

// setLoan records or (with nil) forgets a loan, on disk too, so they outlive
// this program.
func (ds *desktops) setLoan(name string, l *loan) {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	if l == nil {
		delete(ds.lent, name)
	} else {
		ds.lent[name] = *l
	}
	b, _ := json.MarshalIndent(ds.lent, "", "  ")
	if err := os.WriteFile(filepath.Join(ds.dir, "lent.json"), b, 0o600); err != nil {
		log.Print(err)
	}
}

func (ds *desktops) loadLoans() {
	b, err := os.ReadFile(filepath.Join(ds.dir, "lent.json"))
	if err == nil {
		json.Unmarshal(b, &ds.lent)
	}
	if ds.lent == nil {
		ds.lent = map[string]loan{}
	}
}

// touch notes a call to a desktop, which keeps it from being given back or
// paused while idle.
func (ds *desktops) touch(name string) {
	ds.poolMu.Lock()
	ds.used[name] = time.Now()
	ds.poolMu.Unlock()
}

// idle is how long since the desktop's last call, or since this program
// first saw it.
func (ds *desktops) idle(name string) time.Duration {
	ds.poolMu.Lock()
	defer ds.poolMu.Unlock()
	t, ok := ds.used[name]
	if !ok {
		ds.used[name] = time.Now()
		return 0
	}
	return time.Since(t)
}

func (ds *desktops) poke() {
	select {
	case ds.kick <- struct{}{}:
	default:
	}
}

// spares are the pool's desktops nobody has borrowed.
func spares(all []desktop, lent map[string]loan) []desktop {
	var out []desktop
	for _, d := range all {
		if _, ok := lent[d.Name]; d.pool && !ok && d.State != "stopped" {
			out = append(out, d)
		}
	}
	return out
}

// borrow lends the spare desktop, or one made for it.
func (ds *desktops) borrow(ctx context.Context, l loan) (desktop, error) {
	if l.Login != "" {
		if !validProfile.MatchString(l.Login) {
			return desktop{}, fmt.Errorf("no profile %q", l.Login)
		}
		if ds.savedLogin(l.Login) == "" && !l.SigningIn {
			return desktop{}, fmt.Errorf("no profile %q: sign in to sites in one with sign_in", l.Login)
		}
	}
	ds.take.Lock()
	all, err := list(ctx)
	if err != nil {
		ds.take.Unlock()
		return desktop{}, err
	}
	var d desktop
	if free := spares(all, ds.loans()); len(free) > 0 {
		d = free[0]
	} else if !roomForOne() {
		ds.take.Unlock()
		return desktop{}, errors.New("this server has no memory to spare for another desktop: give back one you're done with")
	} else if d, err = create(ctx, newPoolName(), sizes[0], true, false); err != nil {
		ds.take.Unlock()
		return desktop{}, err
	}
	l.Since = time.Now().UTC()
	ds.setLoan(d.Name, &l)
	ds.touch(d.Name)
	ds.take.Unlock()
	log.Printf("%s borrowed %s for %q, login %q", l.By, d.Name, l.For, l.Login)
	ds.poke() // for another spare
	if d.State == "paused" {
		if err := unpause(ctx, d); err != nil {
			return d, err
		}
	}
	if err := ready(ctx, d); err != nil {
		return d, err
	}
	if err := ds.prepare(ctx, d, l); err != nil {
		ds.giveBack(context.WithoutCancel(ctx), d.Name, "couldn't prepare it")
		return d, err
	}
	return d, nil
}

func newPoolName() string {
	b := make([]byte, 3)
	rand.Read(b)
	return poolPrefix + hex.EncodeToString(b)
}

// prepare puts the saved login in the desktop's browser profiles.
func (ds *desktops) prepare(ctx context.Context, d desktop, l loan) error {
	if l.Login == "" {
		return nil
	}
	saved := ds.savedLogin(l.Login)
	if saved == "" {
		return sh(ctx, d, owned(l.Login))
	}
	f, err := os.Open(saved)
	if err != nil {
		return err
	}
	defer f.Close()
	dir := profiles + l.Login
	if err := in(ctx, d, f, nil, "sh", "-c", `mkdir -p "$1" && tar -C "$1" -xzf - && `+owned(l.Login), "sh", dir); err != nil {
		return fmt.Errorf("unpacking the %s login: %w", l.Login, err)
	}
	return nil
}

// owned makes the login's profile one the browser tools will open: they only
// open profiles they made, as a file in it says.
func owned(login string) string {
	dir := profiles + login
	return fmt.Sprintf(`mkdir -p %s && printf '%%s' '{"schema":"cua-driver-browser-profile-v1","mode":"isolated_named","name":%q}' > %s/.cua-driver-owned-profile.json && chmod 700 %s && chown -R cua:cua %s/.local`,
		dir, login, dir, dir, home)
}

// open opens a page in a browser on the login's profile, the person's to use
// through view.
func open(ctx context.Context, d desktop, login, url string) error {
	if url == "" {
		url = "about:blank"
	}
	return sh(ctx, d, owned(login)+` && (runuser -u cua -- env DISPLAY=:1 HOME=`+home+` setsid chromium `+signInClass+
		` --user-data-dir=`+profiles+login+` --password-store=basic --no-first-run --start-maximized `+quote(url)+` >/dev/null 2>&1 </dev/null &)`)
}

func quote(s string) string { return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'" }

// giveBack deletes a borrowed desktop; a sign-in's login is saved first.
func (ds *desktops) giveBack(ctx context.Context, name, why string) error {
	l, ok := ds.loans()[name]
	if !ok {
		return fmt.Errorf("desktop %s isn't borrowed", name)
	}
	d, err := find(ctx, name)
	if err == nil {
		if l.SigningIn {
			if d.State == "paused" {
				unpause(ctx, d)
			}
			if err := ds.saveLogin(ctx, d, l.Login); err != nil {
				return fmt.Errorf("not given back: saving the %s login failed: %w", l.Login, err)
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

// signIn opens a page in a desktop on the saved login, for the person to sign
// in to sites in, reusing one already signing in to it.
func (ds *desktops) signIn(ctx context.Context, login, url string, minutes int, by string) (desktop, time.Time, error) {
	until := time.Now().Add(time.Duration(minutes) * time.Minute).UTC()
	for name, l := range ds.loans() {
		if l.SigningIn && l.Login == login {
			d, err := ds.running(ctx, name)
			if err != nil {
				return d, until, err
			}
			l.Until = until
			ds.setLoan(name, &l)
			return d, until, open(ctx, d, login, url)
		}
	}
	d, err := ds.borrow(ctx, loan{By: by, For: "Signing in to sites", Login: login, SigningIn: true, Until: until})
	if err != nil {
		return d, until, err
	}
	return d, until, open(ctx, d, login, url)
}

func (ds *desktops) loginDir(login string) string { return filepath.Join(ds.dir, "logins", login) }

// keeps reports whether this server keeps the profile, saved or added with no sign-ins yet.
func (ds *desktops) keeps(login string) bool {
	info, err := os.Stat(ds.loginDir(login))
	return err == nil && info.IsDir()
}

// savedLogin is the newest copy of a saved login's profile, or "".
func (ds *desktops) savedLogin(login string) string {
	all := ds.savedCopies(login)
	if len(all) == 0 {
		return ""
	}
	return all[0]
}

// savedCopies are a saved login's copies, newest first.
func (ds *desktops) savedCopies(login string) []string {
	files, _ := filepath.Glob(filepath.Join(ds.loginDir(login), "*.tar.gz"))
	slices.Sort(files)
	slices.Reverse(files)
	return files
}

// saveLogin closes the desktop's browsers, so their profiles are written out,
// and keeps a copy of the login's profile as the saved login.
func (ds *desktops) saveLogin(ctx context.Context, d desktop, login string) error {
	if err := sh(ctx, d, "test -d "+profiles+login); err != nil {
		return fmt.Errorf("desktop %s's browser has no %s profile", d.Name, login)
	}
	closeBrowsers(ctx, d, true)
	if err := os.MkdirAll(ds.loginDir(login), 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(ds.loginDir(login), ".saving-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	args := []string{"tar", "-C", profiles + login, "--warning=no-file-changed"}
	for _, c := range caches {
		args = append(args, "--exclude="+c)
	}
	err = in(ctx, d, nil, tmp, append(args, "-czf", "-", ".")...)
	tmp.Close()
	if !tarred(err) {
		return err
	}
	name := filepath.Join(ds.loginDir(login), time.Now().UTC().Format("20060102T150405Z")+".tar.gz")
	if err := os.Rename(tmp.Name(), name); err != nil {
		return err
	}
	copies := ds.savedCopies(login)
	for _, old := range copies[min(keepLogins, len(copies)):] {
		os.Remove(old)
	}
	log.Printf("saved the %s login from %s", login, d.Name)
	return nil
}

// loginServer is the other server that keeps the saved login, or "".
func (ds *desktops) loginServer(ctx context.Context, login string) string {
	got, _ := ds.everywhere(ctx, "profiles", nil)
	for s, r := range got {
		var theirs struct{ Profiles []struct{ Profile string } }
		json.Unmarshal(r, &theirs)
		for _, l := range theirs.Profiles {
			if l.Profile == login {
				return s
			}
		}
	}
	return ""
}

// profileElsewhere passes a call about a profile this server doesn't keep on
// to the server that does, if one does: each server keeps its own.
func (ds *desktops) profileElsewhere(ctx context.Context, op string, in map[string]any, login string, here bool) json.RawMessage {
	if here || login == "" || ds.keeps(login) {
		return nil
	}
	s := ds.loginServer(ctx, login)
	if s == "" {
		return nil
	}
	fwd := map[string]any{"here": true}
	for k, v := range in {
		fwd[k] = v
	}
	fwd["login"] = login
	r, err := ds.callNode(ctx, s, op, fwd)
	if err != nil {
		log.Printf("%s on %s: %v", op, s, err)
		return nil
	}
	return r
}

// logins lists this server's profiles, those with no sign-ins yet without
// saved: each server keeps its own.
func (ds *desktops) logins(me string) []map[string]any {
	dirs, _ := os.ReadDir(filepath.Join(ds.dir, "logins"))
	out := []map[string]any{}
	for _, e := range dirs {
		if !e.IsDir() {
			continue
		}
		saved := ds.savedLogin(e.Name())
		if saved == "" {
			out = append(out, map[string]any{"profile": e.Name(), "server": me})
			continue
		}
		info, err := os.Stat(saved)
		if err != nil {
			continue
		}
		out = append(out, map[string]any{"profile": e.Name(), "server": me, "saved": info.ModTime().UTC(), "size": info.Size()})
	}
	return out
}

// keep gives back borrowed desktops left idle or whose sign-in time is up,
// keeps one spare, pauses idle desktops and starts again those that should
// be running, for as long as this runs.
func (ds *desktops) keep(ctx context.Context) {
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

func (ds *desktops) tend(ctx context.Context) {
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
	slices.SortFunc(free, func(a, b desktop) int { return strings.Compare(a.Name, b.Name) })
	for _, d := range free[min(1, len(free)):] {
		ds.drop(d.Name)
		remove(ctx, d)
	}
	if len(free) == 0 && keepsSpare() {
		d, err := create(ctx, newPoolName(), sizes[0], true, true)
		if err != nil {
			log.Printf("making a spare desktop: %v", err)
		} else {
			ds.touch(d.Name)
			ds.refreshIfEmpty(ctx)
		}
	}
	ds.pauseIdle(ctx, all, lent)
}

// keepsSpare reports whether this server keeps a spare desktop: a Linux one
// with room for it. A Mac's memory is its person's; it makes desktops only
// when asked, and passes borrowing on to a server with a spare.
func keepsSpare() bool { return runtime.GOOS != "darwin" && roomForOne() }

// makesHere reports whether a borrow here would be quick, or this server is
// the one to make a desktop for it.
func (ds *desktops) makesHere(ctx context.Context) bool {
	all, err := list(ctx)
	return err == nil && len(spares(all, ds.loans())) > 0 || keepsSpare()
}

// roomForOne reports whether another small desktop fits, leaving plenty for
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
