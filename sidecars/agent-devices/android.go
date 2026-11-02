package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"image"
	"image/png"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// androidEmulator runs Android phones in the Android SDK's emulator, on a Linux x86 machine with KVM or on a Mac, with
// no window: each an AVD of the SDK's, reached with adb at its console's port. A lasting device is an AVD of its
// own; a copy is an AVD made for one loan, from a copy of a stopped lasting device's (reflinked where the disk can),
// or with the lasting one running, a new phone from the same system image, deleted when it's given back.
type androidEmulator struct{}

func (androidEmulator) name() string  { return "android" }
func (androidEmulator) title() string { return "Android" }
func (androidEmulator) limits() string {
	return "taps, swipes, typing and keys, and shell commands; the person watches and uses it through view"
}
func (androidEmulator) input() bool      { return true }
func (androidEmulator) port() (int, int) { return 5560, 2 }

// sdk is the Android SDK here, or "".
func sdk() string {
	home, _ := os.UserHomeDir()
	for _, d := range []string{os.Getenv("ANDROID_HOME"), os.Getenv("ANDROID_SDK_ROOT"), filepath.Join(home, "Android", "Sdk"), filepath.Join(home, "Library", "Android", "sdk")} {
		if d == "" {
			continue
		}
		if _, err := os.Stat(filepath.Join(d, "emulator", "emulator")); err == nil {
			return d
		}
	}
	return ""
}

func avdHome() string {
	if d := os.Getenv("ANDROID_AVD_HOME"); d != "" {
		return d
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".android", "avd")
}

func (androidEmulator) can() string {
	switch runtime.GOOS {
	case "linux":
		if runtime.GOARCH != "amd64" {
			return "Android runs on Linux x86 machines with KVM, and on Macs"
		}
		if f, err := os.OpenFile("/dev/kvm", os.O_RDWR, 0); err != nil {
			return "Android needs KVM (/dev/kvm) here"
		} else {
			f.Close()
		}
	case "darwin":
	default:
		return "Android runs on Linux x86 machines with KVM, and on Macs"
	}
	if sdk() == "" {
		return "Android needs the Android SDK's emulator here (ANDROID_HOME)"
	}
	if avds, _ := filepath.Glob(filepath.Join(avdHome(), "*.ini")); len(avds) == 0 {
		return "Android needs an Android Virtual Device here to start from"
	}
	return ""
}

func (androidEmulator) adopt(_ context.Context, v *vm) error {
	if _, err := avdDir(v.ID); err != nil || v.Port == 0 {
		return errors.New("give id (its AVD's name) and port (its console port, like 5554)")
	}
	return nil
}

func adb(ctx context.Context, v *vm, args ...string) (string, error) {
	return run(ctx, filepath.Join(sdk(), "platform-tools", "adb"), append([]string{"-s", fmt.Sprintf("emulator-%d", v.Port)}, args...)...)
}

var avdPath = regexp.MustCompile(`(?m)^path=(.*)$`)

// avdDir is an AVD's folder, as its .ini says.
func avdDir(name string) (string, error) {
	b, err := os.ReadFile(filepath.Join(avdHome(), name+".ini"))
	if err != nil {
		return "", fmt.Errorf("no Android Virtual Device %s here", name)
	}
	m := avdPath.FindSubmatch(b)
	if m == nil {
		return "", fmt.Errorf("%s.ini says no path", name)
	}
	return strings.TrimSpace(string(m[1])), nil
}

func (a androidEmulator) make(ctx context.Context, v *vm, from *vm) error {
	v.ID = "agent-devices-" + v.Name
	v.GB = 4
	var src string
	if from != nil {
		var err error
		if src, err = avdDir(from.ID); err != nil {
			return err
		}
	} else {
		// A new phone from the system image of the first AVD here.
		avds, _ := filepath.Glob(filepath.Join(avdHome(), "*.ini"))
		if len(avds) == 0 {
			return errors.New("there's no Android Virtual Device here to start from")
		}
		var err error
		if src, err = avdDir(strings.TrimSuffix(filepath.Base(avds[0]), ".ini")); err != nil {
			return err
		}
	}
	dst := filepath.Join(avdHome(), v.ID+".avd")
	whole := from != nil && !a.running(ctx, from)
	if whole {
		// A stopped lasting device: all of it, its sign-ins and apps too.
		if _, err := run(ctx, "cp", "-a", "--reflink=auto", src, dst); err != nil {
			return err
		}
		locks, _ := filepath.Glob(filepath.Join(dst, "*.lock"))
		for _, l := range locks {
			os.RemoveAll(l)
		}
	} else {
		// Only its settings: a new phone, the same model and system.
		if err := os.MkdirAll(dst, 0o755); err != nil {
			return err
		}
		if _, err := run(ctx, "cp", filepath.Join(src, "config.ini"), filepath.Join(dst, "config.ini")); err != nil {
			return err
		}
	}
	ini := fmt.Sprintf("avd.ini.encoding=UTF-8\npath=%s\npath.rel=avd/%s.avd\ntarget=%s\n", dst, v.ID, target(src))
	return os.WriteFile(filepath.Join(avdHome(), v.ID+".ini"), []byte(ini), 0o644)
}

