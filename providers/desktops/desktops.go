package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/jpeg"
	_ "image/png"
	"log"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

type desktops struct {
	dir   string // this provider's files
	node  string // its node's local API
	cache string // the desktops' tools, kept for when none is running

	mu    sync.Mutex
	tools []tool

	connMu sync.Mutex
	conns  map[string]*conn // by desktop

	poolMu sync.Mutex
	lent   map[string]loan      // borrowed desktops, by name
	used   map[string]time.Time // each desktop's last call
	take   sync.Mutex           // one borrow, or spare made, at a time
	kick   chan struct{}        // tend the pool now

	backupMu sync.Mutex
}

func newDesktops(dir, node string) *desktops {
	ds := &desktops{dir: dir, node: node, cache: filepath.Join(dir, "tools.json"), conns: map[string]*conn{}, used: map[string]time.Time{}, kick: make(chan struct{}, 1)}
	if b, err := os.ReadFile(ds.cache); err == nil {
		json.Unmarshal(b, &ds.tools)
	}
	ds.loadLoans()
	return ds
}

func (ds *desktops) describe() map[string]any {
	ops := ownOps()
	ds.mu.Lock()
	defer ds.mu.Unlock()
	for _, t := range ds.tools {
		if slices.Contains(browsing, t.Name) {
			ops = append(ops, map[string]any{"name": t.Name, "description": t.Description, "input": withDesktop(t.InputSchema)})
		}
	}
	return map[string]any{"title": "Desktops", "icon": "macwindow.on.rectangle", "color": "#a2845e", "instructions": instructions, "ops": ops, "widgets": []any{}, "screens": screens}
}

// withDesktop adds the desktop argument to a tool's input.
func withDesktop(schema json.RawMessage) map[string]any {
	var s map[string]any
	json.Unmarshal(schema, &s)
	if s == nil {
		s = map[string]any{"type": "object"}
	}
	props, _ := s["properties"].(map[string]any)
	if props == nil {
		props = map[string]any{}
	}
	props["desktop"] = desktopArg
	s["properties"] = props
	return s
}

// refreshIfEmpty reads the tools if it never has.
func (ds *desktops) refreshIfEmpty(ctx context.Context) {
	ds.mu.Lock()
	empty := len(ds.tools) == 0
	ds.mu.Unlock()
	if empty {
		ds.refresh(ctx)
	}
}

// refresh reads the tools from a running desktop: every desktop runs the
// same image, so any one will do. It gives how many there are.
func (ds *desktops) refresh(ctx context.Context) (int, error) {
	all, err := list(ctx)
	if err != nil {
		log.Print(err)
		return 0, err
	}
	err = errors.New("no desktop is running to read them from")
	for _, d := range all {
		if d.State != "running" {
			continue
		}
		var tools []tool
		var c *conn
		c, err = ds.conn(ctx, d)
		if err == nil {
			tools, err = c.tools(ctx)
		}
		if err != nil {
			log.Printf("reading %s's tools: %v", d.Name, err)
			continue
		}
		b, _ := json.Marshal(tools)
		ds.mu.Lock()
		ds.tools = tools
		ds.mu.Unlock()
		if err := os.WriteFile(ds.cache, b, 0o600); err != nil {
			log.Print(err)
		}
		return len(tools), nil
	}
	return 0, err
}

// conn is the desktop's session, made the first time it's needed. A browser
// an earlier session launched can't be used or closed by this one, and holds
// its profile, so those are closed first.
func (ds *desktops) conn(ctx context.Context, d desktop) (*conn, error) {
	ds.connMu.Lock()
	defer ds.connMu.Unlock()
	if c := ds.conns[d.Name]; c != nil {
		return c, nil
	}
	closeBrowsers(ctx, d, false)
	c, err := dial(ctx, d)
	if err != nil {
		return nil, err
	}
	ds.conns[d.Name] = c
	return c, nil
}

func (ds *desktops) drop(name string) {
	ds.connMu.Lock()
	delete(ds.conns, name)
	ds.connMu.Unlock()
}

// A connection's calls share a session label of its own unless they give
// theirs: the browser tools refuse to act without one, and a label stays
// bound to the connection that made it, so a new connection (as after this
// program restarted) can't take an old one over.

