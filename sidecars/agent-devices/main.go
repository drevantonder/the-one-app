// The Agent Devices sidecar: devices for agents. Linux ones (kind linux) are each a Podman container
// running Cua's desktop image (github.com/trycua/cua) with Google Chrome, which
// keeps its sign-ins, offered to the person's devices through this device's
// node. The server inside each device speaks MCP; its
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

const app = "agent-devices"

// dataDir is this sidecar's folder.
var dataDir string

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
	dir := cmp.Or(os.Getenv("AGENT_DEVICES_DATA"), filepath.Join(home, ".local", "share", "one", "agent-devices"))
	dataDir = dir
	node := "http://" + nodeAddr()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	setup(dir)
	d := newDevices(dir, node)
	go d.refresh(context.Background())
	go d.keep(context.Background())
	go d.serveViewer()
	serve(node, d)
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps sidecars in memory) or restarts, and
// when the devices' tools change, once no call is waiting for an answer:
// registering again forgets those.
func serve(node string, d *devices) {
	poll := &http.Client{Timeout: 40 * time.Second, Transport: toNode}
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
						post(fmt.Sprintf("%s/v1/sidecars/%s/calls/%d", node, app, c.ID), map[string]any{"error": fmt.Sprint("devices failed: ", p)})
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

var deviceArg = str("which device, by name; default the only one")

var kindArg = str("linux (with a browser; the default), windows, android or iphone: kinds lists what each server runs")

const instructions = "Devices for agents on this server: Linux with a browser, and Windows, Android and iPhones where this machine runs them (kinds). Agents use them through Computer Use (computer_*), and the person sees them on the Agent Devices page."

// browsing are the device's tools offered as operations of their own; the
// rest are reached through use.
var browsing = []string{"get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_pointer", "browser_dialog"}

func ownOps() []map[string]any {
	name := object(map[string]any{"device": str("the device's name")}, "device")
	sizeArg := str("small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small")
	prof := map[string]any{"type": "object", "description": "a browser profile, as the app gives it from its Vault item: item, name, key (filled in by the node), blob and salt"}
	return []map[string]any{
		{"name": "list", "description": "Every server's devices: each one's server, whether it's running, paused or stopped, its size, the memory and CPU it uses now, and who has borrowed it and what for (or that it's the spare kept for borrowing).",
			"input": object(map[string]any{"here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "kinds", "description": "The kinds of device each server runs (linux, windows, android, iphone), with what input each takes, whether it has room for another and how many lasting ones nobody has borrowed, and its memory to spare in GB.",
			"input": object(map[string]any{"here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "adopt", "description": "Make something already on this server a lasting device: a Windows VM's folder, an Android Virtual Device, or a simulator.",
			"input": object(map[string]any{"device": str("its name"), "kind": kindArg, "dir": str("windows: its folder"), "id": str("android: its AVD; iphone: its simulator's UDID"),
				"port": num("windows: its SSH port here; android: its console port"), "display": num("windows: its VNC display"), "cpus": num("windows: its CPUs"), "gb": num("windows: its memory")}, "device", "kind")},
		{"name": "open", "description": "Open a URL in a device's browser.", "input": object(map[string]any{"device": str("the device's name"), "url": str("the URL")}, "device", "url")},
		{"name": "install", "description": "Install an app on a device: a file here or a URL (.apk, .msi, .msix, .exe, .app, .deb), or a package's name (winget id, Android package, apt package).",
			"input": object(map[string]any{"device": str("the device's name"), "app": str("the app")}, "device", "app")},
		{"name": "run", "description": "Run a command on a device and give its output: PowerShell on Windows, the shell on Android and Linux, simctl spawn on an iPhone.",
			"input": object(map[string]any{"device": str("the device's name"), "command": str("the command")}, "device", "command")},
		{"name": "reach", "description": "Let a Windows, Android or iPhone device reach a port of this machine's 127.0.0.1 at the same port on its own, until it stops.",
			"input": object(map[string]any{"device": str("the device's name"), "port": num("the port")}, "device", "port")},
		{"name": "borrow", "description": "Borrow a device for as long as you need it. Linux (the default): one of your own whose browser starts with a copy of a browser profile, quick, as a spare is kept ready. Other kinds: the person's lasting device, one agent at a time, or a throwaway copy of it. Give it back with give_back when done; a throwaway one is deleted then, or after 15 minutes without a call, with anything in it.",
			"input": object(map[string]any{"kind": kindArg, "device": str("a lasting device, by name"), "copy": map[string]any{"type": "boolean", "description": "a throwaway copy, not the lasting device"},
				"for":     str("what you need it for, in a few words, e.g. Reading Messenger; the person sees it"),
				"profile": prof, "screen": map[string]any{"type": "boolean", "description": "false for a device nobody watches: no view link"},
				"by": str("who's borrowing, e.g. your app's name; default the device you call from")}, "for")},
		{"name": "give_back", "description": "Give back a borrowed device: it's deleted with everything in it, its browser's changes to the profile too. A sign_in device's profile is saved first.", "input": name},
		{"name": "sign_in", "description": "For the person: opens a page in a browser on a browser profile and gives a link to use it from their own devices, to sign in to sites. It's saved when its time is up, or on save_profile.",
			"input": object(map[string]any{"profile": prof, "url": str("the page to open, e.g. a site's sign-in page"), "minutes": num("how long until it's saved and closed; default 30, at most 240")}, "profile")},
		{"name": "save_profile", "description": "Save a browser profile from the device signing in to it (closing that device), or from the device named.",
			"input": object(map[string]any{"device": str("the device's name"), "profile": prof})},
		{"name": "signing_in", "description": "The browser profiles being signed in to on each server: item, profile, device, server and until.", "input": object(map[string]any{})},
		{"name": "create", "description": "Create a lasting device and start it, for the person, on the first server in Where Things Run (Settings > Devices) that runs its kind and has room: Linux with a browser of its own unless kind says another. Takes a minute. Agents borrow instead.",
			"input": object(map[string]any{"device": str("its name: lowercase letters, digits and dashes"), "kind": kindArg, "size": sizeArg}, "device")},
		{"name": "resize", "description": "Change how much CPU and memory a device may use, at once, without restarting it.",
			"input": object(map[string]any{"device": str("the device's name"), "size": sizeArg}, "device", "size")},
		{"name": "start", "description": "Start a stopped (or paused) device. It starts again after the server restarts.", "input": name},
		{"name": "stop", "description": "Stop a device. Its files and sign-ins stay; it stays stopped until started.", "input": name},
		{"name": "delete", "description": "Delete a device and everything in it, its sign-ins too. Only when the person asked.", "input": name},
		{"name": "view", "description": "A link to watch and use a device in a browser, for the person: to solve a challenge, or to see what an agent is doing. Works on their own devices.",
			"input": object(map[string]any{"device": deviceArg, "minutes": num("how long the link works; default 60, at most 1440")})},
		{"name": "screen", "description": "A small picture of a device as it is now.",
			"input": object(map[string]any{"device": deviceArg, "width": num("the picture's width in pixels; default 640, at most 1920")})},
		{"name": "tools", "description": "The device's other tools, for computer_use; with name, that tool's full description and input.",
			"input": object(map[string]any{"device": deviceArg, "name": str("a tool's name")})},
		{"name": "refresh_tools", "description": "Read the devices' tools again from a running device, as after its image was updated.", "input": object(map[string]any{})},
		{"name": "use", "description": "Run one of the device's tools, as computer_tools lists them, e.g. list_windows, launch_app, click, type_text, get_guide.",
			"input": object(map[string]any{"device": deviceArg, "tool": str("the tool's name"), "args": map[string]any{"type": "object", "description": "the tool's input"}}, "tool")},
		{"name": "backup", "description": "Back up a device's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.", "input": name},
		{"name": "backups", "description": "Every server's backups of devices, newest first.",
			"input": object(map[string]any{"device": str("only this device's"), "here": map[string]any{"type": "boolean", "description": "only this server's"}})},
		{"name": "restore", "description": "Make a device on this server from a backup: the newest of the device named, from any server, or the one with blob.",
			"input": object(map[string]any{"device": str("the backed-up device's name"), "blob": str("a particular backup's blob"), "as": str("the new device's name; default the same"), "size": sizeArg})},
		{"name": "move", "description": "Move a device to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
			"input": object(map[string]any{"device": str("the device's name"), "to": str("the server to move it to")}, "device", "to")},
	}
}
