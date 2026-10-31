// The Desktops provider: Linux desktops for agents, each a Podman container
// running Cua's desktop image (github.com/trycua/cua) with a browser that
// keeps its sign-ins, offered to the person's devices through this device's
// node (SPEC.md, "Local API"). The server inside each desktop speaks MCP; its
// browser tools become this app's operations.
package main

import (
	"bytes"
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"runtime/debug"
	"sync/atomic"
	"time"
)

const app = "desktops"

type call struct {
	ID     uint64          `json:"id"`
	Op     string          `json:"op"`
	Input  json.RawMessage `json:"input"`
	Caller struct {
		Name string `json:"name"` // the device it came from
	} `json:"caller"`
}

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("DESKTOPS_DATA"), filepath.Join(home, ".local", "share", "one", "desktops"))
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	if err := podmanEnv(); err != nil {
		log.Fatal(err)
	}
	d := newDesktops(dir, node)
	go d.refresh(context.Background())
	go d.keep(context.Background())
	serve(node, d)
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps providers in memory) or restarts, and
// when the desktops' tools change, once no call is waiting for an answer:
// registering again forgets those.
func serve(node string, d *desktops) {
	poll := &http.Client{Timeout: 40 * time.Second}
	var running atomic.Int64
	for {
		described, _ := json.Marshal(d.describe())
		if err := post(node+"/v1/providers/"+app, json.RawMessage(described)); err != nil {
			log.Printf("registering: %v", err)
			time.Sleep(3 * time.Second)
			continue
		}
		for {
			c, err := next(poll, node)
			if err != nil {
				log.Printf("waiting for calls: %v", err)
				time.Sleep(time.Second)
				break
			}
			if c == nil {
				if now, _ := json.Marshal(d.describe()); running.Load() == 0 && !bytes.Equal(now, described) {
					break
				}
				continue
			}
			running.Add(1)
			go func() {
				defer running.Add(-1)
				defer func() {
					if p := recover(); p != nil {
						log.Printf("%s panicked: %v\n%s", c.Op, p, debug.Stack())
						post(fmt.Sprintf("%s/v1/providers/%s/calls/%d", node, app, c.ID), map[string]any{"error": fmt.Sprint("desktops failed: ", p)})
					}
				}()
				ctx := context.WithValue(context.Background(), callerKey{}, c.Caller.Name)
				result, err := d.run(ctx, c.Op, c.Input)
				answer := map[string]any{"ok": result}
				if r, ok := result.(reply); ok {
					answer = map[string]any{"ok": r.ok, "entries": r.entries}
				}
				if err != nil {
					answer = map[string]any{"error": err.Error()}
				}
				if err := post(fmt.Sprintf("%s/v1/providers/%s/calls/%d", node, app, c.ID), answer); err != nil {
					log.Printf("answering %s: %v", c.Op, err)
				}
			}()
		}
	}
}

// next waits for the node's next call; nil if there was none.
func next(client *http.Client, node string) (*call, error) {
	resp, err := client.Get(node + "/v1/providers/" + app + "/calls")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	switch resp.StatusCode {
	case http.StatusNoContent:
		return nil, nil
	case http.StatusOK:
		var c call
		return &c, json.Unmarshal(body, &c)
	}
	return nil, fmt.Errorf("%s: %s", resp.Status, body)
}

func post(url string, body any) error {
	b, err := json.Marshal(body)
	if err != nil {
		return err
	}
	resp, err := http.Post(url, "application/json", bytes.NewReader(b))
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(resp.Body)
		return errors.New(resp.Status + ": " + string(msg))
	}
	return nil
}

func object(props map[string]any, required ...string) map[string]any {
	o := map[string]any{"type": "object", "properties": props}
	if len(required) > 0 {
		o["required"] = required
	}
	return o
}

func str(description string) map[string]any {
	return map[string]any{"type": "string", "description": description}
}

func num(description string) map[string]any {
	return map[string]any{"type": "integer", "description": description}
}

var desktopArg = str("which desktop, by name; default the only one")

const instructions = "Desktops are Linux desktops on the user's servers, each with a browser. " +
	"To browse, borrow a desktop of your own: desktops_borrow with for (what it's for, in a few words: the person sees it) gives you one whose browser starts signed in to the sites in the saved login (default unless you pass login); " +
	"pass its name as desktop to the other tools, and give it back with desktops_give_back when done: it's deleted, with anything it did to the login. One left 15 minutes without a call is given back for you. " +
	"Borrow rather than use a lasting desktop (desktops_create makes those, for the person), so agents never share a browser. " +
	"In it: desktops_browser_prepare with allow_launch true and profile {\"mode\":\"isolated_named\",\"name\":\"default\"} (the login's name) launches the browser; desktops_use with tool list_windows gives its window_id; " +
	"desktops_get_browser_state with that pid and window_id gives target_id and tab_id; then desktops_browser_navigate, desktops_browser_click and desktops_browser_type act, and desktops_get_browser_state reads the page. " +
	"A browser left idle for 5 minutes closes; prepare it again. desktops_screen shows the desktop. When a site needs the person to sign in, call desktops_sign_in with the site's url and give them its link: " +
	"they sign in there, in the saved login itself, and it's saved when they're done (or desktops_save_login). For a challenge in your own desktop, give them desktops_view's link. " +
	"Never sign in or submit forms for them unless they asked. Everything else on a desktop (apps, windows, keyboard, mouse): desktops_tools lists the tools and desktops_use runs one. " +
	"desktops_list shows every server's desktops, who has borrowed which and what for; to act on one on another server, pass server too. A lasting desktop nobody calls or watches for 30 minutes is paused, and unpaused by the next call."

