// Package devices is the built-in plugin that shows the user's devices in the
// menu bar, with how loaded each one is.
package devices

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

//go:embed devices.html
var devicesHTML string

type noIn struct{}

var Plugin = sdk.Plugin{ID: "devices", Name: "Devices", Version: "0.2.0", Setup: setup}

func setup(c *sdk.Ctx) error {
	var s sampler
	c.OnReady(func(*sdk.Call) { s.start() })

	sdk.Op(c, "usage", "Show this device's memory, CPU, GPU and disk use; CPU and GPU as percent busy over the last two minutes, oldest first.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
		u := s.usage()
		return sdk.Result{Text: u.String(), Data: u}, nil
	})

	sdk.Op(c, "list", "List this user's devices that run a node, this one first, with each online one's memory, CPU, GPU and disk use.", sdk.Read, func(call *sdk.Call, _ noIn) (sdk.Result, error) {
		res, err := call.Invoke("one", "devices", noIn{})
		if err != nil {
			return sdk.Result{}, err
		}
		var list struct {
			Devices []map[string]any `json:"devices"`
		}
		if err := recode(res.Data, &list); err != nil {
			return sdk.Result{}, err
		}
		lines := map[string]string{}
		usages := make([]*Usage, len(list.Devices))
		var mu sync.Mutex
		var wg sync.WaitGroup
		for i, d := range list.Devices {
			name, _ := d["name"].(string)
			lines[name] = fmt.Sprintf("%-20s offline", name)
			if online, _ := d["online"].(bool); !online {
				continue
			}
			wg.Go(func() {
				var u Usage
				var err error
				if self, _ := d["self"].(bool); self {
					u = s.usage()
				} else {
					u, err = remote(call, name)
				}
				mu.Lock()
				defer mu.Unlock()
				if err != nil {
					d["usageError"] = err.Error()
					lines[name] = fmt.Sprintf("%-20s online, load unknown: %v", name, err)
					return
				}
				d["usage"], usages[i] = u, &u
				lines[name] = fmt.Sprintf("%-20s %s", name, u)
			})
		}
		wg.Wait()
		list.Devices = nest(list.Devices, usages)
		var text []string
		for _, d := range list.Devices {
			line := lines[d["name"].(string)]
			if host, ok := d["host"].(string); ok {
				line = "  " + line + " (container on " + host + ")"
			}
			text = append(text, line)
		}
		return sdk.Result{Text: strings.Join(text, "\n"), Data: list}, nil
	})

	view := c.View("list", devicesHTML, nil)
	c.ShowIn("list", view, false)
	c.Menubar(func(items []sdk.MenubarItem) []sdk.MenubarItem {
		return append([]sdk.MenubarItem{{ID: "devices", Tool: "devices_list", Resource: view}}, items...)
	})
	return nil
}

// nest moves each system container right after the device hosting it and
// sets its host.
func nest(devices []map[string]any, usages []*Usage) []map[string]any {
	byName := map[string]map[string]any{}
	for _, d := range devices {
		byName[d["name"].(string)] = d
	}
	guests := map[string][]map[string]any{}
	for i, u := range usages {
		if u == nil {
			continue
		}
		host := devices[i]["name"].(string)
		for _, g := range u.Guests {
			if d := byName[g]; d != nil && d["host"] == nil && g != host {
				d["host"] = host
				guests[host] = append(guests[host], d)
			}
		}
	}
	var out []map[string]any
	for _, d := range devices {
		if d["host"] == nil {
			out = append(out, d)
			out = append(out, guests[d["name"].(string)]...)
		}
	}
	return out
}

// remote asks another device's node, giving up quickly so one slow device
// doesn't hold up the menu.
func remote(call *sdk.Call, device string) (Usage, error) {
	ctx, cancel := context.WithTimeout(call.Context, 3*time.Second)
	defer cancel()
	c := *call
	c.Context = ctx
	res, err := c.Invoke("devices", "usage", map[string]string{"device": device})
	if err != nil {
		return Usage{}, err
	}
	var u Usage
	return u, recode(res.Data, &u)
}

func recode(v, out any) error {
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return json.Unmarshal(b, out)
}
