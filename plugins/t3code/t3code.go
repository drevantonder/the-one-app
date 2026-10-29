// Package t3code is the built-in plugin that runs T3 Code
// (github.com/pingdotgg/t3code) servers on the devices the user chooses and
// pairs the user's T3 Code apps with them over Tailscale, without T3
// Connect. A Mac with the desktop app uses the app's own server; other
// hosts get the t3 CLI and its background service.
package t3code

import (
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/plugins/internal/nodes"
	"github.com/drevantonder/the-one-app/sdk"
)

var Plugin = sdk.Plugin{ID: "t3code", Name: "T3 Code", Version: "0.1.0", Platforms: []string{"linux", "darwin"}, Setup: setup}

type hostsIn struct {
	Devices []string `json:"devices" jsonschema:"every device to run a T3 Code server on, by name; the others stop being hosts"`
}

type hosts struct {
	Devices []string `json:"devices"`
}

type pairIn struct {
	For     string   `json:"for" jsonschema:"the app to pair, e.g. iPhone or Mac; shown in each server's list of connections"`
	Devices []string `json:"devices,omitempty" jsonschema:"the hosts to pair with; default every host"`
}

type pairHereIn struct {
	For string `json:"for" jsonschema:"the app to pair, shown in the server's list of connections"`
}

type plugin struct {
	mu    sync.Mutex
	hosts []string
}

func (p *plugin) isHost(name string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Contains(p.hosts, name)
}

func setup(c *sdk.Ctx) error {
	p := &plugin{}
	c.Entry("hosts", sdk.Available)
	c.OnReset(func() {
		p.mu.Lock()
		p.hosts = nil
		p.mu.Unlock()
	})
	sdk.Handle(c, "hosts", func(_ sdk.Entry, in hosts) error {
		p.mu.Lock()
		p.hosts = in.Devices
		p.mu.Unlock()
		return nil
	})
	c.OnReady(func(call *sdk.Call) { go p.keepServing(call.Detached()) })

	sdk.Op(c, "status", "Show every device's T3 Code server: which devices are hosts, the version, how it runs, and the apps paired with it, with when each pairing was last used and when it expires.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		hs, err := p.all(call)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: report(hs, time.Now()), Data: hs}, nil
	})

	sdk.Op(c, "here", "Show this device's T3 Code server and the apps paired with it; status shows every device.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		h := p.here(call)
		return sdk.Result{Text: report([]host{h}, time.Now()), Data: h}, nil
	})

	sdk.Op(c, "hosts", "Choose the devices to run T3 Code servers on. Each gets T3 Code installed and running in the background, unless it is a Mac with the desktop app, whose own server is used.", sdk.Write, func(call *sdk.Call, in hostsIn) (sdk.Result, error) {
		names, err := nodes.Names(call)
		if err != nil {
			return sdk.Result{}, err
		}
		for _, d := range in.Devices {
			if !slices.Contains(names, d) {
				return sdk.Result{}, fmt.Errorf("no device named %q; devices: %s", d, strings.Join(names, ", "))
			}
		}
		if _, err := call.Append("hosts", hosts{Devices: in.Devices}); err != nil {
			return sdk.Result{}, err
		}
		if len(in.Devices) == 0 {
			return sdk.Text("No T3 Code hosts."), nil
		}
		return sdk.Text("T3 Code hosts: " + strings.Join(in.Devices, ", ") + ". Each is set up within a few minutes; then pair your apps with pair."), nil
	})

	sdk.Op(c, "pair", "Pair a T3 Code app, such as on an iPhone or a Mac, with every host: gives a one-time link for each, over Tailscale HTTPS. Paste each into Add environment in the app, or open it on the device.", sdk.Write, func(call *sdk.Call, in pairIn) (sdk.Result, error) {
		return p.pair(call, in)
	})

	sdk.Op(c, "pair_here", "Make a one-time link to pair a T3 Code app with this device's server, over Tailscale HTTPS; pair does this for every host.", sdk.Write, func(call *sdk.Call, in pairHereIn) (sdk.Result, error) {
		url, err := pairLink(call.Context, in.For)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: url, Data: map[string]string{"url": url}}, nil
	})
	return nil
}

// keepServing sets up T3 Code once this device is a host and keeps its
// background service installed. T3 Code updates itself from its apps.
func (p *plugin) keepServing(call *sdk.Call) {
	for ; ; time.Sleep(time.Minute) {
		me, _ := nodes.Self(call)
		if me == "" || !p.isHost(me) {
			continue
		}
		if err := serve(call.Context); err != nil {
			call.Logf("set up T3 Code: %v", err)
		}
	}
}
