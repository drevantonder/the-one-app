package core

import (
	"encoding/json"
	"fmt"
	"os"
	"slices"
	"strings"
)

// Every plugin operation can run on any device that has the plugin. The
// surfaces show it with a device argument, which the core takes out before
// the plugin sees the input, and with the devices it is available on.

const deviceArg = "device"

// Name is this device's name as the user knows it: its tailnet name once the
// mesh sets it, the hostname until then.
func (n *Node) Name() string {
	n.mu.RLock()
	name := n.name
	n.mu.RUnlock()
	if name != "" {
		return name
	}
	host, _ := os.Hostname()
	host, _, _ = strings.Cut(host, ".")
	return strings.ToLower(host)
}

// SetName sets this device's name.
func (n *Node) SetName(name string) {
	n.mu.Lock()
	n.name = name
	n.mu.Unlock()
	n.changed()
}

func (n *Node) isSelf(device string) bool {
	return device == n.Name() || device == n.store.Device()
}

// availability lists every plugin operation once, with the names of the
// devices that have it, this one first.
func (n *Node) availability() ([]Op, map[string][]string) {
	var ops []Op
	devices := map[string][]string{}
	add := func(o Op, device string) {
		key := o.Plugin + "/" + o.Name
		if _, ok := devices[key]; !ok {
			ops = append(ops, o)
		}
		if !slices.Contains(devices[key], device) {
			devices[key] = append(devices[key], device)
		}
	}
	for _, o := range n.LocalOperations() {
		add(o, n.Name())
	}
	for _, r := range n.remoteOps() {
		add(r.Op, r.DeviceName)
	}
	return ops, devices
}

// withDevices adds the device argument and where the operation is available.
func withDevices(o Op, devices []string) Op {
	o.Description = strings.TrimRight(o.Description, "\n") + "\n\nAvailable on: " + strings.Join(devices, ", ") + "."
	if ownsDevice(o) {
		return o // the plugin has its own device argument; it wins
	}
	var schema map[string]json.RawMessage
	if json.Unmarshal(o.InputSchema, &schema) != nil || schema == nil {
		schema = map[string]json.RawMessage{"type": json.RawMessage(`"object"`)}
	}
	arg, _ := json.Marshal(map[string]any{
		"type":        "string",
		"enum":        devices,
		"description": "The device to run on. Default: this one if it has the plugin, otherwise one that does.",
	})
	// Spliced in last, not re-marshaled, so the plugin's own arguments keep
	// their order: the CLI fills them from positional words in that order.
	props := strings.TrimSpace(string(schema["properties"]))
	device := `"` + deviceArg + `":` + string(arg) + "}"
	if strings.HasPrefix(props, "{") && strings.TrimSpace(props[1:]) != "}" {
		props = strings.TrimSuffix(props, "}") + "," + device
	} else {
		props = "{" + device
	}
	schema["properties"] = json.RawMessage(props)
	o.InputSchema, _ = json.Marshal(schema)
	return o
}

func ownsDevice(o Op) bool {
	var schema struct {
		Properties map[string]json.RawMessage `json:"properties"`
	}
	_ = json.Unmarshal(o.InputSchema, &schema)
	_, ok := schema.Properties[deviceArg]
	return ok
}

// takeDevice removes the device argument from a call's input.
func (n *Node) takeDevice(plugin, op string, input json.RawMessage) (json.RawMessage, string, error) {
	var in map[string]json.RawMessage
	if json.Unmarshal(input, &in) != nil || in[deviceArg] == nil {
		return input, "", nil
	}
	ops, _ := n.availability()
	for _, o := range ops {
		if o.Plugin == plugin && o.Name == op && ownsDevice(o) {
			return input, "", nil
		}
	}
	var device string
	if err := json.Unmarshal(in[deviceArg], &device); err != nil {
		return nil, "", fmt.Errorf("%s must be a device name", deviceArg)
	}
	delete(in, deviceArg)
	out, err := json.Marshal(in)
	return out, device, err
}

// remoteOn finds the device id of the named device, if it offers the
// operation; otherwise it explains where the operation is available.
func (n *Node) remoteOn(plugin, op, device string) (string, error) {
	for _, r := range n.remoteOps() {
		if r.Plugin == plugin && r.Name == op && (r.DeviceName == device || r.Device == device) {
			return r.Device, nil
		}
	}
	return "", n.notOn(plugin, op, device)
}

// notOn explains that an operation isn't on a device, and where it is.
func (n *Node) notOn(plugin, op, device string) error {
	_, devices := n.availability()
	where := devices[plugin+"/"+op]
	if len(where) == 0 {
		return fmt.Errorf("no device has %s %s", plugin, op)
	}
	return fmt.Errorf("%s %s isn't available on %q; it is on: %s", plugin, op, device, strings.Join(where, ", "))
}