// target is the Android version of an AVD's system image, as its .ini names it.
func target(dir string) string {
	b, _ := os.ReadFile(filepath.Join(dir, "config.ini"))
	if m := regexp.MustCompile(`(?m)^image\.sysdir\.1=system-images/([^/]+)/`).FindSubmatch(b); m != nil {
		return string(m[1])
	}
	return "android-36"
}

func (androidEmulator) running(ctx context.Context, v *vm) bool {
	out, err := adb(ctx, v, "get-state")
	return err == nil && strings.TrimSpace(out) == "device"
}

func (a androidEmulator) start(ctx context.Context, v *vm) error {
	log, err := os.Create(filepath.Join(dataDir, "android-"+v.Name+".log"))
	if err != nil {
		return err
	}
	defer log.Close()
	emulator := []string{filepath.Join(sdk(), "emulator", "emulator"), "-avd", v.ID, "-port", strconv.Itoa(v.Port), "-no-window",
		"-gpu", "swiftshader_indirect", "-accel", "on", "-no-audio", "-no-boot-anim", "-no-snapshot"}
	// The emulator's own network puts this machine's 127.0.0.1 at 10.0.2.2 for any app in the phone, and has no way not
	// to. On Linux it runs in a network of its own (pasta), reaching the internet as this machine does but none of its
	// 127.0.0.1, which takes only its console and adb's port in; and asks this machine's resolvers, not its stub. A Mac
	// has nothing like it, so there a phone's apps reach the Mac's 127.0.0.1 (ARCHITECTURE.md).
	_, err = exec.LookPath("pasta")
	apart := err == nil && runtime.GOOS == "linux"
	if apart {
		ports := fmt.Sprintf("127.0.0.1/%d-%d", v.Port, v.Port+1)
		emulator = append([]string{"pasta", "--config-net", "--no-map-gw", "-T", "none", "-U", "none", "-u", "none", "-t", ports, "--quiet", "--"}, emulator...)
		if dns := upstreamDNS(); len(dns) > 0 {
			emulator = append(emulator, "-dns-server", strings.Join(dns, ","))
		}
	}
	name, args := outliving(emulator[0], emulator[1:]...)
	cmd := exec.Command(name, args...)
	cmd.Env = append(os.Environ(), "ANDROID_HOME="+sdk(), "ANDROID_SDK_ROOT="+sdk())
	cmd.Stdout, cmd.Stderr = log, log
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := cmd.Start(); err != nil {
		return err
	}
	go cmd.Wait()
	ctx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	for {
		if apart {
			announce(ctx, v.Port+1)
		}
		if out, err := adb(ctx, v, "shell", "getprop", "sys.boot_completed"); err == nil && strings.TrimSpace(out) == "1" {
			break
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("%s didn't start in 5 minutes: %s", v.Name, filepath.Join(dataDir, "android-"+v.Name+".log"))
		case <-time.After(2 * time.Second):
		}
	}
	adb(ctx, v, "shell", "svc", "power", "stayon", "true")
	adb(ctx, v, "shell", "input", "keyevent", "KEYCODE_WAKEUP")
	adb(ctx, v, "shell", "wm", "dismiss-keyguard")
	return nil
}

// announce tells adb's server of an emulator with adb at port, as the emulator itself would if it could reach the
// server: in a network of its own it can't. The server takes an emulator it knows already as told once.
func announce(ctx context.Context, port int) {
	run(ctx, filepath.Join(sdk(), "platform-tools", "adb"), "start-server")
	c, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", adbPort), time.Second)
	if err != nil {
		return
	}
	defer c.Close()
	msg := fmt.Sprintf("host:emulator:%d", port)
	fmt.Fprintf(c, "%04x%s", len(msg), msg)
}

func (a androidEmulator) stop(ctx context.Context, v *vm) error {
	if !a.running(ctx, v) {
		return nil
	}
	if _, err := adb(ctx, v, "emu", "kill"); err != nil {
		return err
	}
	for range 30 {
		if !a.running(ctx, v) {
			break
		}
		time.Sleep(time.Second)
	}
	return nil
}