// call runs one of a desktop's tools, dialing again once if the connection is
// gone, as after the desktop restarted.
func (ds *desktops) call(ctx context.Context, d desktop, name string, args map[string]any) (result, error) {
	_, given := args["session"]
	ours := !given && ds.takesSession(name)
	var r result
	var err error
	for attempt := range 2 {
		if attempt > 0 {
			ds.drop(d.Name)
		}
		var c *conn
		if c, err = ds.conn(ctx, d); err != nil {
			continue
		}
		if ours {
			args["session"] = c.label
		}
		if r, err = c.call(ctx, name, args); err != nil {
			continue
		}
		if ours && said(r, "session_ended") {
			// It ended, idle; only start_session revives a name.
			if _, err = c.call(ctx, "start_session", map[string]any{"session": c.label}); err != nil {
				break
			}
			r, err = c.call(ctx, name, args)
		}
		if ours && said(r, "session_unavailable") && attempt == 0 {
			continue // a label of another connection's: dial again, with a new one
		}
		if name == "browser_prepare" && said(r, "browser_endpoint_owner_mismatch") {
			// The profile is open in a browser no session of ours owns.
			closeBrowsers(ctx, d, false)
			r, err = c.call(ctx, name, args)
		}
		break
	}
	return r, err
}

// said reports whether a tool's answer has code in it, in its structured
// content or its text.
func said(r result, code string) bool {
	if bytes.Contains(r.Structured, []byte(code)) {
		return true
	}
	for _, c := range r.Content {
		if t, _ := c["text"].(string); strings.Contains(t, code) {
			return true
		}
	}
	return false
}

func (ds *desktops) takesSession(name string) bool {
	ds.mu.Lock()
	defer ds.mu.Unlock()
	for _, t := range ds.tools {
		if t.Name == name {
			var s struct{ Properties map[string]json.RawMessage }
			json.Unmarshal(t.InputSchema, &s)
			_, ok := s.Properties["session"]
			return ok
		}
	}
	return false
}

// closeBrowsers closes the browsers the desktop's tools launched, and with
// signIns the one sign_in opened too, marking their profiles as closed
// normally, or they'd offer to restore their pages.
func closeBrowsers(ctx context.Context, d desktop, signIns bool) {
	keep := signInClass
	if signIns {
		keep = "--matches-no-browser"
	}
	err := sh(ctx, d, `profiles=`+profiles+`
main() { # each browser's own process, not its helpers
	for p in $(pgrep -f -- "[-]-user-data-dir=$profiles"); do
		c=$(tr '\0' ' ' 2>/dev/null </proc/$p/cmdline) || continue
		case "$c" in *--type=*|*`+keep+`*) ;; *) echo $p ;; esac
	done
}
pids=$(main)
[ -n "$pids" ] && kill $pids
for i in $(seq 50); do [ -z "$(main)" ] && break; sleep 0.1; done
sed -i 's/"exit_type":"Crashed"/"exit_type":"Normal"/' $profiles*/Default/Preferences 2>/dev/null
true`)
	if err != nil {
		log.Printf("closing %s's browsers: %v", d.Name, err)
	}
}

// content is a tool's answer for the node: its content, with its structured
// content as text too, since that often holds what its text leaves out.
func content(r result) (any, error) {
	items := r.Content
	if len(r.Structured) > 0 && string(r.Structured) != "null" {
		items = append(items, map[string]any{"type": "text", "text": string(r.Structured)})
	}
	if r.IsError {
		var texts []string
		for _, c := range items {
			if t, ok := c["text"].(string); ok {
				texts = append(texts, t)
			}
		}
		return nil, errors.New(strings.Join(texts, "\n"))
	}
	return map[string]any{"content": items}, nil
}

