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
		lines := make([]string, len(list.Devices))
		var wg sync.WaitGroup
		for i, d := range list.Devices {
			name, _ := d["name"].(string)
			lines[i] = fmt.Sprintf("%-20s offline", name)
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
				if err != nil {
					d["usageError"] = err.Error()
					lines[i] = fmt.Sprintf("%-20s online, load unknown: %v", name, err)
					return
				}
				d["usage"] = u
				lines[i] = fmt.Sprintf("%-20s %s", name, u)
			})
		}
		wg.Wait()
		return sdk.Result{Text: strings.Join(lines, "\n"), Data: list}, nil
	})

	view := c.View("list", devicesHTML, nil)
	c.ShowIn("list", view, false)
	c.Menubar(func(items []sdk.MenubarItem) []sdk.MenubarItem {
		return append([]sdk.MenubarItem{{ID: "devices", Tool: "devices_list", Resource: view}}, items...)
	})
	return nil
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