// browsing are the desktop's tools offered as operations of their own; the
// rest are reached through use.
var browsing = []string{"get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_pointer", "browser_dialog"}

func ownOps() []map[string]any {
	name := object(map[string]any{"desktop": str("the desktop's name")}, "desktop")
	sizeArg := str("small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small")
	login := str("a saved login's name; default default")
	return []map[string]any{
		{"name": "list", "description": "Every server's desktops: each one's server, whether it's running, paused or stopped, its size, the memory and CPU it uses now, and who has borrowed it and what for (or that it's the spare kept for borrowing).",
			"input": object(map[string]any{"here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "borrow", "description": "Borrow a desktop of your own for as long as you need a browser, starting its browser with a copy of a saved login. Quick: a spare is kept ready. Give it back with give_back when done; it's deleted then, or after 15 minutes without a call, with anything in it.",
			"input": object(map[string]any{"for": str("what you need it for, in a few words, e.g. Reading Messenger; the person sees it"),
				"login": str("the saved login its browser starts with; default default if saved, \"\" for none"),
				"by":    str("who's borrowing, e.g. your app's name; default the device you call from")}, "for")},
		{"name": "give_back", "description": "Give back a borrowed desktop: it's deleted with everything in it, its browser's changes to the login too. A sign_in desktop's login is saved first.", "input": name},
		{"name": "sign_in", "description": "For the person: opens a page in a browser on a saved login (made if new) and gives a link to use it from their own devices, to sign in to sites agents then borrow desktops with. It's saved when its time is up, or on give_back or save_login.",
			"input": object(map[string]any{"login": login, "url": str("the page to open, e.g. a site's sign-in page"), "minutes": num("how long until it's saved and closed; default 30, at most 240")})},
		{"name": "save_login", "description": "Save a desktop's browser profile as a saved login, closing its browsers first; a sign_in desktop is also given back. Only when the person asked: borrowed desktops start from saved logins.",
			"input": object(map[string]any{"desktop": str("the desktop's name"), "login": str("the profile and saved login; default the desktop's login, or default")}, "desktop")},
		{"name": "logins", "description": "The saved logins desktops can be borrowed with, and when each was last saved.", "input": object(map[string]any{})},
		{"name": "forget_login", "description": "Delete a saved login, signing agents out of every site in it. Only when the person asked.",
			"input": object(map[string]any{"login": str("the login's name")}, "login")},
		{"name": "create", "description": "Create a lasting desktop and start it, for the person: Linux, with a browser of its own. Takes a minute. Agents borrow instead.",
			"input": object(map[string]any{"desktop": str("its name: lowercase letters, digits and dashes"), "size": sizeArg}, "desktop")},
		{"name": "resize", "description": "Change how much CPU and memory a desktop may use, at once, without restarting it.",
			"input": object(map[string]any{"desktop": str("the desktop's name"), "size": sizeArg}, "desktop", "size")},
		{"name": "start", "description": "Start a stopped (or paused) desktop. It starts again after the server restarts.", "input": name},
		{"name": "stop", "description": "Stop a desktop. Its files and sign-ins stay; it stays stopped until started.", "input": name},
		{"name": "delete", "description": "Delete a desktop and everything in it, its sign-ins too. Only when the person asked.", "input": name},
		{"name": "view", "description": "A link to watch and use a desktop in a browser, for the person: to solve a challenge, or to see what an agent is doing. Works on their own devices.",
			"input": object(map[string]any{"desktop": desktopArg, "minutes": num("how long the link works; default 60, at most 1440")})},
		{"name": "screen", "description": "A small picture of a desktop as it is now.",
			"input": object(map[string]any{"desktop": desktopArg, "width": num("the picture's width in pixels; default 640, at most 1920")})},
		{"name": "tools", "description": "The desktop's other tools, for desktops_use; with name, that tool's full description and input.",
			"input": object(map[string]any{"desktop": desktopArg, "name": str("a tool's name")})},
		{"name": "refresh_tools", "description": "Read the desktops' tools again from a running desktop, as after its image was updated.", "input": object(map[string]any{})},
		{"name": "use", "description": "Run one of the desktop's tools, as desktops_tools lists them, e.g. list_windows, launch_app, click, type_text, get_guide.",
			"input": object(map[string]any{"desktop": desktopArg, "tool": str("the tool's name"), "args": map[string]any{"type": "object", "description": "the tool's input"}}, "tool")},
		{"name": "backup", "description": "Back up a desktop's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.", "input": name},
		{"name": "backups", "description": "Every server's backups of desktops, newest first.",
			"input": object(map[string]any{"desktop": str("only this desktop's"), "here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "restore", "description": "Make a desktop on this server from a backup: the newest of the desktop named, from any server, or the one with blob.",
			"input": object(map[string]any{"desktop": str("the backed-up desktop's name"), "blob": str("a particular backup's blob"), "as": str("the new desktop's name; default the same"), "size": sizeArg})},
		{"name": "move", "description": "Move a desktop to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
			"input": object(map[string]any{"desktop": str("the desktop's name"), "to": str("the server to move it to")}, "desktop", "to")},
	}
}
