package spaces

import (
	"bufio"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	goruntime "runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"text/tabwriter"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// Logins are browser profiles kept in the bucket. The user signs in to
// sites once, in a Space of their own (sign_in); any Space an agent borrows
// then starts with a copy. Agents' copies are thrown away with their Space,
// so only sign_in changes a login.
//
// Borrowed Spaces come from a pool: one free Space is kept running, on the
// device with the most room, as long as one has room to spare for it. A
// borrowed Space is deleted when given back or left idle, so agents never
// share one and none are left running.

const (
	loginFiles  = "@logins/" // @ isn't in Space names, so it isn't a Space's backups
	profiles    = home + "/.local/state/cua-driver/browser-profiles/"
	poolPrefix  = "pool-"
	idleFor     = 15 * time.Minute
	spaceMemory = 4 << 30 // what a Space may use; see the runtimes
)

type borrowed struct {
	Space  string    `json:"space"`
	Login  string    `json:"login,omitempty"`
	SignIn bool      `json:"signIn,omitempty"`
	Until  time.Time `json:"until,omitzero"` // a sign-in's end
	Agent  string    `json:"agent,omitempty"`
	For    string    `json:"for,omitempty"`
	Since  time.Time `json:"since,omitzero"`
}

type borrowIn struct {
	For   string `json:"for" jsonschema:"what you need it for, in a few words, e.g. Syncing X bookmarks; shown to the user"`
	Login string `json:"login,omitempty" jsonschema:"the saved login to start the browser with, e.g. default; none for a browser signed in to nothing"`
	Here  bool   `json:"here,omitempty" jsonschema:"only a Space on this device"`
}

type signInIn struct {
	Login   string `json:"login,omitempty" jsonschema:"the login to sign in to sites in; default is default, the one for everything"`
	Minutes int    `json:"minutes,omitempty" jsonschema:"how long until it is saved and closed; default 30, at most 240"`
	Here    bool   `json:"here,omitempty" jsonschema:"only a Space on this device"`
}

type giveBackIn struct {
	Space string `json:"space" jsonschema:"the borrowed Space"`
}

type loginIn struct {
	Login string `json:"login" jsonschema:"the login's name"`
}

type pool struct {
	t       *tools
	located func(string) string
	locate  func(*sdk.Call)

	mu    sync.Mutex
	lent  map[string]borrowed
	used  map[string]time.Time
	rooms []room // every device's, as last asked
	asked time.Time
	kick  chan struct{}
	take  sync.Mutex // one borrow at a time on this device
}

func setupPool(c *sdk.Ctx, t *tools, located func(string) string, locate func(*sdk.Call)) *pool {
	p := &pool{t: t, located: located, locate: locate, lent: map[string]borrowed{}, used: map[string]time.Time{}, kick: make(chan struct{}, 1)}
	t.used = p.touch
	c.Entry("borrowed", sdk.Available)
	c.Entry("returned", sdk.Available)
	c.Entry("spare", sdk.Available) // no longer written
	c.OnReset(func() { p.mu.Lock(); clear(p.lent); p.mu.Unlock() })
	sdk.Handle(c, "borrowed", func(_ sdk.Entry, in borrowed) error { p.mu.Lock(); p.lent[in.Space] = in; p.mu.Unlock(); return nil })
	sdk.Handle(c, "returned", func(_ sdk.Entry, in released) error { p.mu.Lock(); delete(p.lent, in.Name); p.mu.Unlock(); return nil })
	c.OnReady(func(call *sdk.Call) { go p.keep(call.Detached()) })
	sdk.Op(c, "room", "How much room this device has for Spaces.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		r, err := here(call)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: fmt.Sprintf("%s: %s free of %s", r.Host, size(r.Free), size(r.Total)), Data: r}, nil
	})

	sdk.Op(c, "borrow", "Borrow a Space of your own, for as long as you need a browser or desktop, starting its browser with a saved login. Give it back with give_back when done; one left idle for 15 minutes is given back for you, and anything in it is lost.", sdk.Write, func(call *sdk.Call, in borrowIn) (sdk.Result, error) {
		if strings.TrimSpace(in.For) == "" {
			return sdk.Result{}, errors.New("say what it's for, in a few words: the user sees it")
		}
		b, fwd, err := p.borrow(call, "borrow", in, borrowed{Login: in.Login, Agent: call.Caller.Agent, For: strings.TrimSpace(in.For)})
		if err != nil || fwd != nil {
			return deref(fwd), err
		}
		text := fmt.Sprintf("Borrowed Space %q: pass space %q to the Space tools.", b.Space, b.Space)
		if b.Login != "" {
			text += fmt.Sprintf(` Its browser is signed in as the %s login: call browser_prepare with allow_launch true and profile {"mode":"isolated_named","name":%q}.`, b.Login, b.Login)
		}
		return sdk.Result{Text: text + " Give it back with give_back when done.", Data: b}, nil
	})
	sdk.Op(c, "sign_in", "Sign in to sites, for agents to use: opens a browser with a saved login (or a new one) in a Space of its own and gives a link to it. Sign in to as many sites as you like; the login is saved when given back with give_back, or when the time is up.", sdk.Write, func(call *sdk.Call, in signInIn) (sdk.Result, error) {
		login := cmp(in.Login, "default")
		if !validName.MatchString(login) {
			return sdk.Result{}, errors.New("a login's name is lowercase letters, digits and dashes")
		}
		minutes := min(max(in.Minutes, 1), 240)
		if in.Minutes == 0 {
			minutes = 30
		}
		b, fwd, err := p.borrow(call, "sign_in", borrowIn{Login: login, Here: in.Here}, borrowed{Login: login, SignIn: true, For: "Signing in to sites", Until: time.Now().Add(time.Duration(minutes) * time.Minute)})
		if err != nil || fwd != nil {
			return deref(fwd), err
		}
		s, err := find(call.Context, b.Space)
		if err != nil {
			return sdk.Result{}, err
		}
		url, err := viewURL(call.Context, s, minutes)
		if err != nil {
			return sdk.Result{}, err
		}
		text := fmt.Sprintf("Sign in to sites in the %s login here:\n%s\nIt is saved in %d minutes, or as soon as you give back Space %q (give_back).", login, url, minutes, b.Space)
		return sdk.Result{Text: text, Data: map[string]any{"space": b.Space, "login": login, "url": url, "until": b.Until}}, nil
	})
	sdk.Op(c, "give_back", "Give back a borrowed Space: it is deleted with everything in it. A sign_in Space's login is saved first.", sdk.Write, func(call *sdk.Call, in giveBackIn) (sdk.Result, error) {
		if res, ok, err := t.elsewhere(call, in.Space, "give_back", in); ok {
			return res, err
		}
		p.mu.Lock()
		b, ok := p.lent[in.Space]
		p.mu.Unlock()
		if !ok {
			return sdk.Result{}, fmt.Errorf("Space %q isn't borrowed", in.Space)
		}
		if err := p.giveBack(call, b); err != nil {
			return sdk.Result{}, err
		}
		if b.SignIn {
			return sdk.Text(fmt.Sprintf("Saved the %s login and gave back Space %q.", b.Login, b.Space)), nil
		}
		return sdk.Text(fmt.Sprintf("Gave back Space %q.", b.Space)), nil
	})
	sdk.Op(c, "logins", "List the saved browser logins agents can borrow a Space with.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		files, err := call.Files(loginFiles)
		if err != nil {
			return sdk.Result{}, err
		}
		latest := map[string]sdk.FileInfo{}
		for _, f := range files {
			name, _, _ := strings.Cut(strings.TrimPrefix(f.Name, loginFiles), "/")
			if f.Name > latest[name].Name {
				latest[name] = f
			}
		}
		if len(latest) == 0 {
			return sdk.Result{Text: "No logins. Sign in to sites with sign_in.", Data: []string{}}, nil
		}
		names := slices.Sorted(func(yield func(string) bool) {
			for n := range latest {
				if !yield(n) {
					return
				}
			}
		})
		var b strings.Builder
		w := tabwriter.NewWriter(&b, 0, 0, 2, ' ', 0)
		for _, n := range names {
			fmt.Fprintf(w, "%s\tsaved %s\t%s\n", n, latest[n].Modified.Local().Format(time.DateTime), size(latest[n].Size))
		}
		w.Flush()
		return sdk.Result{Text: strings.TrimSpace(b.String()), Data: names}, nil
	})
	sdk.Op(c, "forget_login", "Delete a saved login, signing agents out of every site in it.", sdk.Write, func(call *sdk.Call, in loginIn) (sdk.Result, error) {
		saved, err := savedLogins(call, in.Login)
		if err != nil {
			return sdk.Result{}, err
		}
		if len(saved) == 0 {
			return sdk.Result{}, fmt.Errorf("no login named %q", in.Login)
		}
		for _, f := range saved {
			if err := call.DeleteFile(f.Name); err != nil {
				return sdk.Result{}, err
			}
		}
		return sdk.Text(fmt.Sprintf("Forgot the %s login.", in.Login)), nil
	})
	return p
}