func (ds *desktops) run(ctx context.Context, op string, raw json.RawMessage) (any, error) {
	if op == "_screen" {
		return screenBody(raw)
	}
	in := map[string]any{}
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, fmt.Errorf("bad input: %w", err)
		}
	}
	// Browser profiles were called saved logins: agents and apps may still name them so.
	switch op {
	case "logins":
		op = "profiles"
	case "save_login":
		op = "save_profile"
	case "forget_login":
		op = "forget_profile"
	}
	if p, ok := in["profile"]; ok {
		in["login"] = p
		delete(in, "profile")
	}
	name, _ := in["desktop"].(string)
	delete(in, "desktop")
	text := func(k string) string { s, _ := in[k].(string); return strings.TrimSpace(s) }
	here, _ := in["here"].(bool)
	delete(in, "here")
	if name != "" && !here && !slices.Contains(local, op) {
		// A desktop on another server: pass the call on to it.
		if _, err := find(ctx, name); err != nil {
			if s := ds.located(ctx, name); s != "" {
				in["desktop"], in["here"] = name, true
				return ds.callNode(ctx, s, op, in)
			}
		}
	}
	switch op {
	case "list":
		if here {
			all, err := ds.listHere(ctx)
			return map[string]any{"desktops": all}, err
		}
		return ds.listAll(ctx)
	case "create":
		sz, err := sizeNamed(text("size"))
		if err != nil {
			return nil, err
		}
		d, err := create(ctx, name, sz, false, true)
		if err != nil {
			return nil, err
		}
		ds.touch(d.Name)
		ds.refreshIfEmpty(ctx)
		return fmt.Sprintf("Created desktop %s, %s. Its browser is yours to sign in to sites with: open it with view.", d.Name, sz), nil
	case "resize":
		sz, err := sizeNamed(text("size"))
		if err != nil {
			return nil, err
		}
		d, err := find(ctx, name)
		if err != nil {
			return nil, err
		}
		if err := resize(ctx, d, sz); err != nil {
			return nil, err
		}
		return fmt.Sprintf("Desktop %s is now %s.", d.Name, sz), nil
	case "start":
		d, err := find(ctx, name)
		if err != nil || d.State != "stopped" {
			if err == nil && d.State == "paused" {
				_, err = ds.running(ctx, d.Name)
			}
			return "It's running.", err
		}
		ds.drop(d.Name)
		if err := start(ctx, d); err != nil {
			return nil, err
		}
		ds.touch(d.Name)
		ds.refreshIfEmpty(ctx)
		return "Started.", nil
	case "stop":
		d, err := find(ctx, name)
		if err != nil {
			return nil, err
		}
		ds.drop(d.Name)
		return "Stopped. Its files and sign-ins stay.", stop(ctx, d)
	case "delete":
		d, err := find(ctx, name)
		if err != nil {
			return nil, err
		}
		ds.drop(d.Name)
		if err := remove(ctx, d); err != nil {
			return nil, err
		}
		if _, ok := ds.loans()[d.Name]; ok {
			ds.setLoan(d.Name, nil)
		}
		return fmt.Sprintf("Deleted desktop %s and everything in it.", d.Name), nil
	case "view":
		d, err := ds.running(ctx, name)
		if err != nil {
			return nil, err
		}
		minutes := 60
		if m, ok := in["minutes"].(float64); ok {
			minutes = min(max(int(m), 1), 1440)
		}
		url, err := viewURL(ctx, d, minutes)
		if err != nil {
			return nil, err
		}
		return fmt.Sprintf("%s\nThe link works for %d minutes, on the person's own devices.", url, minutes), nil
	case "screen":
		d, err := ds.running(ctx, name)
		if err != nil {
			return nil, err
		}
		width := 640
		if w, ok := in["width"].(float64); ok {
			width = min(max(int(w), 160), 1920)
		}
		return ds.screen(ctx, d, width)
	case "tools":
		return ds.list(ctx, name, in["name"])
	case "refresh_tools":
		n, err := ds.refresh(ctx)
		if err != nil {
			return nil, err
		}
		return fmt.Sprintf("Read %d tools from the desktops.", n), nil
	case "use":
		tool, _ := in["tool"].(string)
		args, _ := in["args"].(map[string]any)
		return ds.use(ctx, name, tool, args)
	case "borrow":
		why := text("for")
		if why == "" {
			return nil, errors.New("say what it's for, in a few words: the person sees it")
		}
		if !here && !ds.makesHere(ctx) {
			if r := ds.borrowElsewhere(ctx, op, in); r != nil {
				return r, nil
			}
		}
		login, given := in["login"].(string)
		if !given && ds.savedLogin("default") != "" {
			login = "default"
		}
		if r := ds.profileElsewhere(ctx, op, in, login, here); r != nil {
			return r, nil
		}
		d, err := ds.borrow(ctx, loan{By: cmpOr(text("by"), caller(ctx)), For: why, Login: login})
		if err != nil {
			return nil, err
		}
		me, _, _ := ds.servers(ctx)
		msg := fmt.Sprintf("Borrowed desktop %s: pass desktop %q to the desktops tools.", d.Name, d.Name)
		if login != "" {
			msg += fmt.Sprintf(` Its browser starts with a copy of the %s profile: desktops_browser_prepare with allow_launch true and profile {"mode":"isolated_named","name":%q}.`, login, login)
		}
		msg += " Give it back with give_back when done; it's given back for you after 15 minutes without a call."
		return map[string]any{"desktop": d.Name, "server": me, "profile": login, "login": login, "text": msg}, nil
	case "give_back":
		if name == "" {
			return nil, errors.New("which desktop? pass the one you borrowed")
		}
		l, ok := ds.loans()[name]
		if err := ds.giveBack(ctx, name, "given back by "+caller(ctx)); err != nil {
			return nil, err
		}
		if ok && l.SigningIn {
			return fmt.Sprintf("Saved the %s profile and closed desktop %s.", l.Login, name), nil
		}
		return fmt.Sprintf("Gave back desktop %s.", name), nil
	case "sign_in":
		if !here && !ds.makesHere(ctx) {
			if r := ds.borrowElsewhere(ctx, op, in); r != nil {
				return r, nil
			}
		}
		login := cmpOr(text("login"), "default")
		if !validProfile.MatchString(login) {
			return nil, errors.New("a profile's name is letters, digits and dashes, like Personal or PTC")
		}
		if r := ds.profileElsewhere(ctx, op, in, login, here); r != nil {
			return r, nil
		}
		minutes := int(signInFor.Minutes())
		if m, ok := in["minutes"].(float64); ok {
			minutes = min(max(int(m), 1), 240)
		}
		d, until, err := ds.signIn(ctx, login, text("url"), minutes, cmpOr(text("by"), caller(ctx)))
		if err != nil {
			return nil, err
		}
		url, err := viewURL(ctx, d, minutes)
		if err != nil {
			return nil, err
		}
		msg := fmt.Sprintf("Sign in to sites in the %s profile here:\n%s\nIt's saved in %d minutes, or as soon as desktop %s is given back (give_back) or saved (save_profile).", login, url, minutes, d.Name)
		me, _, _ := ds.servers(ctx)
		return map[string]any{"desktop": d.Name, "server": me, "profile": login, "login": login, "url": url, "until": until, "text": msg}, nil
	case "save_profile":
		d, err := ds.running(ctx, name)
		if err != nil {
			return nil, err
		}
		l, lent := ds.loans()[d.Name]
		login := cmpOr(text("login"), cmpOr(l.Login, "default"))
		if lent && l.SigningIn && login == l.Login {
			if err := ds.giveBack(ctx, d.Name, "signed in"); err != nil {
				return nil, err
			}
			return fmt.Sprintf("Saved the %s profile and closed desktop %s.", login, d.Name), nil
		}
		if err := ds.saveLogin(ctx, d, login); err != nil {
			return nil, err
		}
		return fmt.Sprintf("Saved desktop %s's browser as the %s profile: desktops borrowed with it start signed in to the same sites.", d.Name, login), nil
	case "profiles":
		me, _, err := ds.servers(ctx)
		if err != nil {
			return nil, err
		}
		mine := ds.logins(me)
		if here {
			return map[string]any{"profiles": mine}, nil
		}
		got, failed := ds.everywhere(ctx, "profiles", nil)
		for _, r := range got {
			var theirs struct{ Profiles []map[string]any }
			json.Unmarshal(r, &theirs)
			mine = append(mine, theirs.Profiles...)
		}
		out := map[string]any{"profiles": mine}
		if len(failed) > 0 {
			out["unreachable"] = failed
		}
		return out, nil
	case "forget_profile":
		login := text("login")
		if r := ds.profileElsewhere(ctx, op, in, login, here); r != nil {
			return r, nil
		}
		if !validProfile.MatchString(login) || ds.savedLogin(login) == "" {
			return nil, fmt.Errorf("no profile %q", login)
		}
		if err := os.RemoveAll(ds.loginDir(login)); err != nil {
			return nil, err
		}
		return fmt.Sprintf("Deleted the %s profile.", login), nil
	case "backup":
		d, err := ds.running(ctx, name)
		if err != nil {
			return nil, err
		}
		rec, err := ds.backup(ctx, d)
		if err != nil {
			return nil, err
		}
		return reply{ok: map[string]any{"backup": rec, "text": fmt.Sprintf("Backed up desktop %s (%s). Restore it on any server with restore.", d.Name, bytesText(rec.Bytes))},
			entries: []map[string]any{{"kind": "backup", "data": rec, "blobs": []string{rec.Blob}}}}, nil
	case "backups":
		if here {
			return map[string]any{"backups": ds.backupsHere()}, nil
		}
		all, failed := ds.backupsAll(ctx, name)
		out := map[string]any{"backups": all}
		if len(failed) > 0 {
			out["unreachable"] = failed
		}
		return out, nil
	case "restore":
		rec, err := ds.findBackup(ctx, name, text("blob"))
		if err != nil {
			return nil, err
		}
		as := cmpOr(text("as"), cmpOr(name, rec.Desktop))
		sz, err := sizeNamed(cmpOr(text("size"), rec.Size))
		if err != nil {
			sz = sizes[0]
		}
		d, err := ds.restore(ctx, as, rec, sz)
		if err != nil {
			return nil, err
		}
		ds.touch(d.Name)
		return fmt.Sprintf("Restored desktop %s from its backup of %s.", d.Name, rec.At.Local().Format(time.DateTime)), nil
	case "move":
		to := text("to")
		if to == "" {
			return nil, errors.New("move it to which server? pass to")
		}
		d, err := ds.running(ctx, name)
		if err != nil {
			return nil, err
		}
		if _, ok := ds.loans()[d.Name]; ok {
			return nil, errors.New("a borrowed desktop can't be moved: borrow one on the other server")
		}
		closeBrowsers(ctx, d, true)
		// Backed up through the node, so the entry naming the backup's blob is
		// written before the other server asks for it.
		me, _, err := ds.servers(ctx)
		if err != nil {
			return nil, err
		}
		raw, err := ds.callNode(ctx, me, "backup", map[string]any{"desktop": d.Name, "here": true})
		if err != nil {
			return nil, err
		}
		var backed struct{ Backup backupRecord }
		if err := json.Unmarshal(raw, &backed); err != nil || backed.Backup.Blob == "" {
			return nil, fmt.Errorf("backing it up: %s", raw)
		}
		rec := backed.Backup
		if _, err := ds.callNode(ctx, to, "restore", map[string]any{"desktop": d.Name, "blob": rec.Blob, "size": rec.Size}); err != nil {
			return nil, fmt.Errorf("not moved: restoring it on %s: %w", to, err)
		}
		ds.drop(d.Name)
		if err := stop(ctx, d); err != nil {
			return nil, fmt.Errorf("moved to %s, but stopping it here failed: %w", to, err)
		}
		return fmt.Sprintf("Moved desktop %s to %s, where it's running. It's stopped here; delete it here (delete) once it's fine there.", d.Name, to), nil
	}
	if slices.Contains(browsing, op) {
		return ds.use(ctx, name, op, in)
	}
	return nil, fmt.Errorf("no operation %s", op)
}

