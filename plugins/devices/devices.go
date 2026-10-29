// Package devices is the built-in plugin that shows the user's devices in the
// menu bar.
package devices

import (
	_ "embed"

	"github.com/drevantonder/the-one-app/sdk"
)

//go:embed devices.html
var devicesHTML string

type noIn struct{}

var Plugin = sdk.Plugin{ID: "devices", Name: "Devices", Version: "0.1.0", Setup: setup}

func setup(c *sdk.Ctx) error {
	sdk.Op(c, "list", "List this user's devices that run a node, this one first, and whether each is online.", sdk.Read, func(call *sdk.Call, _ noIn) (sdk.Result, error) {
		return call.Invoke("one", "devices", noIn{})
	})
	view := c.View("list", devicesHTML, nil)
	c.ShowIn("list", view, false)
	c.Menubar(func(items []sdk.MenubarItem) []sdk.MenubarItem {
		return append([]sdk.MenubarItem{{ID: "devices", Title: "Devices", Tool: "devices_list", Resource: view}}, items...)
	})
	return nil
}
