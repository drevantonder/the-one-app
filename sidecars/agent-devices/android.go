package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"image"
	"image/png"
	"log"
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
//
// Stopping one keeps what it was doing (the emulator's Quick Boot snapshot), and starting it carries on from there in
// seconds, not a boot of half a minute or more; a copy of a stopped one carries on from its snapshot too. The emulator
// boots instead when a snapshot won't load.
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
		// Without its own network, a phone's apps would reach this machine's 127.0.0.1 and adb (start).
		if _, err := exec.LookPath("pasta"); err != nil {
			return "Android needs pasta here (the passt package), for a network of its own"
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

// phoneGB is each phone's memory, whatever its AVD says (the SDK's are 4 GB): Android 36 with the Play Store boots
// in 2, and a machine holding several phones beside builds runs out of memory at 4.
const phoneGB = 2

func (a androidEmulator) make(ctx context.Context, v *vm, from *vm) error {
	v.ID = "agent-devices-" + v.Name
	v.GB = phoneGB
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
	// Stopped, and done saving its snapshot, which it does after adb stops answering.
	whole := from != nil && !a.running(ctx, from) && len(emulators(from)) == 0
	if whole {
		// A stopped lasting device: all of it, its sign-ins and apps too.
		if _, err := run(ctx, "cp", "-a", "--reflink=auto", src, dst); err != nil {
			return err
		}
		locks, _ := filepath.Glob(filepath.Join(dst, "*.lock"))
		for _, l := range locks {
			os.RemoveAll(l)
		}
		// Its snapshots name the AVD they're of, which the emulator loads them only into: now this one.
		snaps, _ := filepath.Glob(filepath.Join(dst, "snapshots", "*", "hardware.ini"))
		for _, f := range snaps {
			b, err := os.ReadFile(f)
			if err != nil {
				continue
			}
			ini := strings.ReplaceAll(string(b), "/"+from.ID+".avd/", "/"+v.ID+".avd/")
			ini = regexp.MustCompile(`(?m)^(avd\.(?:name|id) = )`+regexp.QuoteMeta(from.ID)+`$`).ReplaceAllString(ini, "${1}"+v.ID)
			os.WriteFile(f, []byte(ini), 0o644)
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
	// Its snapshot only if its last stop kept it: loading one also takes its disk back to then, losing what it did
	// since if it ended any other way.
	saved := false
	if dir, err := avdDir(v.ID); err == nil {
		_, err = os.Stat(filepath.Join(dir, kept))
		saved = err == nil
		os.Remove(filepath.Join(dir, kept))
	}
	err := a.boot(ctx, v, saved)
	if err != nil && saved {
		// A snapshot that loads but never finishes starting: it boots instead.
		log.Printf("%s didn't carry on where it stopped, so it boots: %v", v.Name, err)
		a.kill(v)
		err = a.boot(ctx, v, false)
	}
	return err
}

// boot starts the emulator, carrying on from its snapshot with saved, and waits for Android to have started.
func (a androidEmulator) boot(ctx context.Context, v *vm, saved bool) error {
	a.kill(v) // one left starting, or stuck, which adb doesn't answer for: the emulator runs one at a time
	out, err := os.Create(filepath.Join(dataDir, "android-"+v.Name+".log"))
	if err != nil {
		return err
	}
	defer out.Close()
	emulator := []string{filepath.Join(sdk(), "emulator", "emulator"), "-avd", v.ID, "-port", strconv.Itoa(v.Port), "-no-window",
		"-gpu", "swiftshader_indirect", "-accel", "on", "-no-audio", "-no-boot-anim", "-memory", strconv.Itoa(phoneGB * 1024)}
	if !saved {
		emulator = append(emulator, "-no-snapshot-load")
	}
	limit := 5 * time.Minute
	if saved {
		limit = 2 * time.Minute
	}
	// The emulator's own network puts this machine's 127.0.0.1 at 10.0.2.2 for any app in the phone, and has no way not
	// to. On Linux it runs in a network of its own (pasta), reaching the internet as this machine does but none of its
	// 127.0.0.1, which takes only its console and adb's port in; and asks this machine's resolvers, not its stub. A Mac
	// has nothing like it, so there a phone's apps reach the Mac's 127.0.0.1 (ARCHITECTURE.md).
	apart := runtime.GOOS == "linux"
	if apart {
		ports := fmt.Sprintf("127.0.0.1/%d-%d", v.Port, v.Port+1)
		emulator = append([]string{"pasta", "--config-net", "--no-map-gw", "-T", "none", "-U", "none", "-u", "none", "-t", ports, "--quiet", "--"}, emulator...)
		if dns := upstreamDNS(); len(dns) > 0 {
			emulator = append(emulator, "-dns-server", strings.Join(dns, ","))
		}
	}
	// Its own runtime folder, for a netsimd (its radios) of its own: one an emulator in another network started is
	// unreachable here, and a phone that waits for it never starts.
	run := filepath.Join(dataDir, "android-"+v.Name+".run")
	os.MkdirAll(run, 0o700)
	emulator = append([]string{"env", "XDG_RUNTIME_DIR=" + run}, emulator...)
	name, args := outliving(emulator[0], emulator[1:]...)
	cmd := exec.Command(name, args...)
	cmd.Env = append(os.Environ(), "ANDROID_HOME="+sdk(), "ANDROID_SDK_ROOT="+sdk())
	cmd.Stdout, cmd.Stderr = out, out
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := cmd.Start(); err != nil {
		return err
	}
	go cmd.Wait()
	ctx, cancel := context.WithTimeout(ctx, limit)
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
			return fmt.Errorf("%s didn't start in %s: %s", v.Name, limit, filepath.Join(dataDir, "android-"+v.Name+".log"))
		case <-time.After(500 * time.Millisecond):
		}
	}
	adb(ctx, v, "shell", "svc", "power", "stayon", "true")
	adb(ctx, v, "shell", "input", "keyevent", "KEYCODE_WAKEUP")
	adb(ctx, v, "shell", "wm", "dismiss-keyguard")
	setClock(ctx, v)
	return nil
}

// readLog is what the emulator said at its last start.
func readLog(v *vm) string {
	b, _ := os.ReadFile(filepath.Join(dataDir, "android-"+v.Name+".log"))
	return string(b)
}

// setClock sets the phone's clock to this machine's: one carrying on from a pause is behind by as long as it was
// paused, and from a snapshot, can be.
func setClock(ctx context.Context, v *vm) {
	adb(ctx, v, "shell", "cmd", "alarm", "set-time", strconv.FormatInt(time.Now().UnixMilli(), 10))
}

// emulators are the processes running v's AVD: the emulator, and on Linux its pasta.
func emulators(v *vm) []int {
	out, _ := exec.Command("ps", "-axo", "pid=,args=").Output()
	var pids []int
	for _, line := range strings.Split(string(out), "\n") {
		pid, args, _ := strings.Cut(strings.TrimSpace(line), " ")
		if n, err := strconv.Atoi(pid); err == nil && strings.Contains(args+" ", " -avd "+v.ID+" ") {
			pids = append(pids, n)
		}
	}
	return pids
}

// kill ends v's emulator at once, keeping nothing, and the locks it leaves, which would keep the next from starting.
func (androidEmulator) kill(v *vm) {
	for _, pid := range emulators(v) {
		syscall.Kill(pid, syscall.SIGKILL)
	}
	for range 50 {
		if len(emulators(v)) == 0 {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if dir, err := avdDir(v.ID); err == nil {
		locks, _ := filepath.Glob(filepath.Join(dir, "*.lock"))
		for _, l := range locks {
			os.RemoveAll(l)
		}
	}
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

// kept, in an AVD's folder, says its snapshot is what it was doing when it last stopped.
const kept = "agent-devices-kept"

// stop ends the emulator, which keeps what the phone was doing as it goes, a few seconds' work it's waited for: a
// copy taken before it's done would be half of one.
func (a androidEmulator) stop(ctx context.Context, v *vm) error {
	if !a.running(ctx, v) {
		a.kill(v) // starting, or stuck: nothing to keep
		return nil
	}
	if _, err := adb(ctx, v, "emu", "kill"); err != nil {
		return err
	}
	for range 600 {
		if len(emulators(v)) == 0 {
			if dir, err := avdDir(v.ID); err == nil && strings.Contains(readLog(v), "Saving snapshot 'default_boot' took") {
				os.WriteFile(filepath.Join(dir, kept), nil, 0o644)
			}
			return nil
		}
		time.Sleep(200 * time.Millisecond)
	}
	a.kill(v)
	return fmt.Errorf("%s took over 2 minutes to stop, so it was ended: it boots at its next start", v.Name)
}

func (androidEmulator) pause(ctx context.Context, v *vm) error {
	_, err := adb(ctx, v, "emu", "avd", "pause")
	return err
}

func (androidEmulator) resume(ctx context.Context, v *vm) error {
	if _, err := adb(ctx, v, "emu", "avd", "resume"); err != nil {
		return err
	}
	setClock(ctx, v)
	return nil
}

func (a androidEmulator) remove(ctx context.Context, v *vm) error {
	a.kill(v) // nothing to keep
	dir, err := avdDir(v.ID)
	if err == nil {
		os.RemoveAll(dir)
	}
	os.Remove(filepath.Join(avdHome(), v.ID+".ini"))
	os.Remove(filepath.Join(dataDir, "android-"+v.Name+".log"))
	os.RemoveAll(filepath.Join(dataDir, "android-"+v.Name+".run"))
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