func (ds *desktops) use(ctx context.Context, name, tool string, args map[string]any) (any, error) {
	if tool == "" {
		return nil, errors.New("which tool? desktops_tools lists them")
	}
	d, err := ds.running(ctx, name)
	if err != nil {
		return nil, err
	}
	if args == nil {
		args = map[string]any{}
	}
	r, err := ds.call(ctx, d, tool, args)
	if err != nil {
		return nil, err
	}
	return content(r)
}

// list is the desktop's tools other than the browser's, or one in full.
func (ds *desktops) list(ctx context.Context, desktop string, name any) (any, error) {
	ds.mu.Lock()
	tools := ds.tools
	ds.mu.Unlock()
	if len(tools) == 0 {
		if _, err := ds.running(ctx, desktop); err != nil {
			return nil, err
		}
		ds.refresh(ctx)
		ds.mu.Lock()
		tools = ds.tools
		ds.mu.Unlock()
	}
	if n, _ := name.(string); n != "" {
		for _, t := range tools {
			if t.Name == n {
				return t.Description + "\n\nInput: " + string(t.InputSchema), nil
			}
		}
		return nil, fmt.Errorf("no tool %s", n)
	}
	var b strings.Builder
	for _, t := range tools {
		if !slices.Contains(browsing, t.Name) {
			fmt.Fprintf(&b, "%s: %s\n", t.Name, gist(t.Description))
		}
	}
	return strings.TrimSpace(b.String()), nil
}

