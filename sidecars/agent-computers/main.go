// The Agent Computers sidecar: Linux computers for agents, each a Podman container
// running Cua's desktop image (github.com/trycua/cua) with Google Chrome, which
// keeps its sign-ins, offered to the person's devices through this device's
// node. The server inside each computer speaks MCP; its
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

const app = "agent-computers"

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
	dir := cmp.Or(os.Getenv("AGENT_COMPUTERS_DATA"), filepath.Join(home, ".local", "share", "one", "agent-computers"))
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	if err := setup(dir); err != nil {
		log.Fatal(err)
	}
	d := newComputers(dir, node)
	go d.refresh(context.Background())
	go d.keep(context.Background())
	serve(node, d)
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps sidecars in memory) or restarts, and
// when the computers' tools change, once no call is waiting for an answer:
// registering again forgets those.
func serve(node string, d *computers) {
	poll := &http.Client{Timeout: 40 * time.Second}
	var running atomic.Int64
	for {
		described, _ := json.Marshal(d.describe())
		if err := post(node+"/v1/sidecars/"+app, json.RawMessage(described)); err != nil {
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
						post(fmt.Sprintf("%s/v1/sidecars/%s/calls/%d", node, app, c.ID), map[string]any{"error": fmt.Sprint("computers failed: ", p)})
					}
				}()
				ctx := context.WithValue(context.Background(), callerKey{}, c.Caller.Name)
				result, err := d.run(ctx, c.Op, c.Input)
				answer := map[string]any{"ok": result}
				if r, ok := result.(reply); ok {
					answer = map[string]any{"ok": r.ok, "blobs": r.blobs}
				}
				if err != nil {
					answer = map[string]any{"error": err.Error()}
				}
				if err := post(fmt.Sprintf("%s/v1/sidecars/%s/calls/%d", node, app, c.ID), answer); err != nil {
					log.Printf("answering %s: %v", c.Op, err)
				}
			}()
		}
	}
}

// next waits for the node's next call; nil if there was none.
func next(client *http.Client, node string) (*call, error) {
	resp, err := client.Get(node + "/v1/sidecars/" + app + "/calls")
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

var computerArg = str("which computer, by name; default the only one")

const instructions = "Linux computers for agents, each with a browser, on this server. Agents use them through Computer Use (computer_*), and the person sees them on the Agent Computers page."

// browsing are the computer's tools offered as operations of their own; the
// rest are reached through use.
var browsing = []string{"get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_pointer", "browser_dialog"}

func ownOps() []map[string]any {
	name := object(map[string]any{"computer": str("the computer's name")}, "computer")
	sizeArg := str("small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small")
	prof := map[string]any{"type": "object", "description": "a browser profile, as the app gives it from its Vault item: item, name, key (filled in by the node), blob and salt"}
	return []map[string]any{
		{"name": "list", "description": "Every server's computers: each one's server, whether it's running, paused or stopped, its size, the memory and CPU it uses now, and who has borrowed it and what for (or that it's the spare kept for borrowing).",
			"input": object(map[string]any{"here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "borrow", "description": "Borrow a computer of your own for as long as you need a browser, starting its browser with a copy of a browser profile. Quick: a spare is kept ready. Give it back with give_back when done; it's deleted then, or after 15 minutes without a call, with anything in it.",
			"input": object(map[string]any{"for": str("what you need it for, in a few words, e.g. Reading Messenger; the person sees it"),
				"profile": prof, "screen": map[string]any{"type": "boolean", "description": "false for a computer nobody watches: no view link"},
				"by": str("who's borrowing, e.g. your app's name; default the device you call from")}, "for")},
		{"name": "give_back", "description": "Give back a borrowed computer: it's deleted with everything in it, its browser's changes to the profile too. A sign_in computer's profile is saved first.", "input": name},
		{"name": "sign_in", "description": "For the person: opens a page in a browser on a browser profile and gives a link to use it from their own devices, to sign in to sites. It's saved when its time is up, or on save_profile.",
			"input": object(map[string]any{"profile": prof, "url": str("the page to open, e.g. a site's sign-in page"), "minutes": num("how long until it's saved and closed; default 30, at most 240")}, "profile")},
		{"name": "save_profile", "description": "Save a browser profile from the computer signing in to it (closing that computer), or from the computer named.",
			"input": object(map[string]any{"computer": str("the computer's name"), "profile": prof})},
		{"name": "signing_in", "description": "The browser profiles being signed in to on each server: item, profile, computer, server and until.", "input": object(map[string]any{})},
		{"name": "create", "description": "Create a lasting computer and start it, for the person: Linux, with a browser of its own. Takes a minute. Agents borrow instead.",
			"input": object(map[string]any{"computer": str("its name: lowercase letters, digits and dashes"), "size": sizeArg}, "computer")},
		{"name": "resize", "description": "Change how much CPU and memory a computer may use, at once, without restarting it.",
			"input": object(map[string]any{"computer": str("the computer's name"), "size": sizeArg}, "computer", "size")},
		{"name": "start", "description": "Start a stopped (or paused) computer. It starts again after the server restarts.", "input": name},
		{"name": "stop", "description": "Stop a computer. Its files and sign-ins stay; it stays stopped until started.", "input": name},
		{"name": "delete", "description": "Delete a computer and everything in it, its sign-ins too. Only when the person asked.", "input": name},
		{"name": "view", "description": "A link to watch and use a computer in a browser, for the person: to solve a challenge, or to see what an agent is doing. Works on their own devices.",
			"input": object(map[string]any{"computer": computerArg, "minutes": num("how long the link works; default 60, at most 1440")})},
		{"name": "screen", "description": "A small picture of a computer as it is now.",
			"input": object(map[string]any{"computer": computerArg, "width": num("the picture's width in pixels; default 640, at most 1920")})},
		{"name": "tools", "description": "The computer's other tools, for computers_use; with name, that tool's full description and input.",
			"input": object(map[string]any{"computer": computerArg, "name": str("a tool's name")})},
		{"name": "refresh_tools", "description": "Read the computers' tools again from a running computer, as after its image was updated.", "input": object(map[string]any{})},
		{"name": "use", "description": "Run one of the computer's tools, as computers_tools lists them, e.g. list_windows, launch_app, click, type_text, get_guide.",
			"input": object(map[string]any{"computer": computerArg, "tool": str("the tool's name"), "args": map[string]any{"type": "object", "description": "the tool's input"}}, "tool")},
		{"name": "backup", "description": "Back up a computer's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.", "input": name},
		{"name": "backups", "description": "Every server's backups of computers, newest first.",
			"input": object(map[string]any{"computer": str("only this computer's"), "here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "restore", "description": "Make a computer on this server from a backup: the newest of the computer named, from any server, or the one with blob.",
			"input": object(map[string]any{"computer": str("the backed-up computer's name"), "blob": str("a particular backup's blob"), "as": str("the new computer's name; default the same"), "size": sizeArg})},
		{"name": "move", "description": "Move a computer to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
			"input": object(map[string]any{"computer": str("the computer's name"), "to": str("the server to move it to")}, "computer", "to")},
	}
}