// status says what a pool's Space is doing, for list.
func (p *pool) status(name string) string {
	if !strings.HasPrefix(name, poolPrefix) {
		return ""
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	b, ok := p.lent[name]
	switch {
	case !ok:
		return "free"
	case b.SignIn:
		return "signing in to the " + b.Login + " login"
	}
	who := cmp(b.Agent, "an agent")
	return fmt.Sprintf("%s: %s", who, b.For)
}

func deref(r *sdk.Result) sdk.Result {
	if r == nil {
		return sdk.Result{}
	}
	return *r
}

func cmp(s, def string) string {
	if s == "" {
		return def
	}
	return s
}

func (p *pool) touch(name string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if _, ok := p.lent[name]; ok {
		p.used[name] = time.Now()
	}
}

func (p *pool) poke() {
	select {
	case p.kick <- struct{}{}:
	default:
	}
}

// spares are the pool's Spaces on this device that nobody has borrowed.
func (p *pool) spares(spaces []space) []space {
	p.mu.Lock()
	defer p.mu.Unlock()
	var out []space
	for _, s := range spaces {
		if _, lent := p.lent[s.Name]; strings.HasPrefix(s.Name, poolPrefix) && !lent && s.State == "running" {
			out = append(out, s)
		}
	}
	return out
}

// borrow lends a spare Space, or one made for it, on this device; without
// in.Here it prefers a spare on another device to making one, and passes
// op on to it, answering with its result.
func (p *pool) borrow(call *sdk.Call, op string, in borrowIn, b borrowed) (borrowed, *sdk.Result, error) {
	if b.Login != "" && !validName.MatchString(b.Login) {
		return b, nil, fmt.Errorf("no login named %q", b.Login)
	}
	if b.Login != "" {
		saved, err := savedLogins(call, b.Login)
		if err != nil {
			return b, nil, err
		}
		if len(saved) == 0 && !b.SignIn {
			return b, nil, fmt.Errorf("no login named %q: sign in to sites in one with sign_in", b.Login)
		}
	}
	p.take.Lock()
	defer p.take.Unlock()
	spaces, err := list(call.Context)
	if err != nil {
		return b, nil, err
	}
	ready := p.spares(spaces)
	if len(ready) == 0 && !in.Here {
		if host := p.elsewhere(call); host != "" {
			args := map[string]any{"for": in.For, "login": in.Login, "here": true, "device": strings.Split(host, ".")[0]}
			if op == "sign_in" {
				args["minutes"] = int(time.Until(b.Until).Round(time.Minute).Minutes())
			}
			res, err := call.Invoke("spaces", op, args)
			if err == nil {
				return b, &res, nil
			}
			call.Logf("spaces: borrowing on %s: %v", host, err)
		}
	}
	var s space
	if len(ready) > 0 {
		s = ready[0]
	} else if s, err = p.make(call); err != nil {
		return b, nil, err
	}
	b.Space = s.Name
	b.Since = time.Now().UTC()
	if err := p.prepare(call, s, b); err != nil {
		_ = p.drop(call, s)
		return b, nil, err
	}
	p.mu.Lock()
	p.used[s.Name] = time.Now()
	p.mu.Unlock()
	if _, err := call.Append("borrowed", b); err != nil {
		return b, nil, err
	}
	p.poke()
	if !b.SignIn {
		b.Login = in.Login
	}
	return b, nil, nil
}

// elsewhere is another device to borrow on: one with a free Space, or else
// the one with the most room, if that isn't this one.
func (p *pool) elsewhere(call *sdk.Call) string {
	me, _ := tailnetName(call.Context)
	if h := p.freeOn(call, me); h != "" {
		return h
	}
	if best := p.best(call); best != nil && best.Host != me {
		return best.Host
	}
	return ""
}

// freeOn is a device other than me with a free Space.
func (p *pool) freeOn(call *sdk.Call, me string) string {
	names := p.t.names()
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, name := range names {
		if _, lent := p.lent[name]; strings.HasPrefix(name, poolPrefix) && !lent {
			if h := p.located(name); h != "" && h != me {
				return h
			}
		}
	}
	return ""
}

func (p *pool) make(call *sdk.Call) (space, error) {
	id := make([]byte, 3)
	_, _ = rand.Read(id)
	s, err := create(call.Context, poolPrefix+hex.EncodeToString(id))
	if err != nil {
		return space{}, err
	}
	p.locate(call)
	_ = p.t.refresh(call.Context)
	return s, nil
}

func (p *pool) drop(call *sdk.Call, s space) error {
	p.t.drop(s.Name)
	err := remove(call.Context, s)
	p.locate(call)
	_ = p.t.refresh(call.Context)
	return err
}

// prepare puts the login in the Space's browser profiles; for a sign-in it
// also opens a browser with it, as the desktop's user.
func (p *pool) prepare(call *sdk.Call, s space, b borrowed) error {
	if b.Login == "" {
		return nil
	}
	saved, err := savedLogins(call, b.Login)
	if err != nil {
		return err
	}
	dir := profiles + b.Login
	// The browser tools only open a profile they made, as this marks.
	owned := fmt.Sprintf(`mkdir -p "$1" && printf '%%s' '{"schema":"cua-driver-browser-profile-v1","mode":"isolated_named","name":%q}' > "$1/.cua-driver-owned-profile.json" && chmod 700 "$1" && chown -R cua:cua %s/.local`, b.Login, home)
	if len(saved) > 0 {
		tmp, err := os.CreateTemp("", "one-login-*.tar.gz")
		if err != nil {
			return err
		}
		tmp.Close()
		defer os.Remove(tmp.Name())
		if err := call.GetFile(saved[0].Name, tmp.Name()); err != nil {
			return err
		}
		in, err := os.Open(tmp.Name())
		if err != nil {
			return err
		}
		defer in.Close()
		if err := rt.exec(call.Context, s.Name, in, nil, "sh", "-c", `mkdir -p "$1" && tar -C "$1" -xzf - && `+owned, "sh", dir); err != nil {
			return fmt.Errorf("unpacking the %s login: %w", b.Login, err)
		}
	}
	if !b.SignIn {
		return nil
	}
	return rt.exec(call.Context, s.Name, nil, nil, "sh", "-c",
		owned+` && runuser -u cua -- env DISPLAY=:1 HOME=`+home+` setsid chromium --user-data-dir="$1" --password-store=basic --no-first-run --start-maximized >/dev/null 2>&1 </dev/null &`, "sh", dir)
}

// giveBack saves a sign-in's login and deletes the Space.
func (p *pool) giveBack(call *sdk.Call, b borrowed) error {
	s, err := find(call.Context, b.Space)
	if err == nil && b.SignIn {
		if err := saveLogin(call, s, b.Login); err != nil {
			return fmt.Errorf("not given back: saving the %s login failed: %w", b.Login, err)
		}
	}
	if err == nil {
		if err := p.drop(call, s); err != nil {
			return err
		}
	}
	if _, err := call.Append("returned", released{Name: b.Space}); err != nil {
		return err
	}
	p.mu.Lock()
	delete(p.used, b.Space)
	p.mu.Unlock()
	p.poke()
	return nil
}

func savedLogins(call *sdk.Call, login string) ([]sdk.FileInfo, error) {
	files, err := call.Files(loginFiles + login + "/")
	if err != nil {
		return nil, err
	}
	slices.SortFunc(files, func(a, b sdk.FileInfo) int { return strings.Compare(b.Name, a.Name) })
	return files, nil
}

// saveLogin closes the browsers in the Space, so their profiles are
// written out, and stores the login's profile, keeping the last few.
func saveLogin(call *sdk.Call, s space, login string) error {
	closeBrowsers(call.Context, s)
	tmp, err := os.CreateTemp("", "one-login-*.tar.gz")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	args := []string{"tar", "-C", profiles + login, "--warning=no-file-changed"}
	for _, c := range caches {
		args = append(args, "--exclude="+c)
	}
	err = rt.exec(call.Context, s.Name, nil, tmp, append(args, "-czf", "-", ".")...)
	tmp.Close()
	var exit *exec.ExitError
	if err != nil && !(errors.As(err, &exit) && exit.ExitCode() == 1) {
		return err
	}
	name := fmt.Sprintf("%s%s/%s.tar.gz", loginFiles, login, time.Now().UTC().Format("20060102T150405Z"))
	if err := call.PutFile(name, tmp.Name()); err != nil {
		return err
	}
	all, err := savedLogins(call, login)
	if err != nil {
		return err
	}
	for _, old := range all[min(keepBackups, len(all)):] {
		if err := call.DeleteFile(old.Name); err != nil {
			call.Logf("dropping old login %s: %v", old.Name, err)
		}
	}
	return nil
}

// keep gives back this device's borrowed Spaces when idle or their time is
// up, and keeps its spares, for as long as the plugin runs.
func (p *pool) keep(call *sdk.Call) {
	for {
		p.tend(call)
		select {
		case <-call.Context.Done():
			return
		case <-p.kick:
		case <-time.After(30 * time.Second):
		}
	}
}

func (p *pool) tend(call *sdk.Call) {
	me, err := tailnetName(call.Context)
	if err != nil {
		return
	}
	spaces, err := list(call.Context)
	if err != nil {
		return
	}
	here := map[string]bool{}
	for _, s := range spaces {
		here[s.Name] = true
	}
	p.mu.Lock()
	var due []borrowed
	for name, b := range p.lent {
		h := p.located(name)
		switch {
		case here[name]:
			if _, ok := p.used[name]; !ok {
				p.used[name] = time.Now() // since this plugin started
			}
			if b.SignIn && time.Now().After(b.Until) || !b.SignIn && time.Since(p.used[name]) > idleFor {
				due = append(due, b)
			}
		case h == "" || h == me: // gone, as with its device
			due = append(due, b)
		}
	}
	p.mu.Unlock()
	for _, b := range due {
		if err := p.giveBack(call, b); err != nil {
			call.Logf("spaces: giving back %s: %v", b.Space, err)
		}
	}
	p.take.Lock()
	defer p.take.Unlock()
	if spaces, err = list(call.Context); err != nil {
		return
	}
	for _, s := range spaces {
		if strings.HasPrefix(s.Name, poolPrefix) && s.State != "running" && !p.isLent(s.Name) {
			_ = p.drop(call, s)
		}
	}
	// One free Space in all: the one whose name sorts first stays. The
	// device with the most room makes one when there is none.
	ready := p.spares(spaces)
	free := p.free()
	for _, s := range ready {
		if len(free) > 0 && s.Name != free[0] {
			_ = p.drop(call, s)
		}
	}
	if len(free) == 0 {
		if best := p.best(call); best != nil && best.Host == me {
			if _, err := p.make(call); err != nil {
				call.Logf("spaces: making a free Space: %v", err)
			}
		}
	}
}

// free are the names of every device's free Spaces, sorted.
func (p *pool) free() []string {
	names := p.t.names()
	p.mu.Lock()
	defer p.mu.Unlock()
	var out []string
	for _, n := range names {
		if _, lent := p.lent[n]; strings.HasPrefix(n, poolPrefix) && !lent {
			out = append(out, n)
		}
	}
	slices.Sort(out)
	return out
}

func (p *pool) isLent(name string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	_, ok := p.lent[name]
	return ok
}

// A room is a device's room for Spaces.
type room struct {
	Host   string `json:"host"`
	Free   int64  `json:"free"`             // memory, in bytes
	Total  int64  `json:"total"`            // memory, in bytes
	Nested bool   `json:"nested,omitempty"` // in a container, sharing a host's memory
}

// fits reports whether another Space fits, leaving plenty for everything
// else: a third of the memory, and at least 6 GB.
func (r room) fits() bool {
	return r.Total > 0 && r.Free-spaceMemory >= max(r.Total*3/10, 6<<30)
}

func here(call *sdk.Call) (room, error) {
	me, err := tailnetName(call.Context)
	if err != nil {
		return room{}, err
	}
	if _, err := list(call.Context); err != nil {
		return room{}, err
	}
	free, total := memory()
	out, _ := exec.Command("systemd-detect-virt", "--container").Output()
	v := strings.TrimSpace(string(out))
	return room{Host: me, Free: free, Total: total, Nested: v != "" && v != "none"}, nil
}

// roomsAll asks every online device that runs Spaces for its room, at most
// every 20 seconds.
func (p *pool) roomsAll(call *sdk.Call) []room {
	p.mu.Lock()
	if time.Since(p.asked) < 20*time.Second {
		defer p.mu.Unlock()
		return p.rooms
	}
	p.mu.Unlock()
	var rooms []room
	if r, err := here(call); err == nil {
		rooms = append(rooms, r)
	}
	res, err := call.Invoke("one", "devices", map[string]any{})
	if err == nil {
		raw, _ := json.Marshal(res.Data)
		var d struct {
			Devices []struct {
				Name   string `json:"name"`
				Self   bool   `json:"self"`
				Online bool   `json:"online"`
			} `json:"devices"`
		}
		_ = json.Unmarshal(raw, &d)
		var wg sync.WaitGroup
		var mu sync.Mutex
		for _, dev := range d.Devices {
			if dev.Self || !dev.Online {
				continue
			}
			wg.Go(func() {
				res, err := call.Invoke("spaces", "room", map[string]any{"device": dev.Name})
				if err != nil {
					return
				}
				raw, _ := json.Marshal(res.Data)
				var r room
				if json.Unmarshal(raw, &r) == nil && r.Host != "" {
					mu.Lock()
					rooms = append(rooms, r)
					mu.Unlock()
				}
			})
		}
		wg.Wait()
	}
	slices.SortFunc(rooms, func(a, b room) int { return strings.Compare(a.Host, b.Host) })
	p.mu.Lock()
	p.rooms, p.asked = rooms, time.Now()
	p.mu.Unlock()
	return rooms
}

// best is the device a new Space should go on: one it fits on, not inside
// another device, with the most free memory. Nil when it fits nowhere.
func (p *pool) best(call *sdk.Call) *room {
	var best *room
	for _, r := range p.roomsAll(call) {
		if !r.fits() {
			continue
		}
		if best == nil || best.Nested && !r.Nested || best.Nested == r.Nested && r.Free > best.Free {
			best = &r
		}
	}
	return best
}

// memory is this device's free and total memory, in bytes; 0 if unknown.
func memory() (free, total int64) {
	if goruntime.GOOS == "darwin" {
		out, err := exec.Command("sysctl", "-n", "hw.memsize").Output()
		if err != nil {
			return 0, 0
		}
		total, _ = strconv.ParseInt(strings.TrimSpace(string(out)), 10, 64)
		out, err = exec.Command("vm_stat").Output()
		if err != nil {
			return 0, 0
		}
		page := int64(16384)
		for line := range strings.Lines(string(out)) {
			if _, rest, ok := strings.Cut(line, "page size of "); ok {
				page, _ = strconv.ParseInt(strings.Fields(rest)[0], 10, 64)
			}
			for _, k := range []string{"Pages free:", "Pages inactive:", "Pages speculative:"} {
				if v, ok := strings.CutPrefix(line, k); ok {
					n, _ := strconv.ParseInt(strings.TrimSuffix(strings.TrimSpace(v), "."), 10, 64)
					free += n * page
				}
			}
		}
		return free, total
	}
	f, err := os.Open("/proc/meminfo")
	if err != nil {
		return 0, 0
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		k, v, _ := strings.Cut(sc.Text(), ":")
		n, _ := strconv.ParseInt(strings.TrimSuffix(strings.TrimSpace(v), " kB"), 10, 64)
		switch k {
		case "MemAvailable":
			free = n << 10
		case "MemTotal":
			total = n << 10
		}
	}
	return free, total
}