func (androidEmulator) pause(ctx context.Context, v *vm) error {
	_, err := adb(ctx, v, "emu", "avd", "pause")
	return err
}

func (androidEmulator) resume(ctx context.Context, v *vm) error {
	_, err := adb(ctx, v, "emu", "avd", "resume")
	return err
}

func (a androidEmulator) remove(ctx context.Context, v *vm) error {
	a.stop(ctx, v)
	dir, err := avdDir(v.ID)
	if err == nil {
		os.RemoveAll(dir)
	}
	os.Remove(filepath.Join(avdHome(), v.ID+".ini"))
	os.Remove(filepath.Join(dataDir, "android-"+v.Name+".log"))
	return nil
}

func (androidEmulator) screen(ctx context.Context, v *vm) (image.Image, error) {
	cmd := exec.CommandContext(ctx, filepath.Join(sdk(), "platform-tools", "adb"), "-s", fmt.Sprintf("emulator-%d", v.Port), "exec-out", "screencap", "-p")
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("screencap: %w", err)
	}
	return png.Decode(bytes.NewReader(out))
}

func (androidEmulator) act(ctx context.Context, v *vm, a action) error {
	n := func(f float64) string { return strconv.Itoa(int(f)) }
	var err error
	switch a.Do {
	case "click":
		_, err = adb(ctx, v, "shell", "input", "tap", n(a.X), n(a.Y))
	case "double_click":
		adb(ctx, v, "shell", "input", "tap", n(a.X), n(a.Y))
		_, err = adb(ctx, v, "shell", "input", "tap", n(a.X), n(a.Y))
	case "right_click":
		_, err = adb(ctx, v, "shell", "input", "swipe", n(a.X), n(a.Y), n(a.X), n(a.Y), "800")
	case "scroll", "swipe":
		_, err = adb(ctx, v, "shell", "input", "swipe", n(a.X), n(a.Y), n(a.ToX), n(a.ToY), "300")
	case "type":
		// input text takes no spaces, and the device's shell reads the rest.
		_, err = adb(ctx, v, "shell", "input", "text", quote(strings.ReplaceAll(a.Text, " ", "%s")))
	case "key":
		_, err = adb(ctx, v, "shell", "input", "keyevent", androidKey(a.Key))
	default:
		err = fmt.Errorf("no action %q", a.Do)
	}
	return err
}

// androidKey is a key as Android names it: enter, back, home, or KEYCODE_ anything.
func androidKey(key string) string {
	k := strings.ToUpper(strings.TrimPrefix(strings.ToLower(key), "keycode_"))
	names := map[string]string{"RETURN": "ENTER", "ESCAPE": "BACK", "ESC": "BACK", "BACKSPACE": "DEL", "DELETE": "FORWARD_DEL", "UP": "DPAD_UP",
		"DOWN": "DPAD_DOWN", "LEFT": "DPAD_LEFT", "RIGHT": "DPAD_RIGHT", "RECENTS": "APP_SWITCH", "POWER": "POWER", "SPACE": "SPACE"}
	if n, ok := names[k]; ok {
		k = n
	}
	return "KEYCODE_" + k
}

func (androidEmulator) open(ctx context.Context, v *vm, url string) error {
	_, err := adb(ctx, v, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", quote(url))
	return err
}

func (androidEmulator) install(ctx context.Context, v *vm, app string) (string, error) {
	if app == "" {
		return "", errors.New("install what? an .apk file here or its URL")
	}
	if !strings.ContainsAny(app, "/") {
		// A package: only the Play Store installs it, signed in, by the person or an agent tapping Install.
		_, err := adb(ctx, v, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", quote("market://details?id="+app))
		return "Opened its page in the Play Store: tap Install there (it needs a Google account signed in).", err
	}
	path, temp, err := fetch(ctx, app)
	if err != nil {
		return "", err
	}
	if temp {
		defer os.RemoveAll(filepath.Dir(path))
	}
	return adb(ctx, v, "install", "-r", "-g", path)
}

func (androidEmulator) reach(ctx context.Context, v *vm, port int) (string, error) {
	p := "tcp:" + strconv.Itoa(port)
	if _, err := adb(ctx, v, "reverse", p, p); err != nil {
		return "", err
	}
	return fmt.Sprintf("%s reaches this machine's port %d at 127.0.0.1:%d.", v.Name, port, port), nil
}

func (androidEmulator) run(ctx context.Context, v *vm, command string) (string, error) {
	return adb(ctx, v, "shell", command)
}
