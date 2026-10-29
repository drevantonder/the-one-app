// Package claude is the built-in plugin that sets up Claude Code on the
// devices the user codes on: it keeps Claude Code and claude-swap
// (github.com/realiti4/claude-swap) installed there, and signs each one in
// to the user's Claude accounts from a link opened on any device, so
// claude-swap can switch between them.
package claude

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

var Plugin = sdk.Plugin{ID: "claude", Name: "Claude Code", Version: "0.1.0", Platforms: []string{"linux", "darwin"}, Setup: setup}

type codingIn struct {
	Devices []string `json:"devices" jsonschema:"every device to code on, by name; the others stop being coding devices"`
}

type coding struct {
	Devices []string `json:"devices"`
}

type signInIn struct {
	Email   string   `json:"email,omitempty" jsonschema:"the account to sign in to: filled in on the sign-in page, and devices that already have it are skipped"`
	Devices []string `json:"devices,omitempty" jsonschema:"the devices to sign in on; default every coding device"`
}

type startIn struct {
	Email string `json:"email,omitempty" jsonschema:"the account to sign in to, filled in on the sign-in page"`
}

type finishIn struct {
	Code string `json:"code,omitempty" jsonschema:"the code the sign-in page shows once signed in; leave out if the device's own browser finished signing in"`
}

type plugin struct {
	mu     sync.Mutex
	coding []string
	login  *login
}

// me is this device's name. It isn't kept: it is the hostname until the
// node joins the tailnet.
func (p *plugin) me(call *sdk.Call) string {
	name, _ := selfName(call)
	return name
}

func (p *plugin) isCoding(name string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	return slices.Contains(p.coding, name)
}

func setup(c *sdk.Ctx) error {
	p := &plugin{}
	c.Entry("coding", sdk.Available)
	c.OnReset(func() {
		p.mu.Lock()
		p.coding = nil
		p.mu.Unlock()
	})
	sdk.Handle(c, "coding", func(_ sdk.Entry, in coding) error {
		p.mu.Lock()
		p.coding = in.Devices
		p.mu.Unlock()
		return nil
	})
	c.OnReady(func(call *sdk.Call) { go p.keepInstalled(call.Detached()) })

	sdk.Op(c, "status", "Show every device's Claude Code and claude-swap, which devices are for coding, and the Claude accounts each is signed in to, with how much of each account's limits is used.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		devices, err := p.all(call)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: report(devices), Data: devices}, nil
	})

	sdk.Op(c, "here", "Show this device's Claude Code, claude-swap and Claude accounts; status shows every device.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		d := p.here(call)
		return sdk.Result{Text: report([]device{d}), Data: d}, nil
	})

	sdk.Op(c, "coding", "Choose the devices to code on. Each gets Claude Code and claude-swap installed and kept up to date, and sign_in signs them in.", sdk.Write, func(call *sdk.Call, in codingIn) (sdk.Result, error) {
		names, err := deviceNames(call)
		if err != nil {
			return sdk.Result{}, err
		}
		for _, d := range in.Devices {
			if !slices.Contains(names, d) {
				return sdk.Result{}, fmt.Errorf("no device named %q; devices: %s", d, strings.Join(names, ", "))
			}
		}
		if _, err := call.Append("coding", coding{Devices: in.Devices}); err != nil {
			return sdk.Result{}, err
		}
		if len(in.Devices) == 0 {
			return sdk.Text("No coding devices."), nil
		}
		return sdk.Text("Coding on " + strings.Join(in.Devices, ", ") + ". Claude Code and claude-swap are installed on each within a few minutes; then sign them in with sign_in."), nil
	})

	sdk.Op(c, "sign_in", "Sign coding devices in to a Claude account: gives a link for each device. Open each, sign in, and give the code the page shows to finish_sign_in on that device. The account is added to claude-swap; the device keeps using the account it was on.", sdk.Write, func(call *sdk.Call, in signInIn) (sdk.Result, error) {
		return p.signIn(call, in)
	})

	sdk.Op(c, "start_sign_in", "Start signing this device in to a Claude account and give the link to sign in at; sign_in does this for every coding device.", sdk.Write, func(call *sdk.Call, in startIn) (sdk.Result, error) {
		url, err := p.start(in.Email)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: url, Data: map[string]string{"url": url}}, nil
	})

	sdk.Op(c, "finish_sign_in", "Finish signing this device in with the code the sign-in page showed, and add the account to claude-swap.", sdk.Write, func(call *sdk.Call, in finishIn) (sdk.Result, error) {
		email, err := p.finish(call.Context, in.Code)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Signed in to " + email + "."), nil
	})

	sdk.Op(c, "install", "Install or update Claude Code and claude-swap on this device now.", sdk.Write, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		if err := install(call.Context); err != nil {
			return sdk.Result{}, err
		}
		d := p.here(call)
		return sdk.Result{Text: report([]device{d}), Data: d}, nil
	})
	return nil
}

// keepInstalled installs Claude Code and claude-swap once this device is a
// coding device, and updates claude-swap daily. Claude Code updates itself.
func (p *plugin) keepInstalled(call *sdk.Call) {
	var updated time.Time
	for ; ; time.Sleep(time.Minute) {
		if me := p.me(call); me == "" || !p.isCoding(me) {
			continue
		}
		if installed() && time.Since(updated) < 24*time.Hour {
			continue
		}
		if err := install(call.Context); err != nil {
			call.Logf("install: %v", err)
			continue
		}
		updated = time.Now()
		if err := adopt(call.Context); err != nil {
			call.Logf("add the signed-in account to claude-swap: %v", err)
		}
	}
}

type onlineDevice struct {
	Name   string `json:"name"`
	Self   bool   `json:"self"`
	Online bool   `json:"online"`
}

func nodes(call *sdk.Call) ([]onlineDevice, error) {
	res, err := call.Invoke("one", "devices", struct{}{})
	if err != nil {
		return nil, err
	}
	raw, _ := json.Marshal(res.Data)
	var d struct {
		Devices []onlineDevice `json:"devices"`
	}
	return d.Devices, json.Unmarshal(raw, &d)
}

func deviceNames(call *sdk.Call) ([]string, error) {
	ds, err := nodes(call)
	var names []string
	for _, d := range ds {
		names = append(names, d.Name)
	}
	return names, err
}

func selfName(call *sdk.Call) (string, error) {
	ds, err := nodes(call)
	for _, d := range ds {
		if d.Self {
			return d.Name, nil
		}
	}
	if err == nil {
		err = errors.New("this device isn't in the device list")
	}
	return "", err
}
