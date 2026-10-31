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
	"sync/atomic"
	"time"
)

const app = "desktops"

type call struct {
	ID    uint64          `json:"id"`
	Op    string          `json:"op"`
	Input json.RawMessage `json:"input"`
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
	d := newDesktops(filepath.Join(dir, "tools.json"))
	go d.refresh(context.Background())
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
				result, err := d.run(context.Background(), c.Op, c.Input)
				reply := map[string]any{"ok": result}
				if err != nil {
					reply = map[string]any{"error": err.Error()}
				}
				if err := post(fmt.Sprintf("%s/v1/providers/%s/calls/%d", node, app, c.ID), reply); err != nil {
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

const instructions = "Desktops are Linux desktops on the device that runs them, each with a browser of its own that keeps its sign-ins. " +
	"To browse: desktops_browser_prepare with allow_launch true and profile {\"mode\":\"isolated_named\",\"name\":\"default\"} (the profile with the desktop's sign-ins) " +
	"launches the browser; desktops_use with tool list_windows gives its window_id; desktops_get_browser_state with that pid and window_id gives target_id and tab_id; " +
	"then desktops_browser_navigate, desktops_browser_click and desktops_browser_type act, and desktops_get_browser_state reads the page. " +
	"A browser left idle for 5 minutes closes; prepare it again. desktops_screen shows the desktop. When a site needs the person (to sign in, or to solve a challenge), " +
	"open the page and give them desktops_view's link: they use the desktop in their browser. Never sign in or submit forms for them unless they asked. " +
	"Everything else on a desktop (apps, windows, keyboard, mouse): desktops_tools lists the tools and desktops_use runs one. With several desktops, pass desktop."

// browsing are the desktop's tools offered as operations of their own; the
// rest are reached through use.
var browsing = []string{"get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_pointer", "browser_dialog"}

func ownOps() []map[string]any {
	name := object(map[string]any{"desktop": str("the desktop's name")}, "desktop")
	return []map[string]any{
		{"name": "list", "description": "The desktops: each one's name, whether it's running, and when it last started.",
			"input": object(map[string]any{})},
		{"name": "create", "description": "Create a desktop and start it: Linux, with a browser of its own. Takes a minute.",
			"input": object(map[string]any{"desktop": str("its name: lowercase letters, digits and dashes")}, "desktop")},
		{"name": "start", "description": "Start a stopped desktop. It starts again after the device restarts.", "input": name},
		{"name": "stop", "description": "Stop a desktop. Its files and sign-ins stay; it stays stopped until started.", "input": name},
		{"name": "delete", "description": "Delete a desktop and everything in it, its sign-ins too. Only when the person asked.", "input": name},
		{"name": "view", "description": "A link to watch and use a desktop in a browser, for the person: to sign in to a site, or to see what an agent is doing. Works on their own devices.",
			"input": object(map[string]any{"desktop": desktopArg, "minutes": num("how long the link works; default 60, at most 1440")})},
		{"name": "screen", "description": "A small picture of a desktop as it is now.",
			"input": object(map[string]any{"desktop": desktopArg, "width": num("the picture's width in pixels; default 640, at most 1920")})},
		{"name": "tools", "description": "The desktop's other tools, for desktops_use; with name, that tool's full description and input.",
			"input": object(map[string]any{"desktop": desktopArg, "name": str("a tool's name")})},
		{"name": "use", "description": "Run one of the desktop's tools, as desktops_tools lists them, e.g. list_windows, launch_app, click, type_text, get_guide.",
			"input": object(map[string]any{"desktop": desktopArg, "tool": str("the tool's name"), "args": map[string]any{"type": "object", "description": "the tool's input"}}, "tool")},
	}
}