// gist is a description's first sentence.
func gist(s string) string {
	s, _, _ = strings.Cut(s, "\n")
	if i := strings.Index(s, ". "); i >= 0 {
		s = s[:i+1]
	}
	return s
}

// screen is a small JPEG of the desktop.
func (ds *desktops) screen(ctx context.Context, d desktop, width int) (any, error) {
	r, err := ds.call(ctx, d, "get_desktop_state", map[string]any{"max_image_dimension": width})
	if err != nil {
		return nil, err
	}
	for _, c := range r.Content {
		data, _ := c["data"].(string)
		if c["type"] != "image" || data == "" {
			continue
		}
		raw, err := base64.StdEncoding.DecodeString(data)
		if err != nil {
			return nil, err
		}
		img, _, err := image.Decode(bytes.NewReader(raw))
		if err != nil {
			return nil, err
		}
		var b bytes.Buffer
		if err := jpeg.Encode(&b, img, &jpeg.Options{Quality: 70}); err != nil {
			return nil, err
		}
		return map[string]any{"content": []map[string]any{
			{"type": "image", "mimeType": "image/jpeg", "data": base64.StdEncoding.EncodeToString(b.Bytes())},
			{"type": "text", "text": "Desktop " + d.Name + " now."},
		}}, nil
	}
	return nil, errors.New("the desktop sent no picture")
}

