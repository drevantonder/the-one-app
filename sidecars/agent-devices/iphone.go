package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
)

// iosSimulator runs iPhones and iPads in Xcode's Simulator on a Mac, each a simulator of its own (simctl), with no
// window unless the person opens Simulator. Simulator takes no clicks or typing from outside it, so agents get
// pictures, URLs, apps and commands, and the person uses it in Simulator on the Mac. A copy is a clone of a
// stopped lasting device, or with the lasting one running, a new simulator of the same model.
type iosSimulator struct{}

func (iosSimulator) name() string  { return "iphone" }
func (iosSimulator) title() string { return "iPhone" }
func (iosSimulator) limits() string {
	return "no clicks, typing or keys: Simulator takes none from agents. Pictures, open_url, install_app (an .app built for the simulator) and run_command (a program in the simulator) work; the person uses it in Simulator on the Mac"
}
func (iosSimulator) input() bool      { return false }
func (iosSimulator) port() (int, int) { return 0, 0 }

func (iosSimulator) can() string {
	if runtime.GOOS != "darwin" {
		return "iPhones run on Macs with Xcode"
	}
	if err := exec.Command("xcrun", "simctl", "help").Run(); err != nil {
		return "iPhones need Xcode's simctl here"
	}
	return ""
}

func (iosSimulator) adopt(ctx context.Context, v *vm) error {
	if _, _, ok := simulatorOf(ctx, v.ID); !ok {
		return errors.New("give id (its simulator's UDID)")
	}
	return nil
}

func simctl(ctx context.Context, args ...string) (string, error) {
	return run(ctx, "xcrun", append([]string{"simctl"}, args...)...)
}

type simulator struct {
	UDID       string `json:"udid"`
	Name       string `json:"name"`
	State      string `json:"state"`
	DeviceType string `json:"deviceTypeIdentifier"`
}

// simulators are every simulator here, by runtime.
func simulators(ctx context.Context) (map[string][]simulator, error) {
	out, err := simctl(ctx, "list", "--json", "devices")
	if err != nil {
		return nil, err
	}
	var got struct{ Devices map[string][]simulator }
	return got.Devices, json.Unmarshal([]byte(out), &got)
}

func simulatorOf(ctx context.Context, udid string) (simulator, string, bool) {
	all, _ := simulators(ctx)
	for rt, sims := range all {
		for _, s := range sims {
			if s.UDID == udid {
				return s, rt, true
			}
		}
	}
	return simulator{}, "", false
}

func (s iosSimulator) make(ctx context.Context, v *vm, from *vm) error {
	name := "Agent Devices " + v.Name
	v.GB = 2
	if from != nil {
		src, rt, ok := simulatorOf(ctx, from.ID)
		if !ok {
			return fmt.Errorf("%s's simulator is gone", from.Name)
		}
		if src.State == "Shutdown" {
			out, err := simctl(ctx, "clone", from.ID, name)
			v.ID = strings.TrimSpace(out)
			return err
		}
		out, err := simctl(ctx, "create", name, src.DeviceType, rt)
		v.ID = strings.TrimSpace(out)
		return err
	}
	// The newest iPhone Pro with the newest iOS.
	out, err := simctl(ctx, "list", "--json", "runtimes", "devicetypes")
	if err != nil {
		return err
	}
	var got struct {
		Runtimes []struct {
			Identifier  string
			Platform    string
			IsAvailable bool
			Version     string
		}
		DeviceTypes []struct{ Identifier, Name string }
	}
	if err := json.Unmarshal([]byte(out), &got); err != nil {
		return err
	}
	var rt, kind string
	for _, r := range got.Runtimes {
		if r.Platform == "iOS" && r.IsAvailable {
			rt = r.Identifier
		}
	}
	for _, d := range slices.Backward(got.DeviceTypes) {
		if strings.HasPrefix(d.Name, "iPhone") && strings.HasSuffix(d.Name, " Pro") {
			kind = d.Identifier
			break
		}
	}
	if rt == "" || kind == "" {
		return errors.New("Xcode here has no iOS runtime or iPhone to make one with")
	}
	out, err = simctl(ctx, "create", name, kind, rt)
	v.ID = strings.TrimSpace(out)
	return err
}

func (iosSimulator) running(ctx context.Context, v *vm) bool {
	s, _, ok := simulatorOf(ctx, v.ID)
	return ok && s.State == "Booted"
}

func (iosSimulator) start(ctx context.Context, v *vm) error {
	if _, err := simctl(ctx, "boot", v.ID); err != nil && !strings.Contains(err.Error(), "current state: Booted") {
		return err
	}
	_, err := simctl(ctx, "bootstatus", v.ID, "-b")
	return err
}

func (iosSimulator) stop(ctx context.Context, v *vm) error {
	_, err := simctl(ctx, "shutdown", v.ID)
	if err != nil && strings.Contains(err.Error(), "current state: Shutdown") {
		return nil
	}
	return err
}

// pause shuts it down: Simulator can't pause one.
func (s iosSimulator) pause(ctx context.Context, v *vm) error { return s.stop(ctx, v) }

func (s iosSimulator) resume(ctx context.Context, v *vm) error { return s.start(ctx, v) }

func (s iosSimulator) remove(ctx context.Context, v *vm) error {
	s.stop(ctx, v)
	_, err := simctl(ctx, "delete", v.ID)
	return err
}

func (iosSimulator) screen(ctx context.Context, v *vm) (image.Image, error) {
	f, err := os.CreateTemp("", "agent-devices-screen-*.png")
	if err != nil {
		return nil, err
	}
	f.Close()
	defer os.Remove(f.Name())
	if _, err := simctl(ctx, "io", v.ID, "screenshot", "--type=png", f.Name()); err != nil {
		return nil, err
	}
	return decodeFile(f.Name())
}

func (s iosSimulator) act(context.Context, *vm, action) error {
	return errors.New("Simulator takes no clicks, typing or keys from agents: open_url, install_app and run_command work, and the person uses it in Simulator on the Mac")
}

func (iosSimulator) open(ctx context.Context, v *vm, url string) error {
	_, err := simctl(ctx, "openurl", v.ID, url)
	return err
}

func (iosSimulator) install(ctx context.Context, v *vm, app string) (string, error) {
	if !strings.HasSuffix(strings.TrimSuffix(app, "/"), ".app") {
		return "", errors.New("an iPhone simulator installs an .app built for the simulator, given as its folder on the Mac")
	}
	if _, err := simctl(ctx, "install", v.ID, app); err != nil {
		return "", err
	}
	return fmt.Sprintf("Installed %s.", filepath.Base(app)), nil
}

func (iosSimulator) reach(ctx context.Context, v *vm, port int) (string, error) {
	return fmt.Sprintf("%s shares this Mac's network, so it reaches 127.0.0.1:%d already.", v.Name, port), nil
}

func (iosSimulator) run(ctx context.Context, v *vm, command string) (string, error) {
	// The simulator has no shell: the command's first word is a program in it, the rest its arguments.
	args := strings.Fields(command)
	if len(args) == 0 {
		return "", errors.New("run what? a program in the simulator, like launchctl list")
	}
	return simctl(ctx, append([]string{"spawn", v.ID}, args...)...)
}
