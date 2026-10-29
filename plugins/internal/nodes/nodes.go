// Package nodes lists the user's devices that run a node, for plugins that
// act on several of them.
package nodes

import (
	"encoding/json"
	"errors"

	"github.com/drevantonder/the-one-app/sdk"
)

type Node struct {
	Name   string `json:"name"`
	Self   bool   `json:"self"`
	Online bool   `json:"online"`
}

func List(call *sdk.Call) ([]Node, error) {
	res, err := call.Invoke("one", "devices", struct{}{})
	if err != nil {
		return nil, err
	}
	raw, _ := json.Marshal(res.Data)
	var d struct {
		Devices []Node `json:"devices"`
	}
	return d.Devices, json.Unmarshal(raw, &d)
}

func Names(call *sdk.Call) ([]string, error) {
	ns, err := List(call)
	var names []string
	for _, n := range ns {
		names = append(names, n.Name)
	}
	return names, err
}

// Self is this device's name. It is the hostname until the node joins the
// tailnet, so don't keep it.
func Self(call *sdk.Call) (string, error) {
	ns, err := List(call)
	for _, n := range ns {
		if n.Self {
			return n.Name, nil
		}
	}
	if err == nil {
		err = errors.New("this device isn't in the device list")
	}
	return "", err
}