// viewURL is a link to the desktop's viewer that works for minutes, served
// over HTTPS on the tailnet: browsers only play its video on secure pages.
func viewURL(ctx context.Context, d desktop, minutes int) (string, error) {
	port := strconv.Itoa(d.port)
	if _, err := tailscale(ctx, "serve", "--bg", "--https="+port, "http://127.0.0.1:"+port); err != nil {
		return "", err
	}
	out, err := tailscale(ctx, "status", "--json")
	if err != nil {
		return "", err
	}
	var st struct{ Self struct{ DNSName string } }
	if err := json.Unmarshal([]byte(out), &st); err != nil {
		return "", err
	}
	path, err := viewerPath(ctx, d, minutes*60)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("https://%s:%s%s", strings.TrimSuffix(st.Self.DNSName, "."), port, path), nil
}

// local are the operations that act on this server even when given a
// desktop it doesn't have.
var local = []string{"list", "create", "borrow", "sign_in", "profiles", "forget_profile", "backups", "restore", "refresh_tools"}

// reply is a result with entries for the node to write in the caller's log.
type reply struct {
	ok      any
	entries []map[string]any
}

type callerKey struct{}

// caller is the name of the device the call came from.
func caller(ctx context.Context) string {
	name, _ := ctx.Value(callerKey{}).(string)
	return cmpOr(name, "someone")
}

func bytesText(n int64) string {
	switch {
	case n >= 1<<30:
		return fmt.Sprintf("%.1f GB", float64(n)/(1<<30))
	case n >= 1<<20:
		return fmt.Sprintf("%.0f MB", float64(n)/(1<<20))
	}
	return fmt.Sprintf("%d KB", n>>10)
}
