package main

import (
	"bufio"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"image"
	_ "image/png"
	"io"
	"log"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// windowsQEMU runs Windows 11 in QEMU on a Linux x86 machine with KVM: UEFI Secure Boot (OVMF) and a TPM (swtpm), as
// Windows 11 needs. A device's folder has its disk, its UEFI variables and its TPM; the machine reaches it through
// QEMU's monitor (pictures, mouse and keys), and through SSH forwarded to a port here (commands), as the Windows
// account one that trusts this machine's key. Its network is passt's, which reaches the internet as this machine does
// but none of this machine's own 127.0.0.1, where QEMU's own would put it at 10.0.2.2 for any program in Windows to
// reach what only this machine's programs use; only its SSH comes in. A copy is a disk on top of a lasting device's base: a copy of its
// disk taken when it's copied, kept for the next copy until the lasting device next stops.
//
// Stopping one keeps what it was doing (its memory and devices, as QEMU migrates them, the TPM's state too) in its
// folder, beside the disk as it was then, and starting it carries on from there in seconds, not a boot of a minute
// or more; a copy of a stopped device starts from its base's. Its disk is on AHCI, not NVMe, which QEMU can't
// migrate.
type windowsQEMU struct{}

const (
	ovmfCode    = "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd"
	windowsUser = "one"
)

func (windowsQEMU) name() string  { return "windows" }
func (windowsQEMU) title() string { return "Windows" }
func (windowsQEMU) limits() string {
	return "mouse, keyboard and commands; the person watches and uses it through view"
}
func (windowsQEMU) input() bool      { return true }
func (windowsQEMU) port() (int, int) { return 22220, 1 }

func (windowsQEMU) can() string {
	if runtime.GOOS != "linux" || runtime.GOARCH != "amd64" {
		return "Windows runs on Linux x86 machines with KVM"
	}
	if f, err := os.OpenFile("/dev/kvm", os.O_RDWR, 0); err != nil {
		return "Windows needs KVM (/dev/kvm) here"
	} else {
		f.Close()
	}
	for _, c := range []string{"qemu-system-x86_64", "qemu-img", "swtpm", "passt"} {
		if _, err := exec.LookPath(c); err != nil {
			return "Windows needs " + c + " here"
		}
	}
	if _, err := os.Stat(ovmfCode); err != nil {
		return "Windows needs OVMF with Secure Boot here (" + ovmfCode + ")"
	}
	return ""
}

func (windowsQEMU) adopt(_ context.Context, v *vm) error {
	if _, err := os.Stat(winFile(v, "disk.qcow2")); err != nil || v.Port == 0 || v.Display == 0 {
		return errors.New("give dir (its folder, with its disk.qcow2, OVMF_VARS.fd, tpm and monitor.sock), port (its SSH, forwarded here) and display (its VNC display)")
	}
	return nil
}

func winFile(v *vm, name string) string { return filepath.Join(v.Dir, name) }

func (w windowsQEMU) make(ctx context.Context, v *vm, from *vm) error {
	if from == nil {
		return errors.New("a new Windows device is made from scratch only by installing Windows, which isn't built yet: borrow a copy of one instead")
	}
	v.Dir = filepath.Join(dataDir, "windows", v.Name)
	v.Display = v.Port - 22220 + 10
	// The size of the device it copies, which its saved state is for.
	v.CPUs, v.GB = from.CPUs, from.GB
	if err := os.MkdirAll(v.Dir, 0o700); err != nil {
		return err
	}
	base, err := w.base(ctx, from)
	if err != nil {
		os.RemoveAll(v.Dir)
		return err
	}
	steps := [][]string{
		{"qemu-img", "create", "-q", "-f", "qcow2", "-b", base, "-F", "qcow2", winFile(v, "disk.qcow2")},
		{"cp", winFile(from, "base-vars.fd"), winFile(v, "OVMF_VARS.fd")},
		{"cp", "-a", winFile(from, "base-tpm"), winFile(v, "tpm")},
	}
	for _, s := range steps {
		if _, err := run(ctx, s[0], s[1:]...); err != nil {
			os.RemoveAll(v.Dir)
			return err
		}
	}
	// It starts from the base's saved state, if it has one: a link, which its start removes, leaving the base's.
	if id, err := os.ReadFile(winFile(from, "base-state.id")); err == nil && os.Link(winFile(from, "base-state"), winFile(v, "state")) == nil {
		os.WriteFile(winFile(v, "state.id"), id, 0o600)
	}
	return nil
}

// base is the lasting device's base disk, which copies are made on: taken now if it has none. A running device's
// is copied by QEMU as it runs, as a sudden power cut would leave it, which a throwaway copy recovers from.
func (w windowsQEMU) base(ctx context.Context, from *vm) (string, error) {
	base := winFile(from, "base.qcow2")
	if _, err := os.Stat(base); err == nil {
		return base, nil
	}
	tmp := base + ".part"
	os.Remove(tmp)
	os.Remove(winFile(from, "base-state"))
	os.Remove(winFile(from, "base-state.id"))
	if w.running(ctx, from) {
		if _, err := monitor(from, "drive_backup -f d0 "+tmp+" qcow2"); err != nil {
			return "", err
		}
		for {
			out, err := monitor(from, "info block-jobs")
			if err != nil {
				return "", err
			}
			if strings.Contains(out, "No active jobs") {
				break
			}
			select {
			case <-ctx.Done():
				return "", ctx.Err()
			case <-time.After(2 * time.Second):
			}
		}
		if _, err := os.Stat(tmp); err != nil {
			return "", fmt.Errorf("copying %s's disk failed", from.Name)
		}
	} else {
		if _, err := run(ctx, "cp", "--reflink=auto", winFile(from, "disk.qcow2"), tmp); err != nil {
			return "", err
		}
		// Stopped, its saved state goes with this disk.
		if id, err := os.ReadFile(winFile(from, "state.id")); err == nil && os.Link(winFile(from, "state"), winFile(from, "base-state")) == nil {
			os.WriteFile(winFile(from, "base-state.id"), id, 0o600)
		}
	}
	os.RemoveAll(winFile(from, "base-tpm"))
	if _, err := run(ctx, "cp", winFile(from, "OVMF_VARS.fd"), winFile(from, "base-vars.fd")); err != nil {
		return "", err
	}
	if _, err := run(ctx, "cp", "-a", winFile(from, "tpm"), winFile(from, "base-tpm")); err != nil {
		return "", err
	}
	os.Remove(winFile(from, "base-tpm/sock"))
	return base, os.Rename(tmp, base)
}

func (windowsQEMU) running(_ context.Context, v *vm) bool {
	b, err := os.ReadFile(winFile(v, "qemu.pid"))
	if err != nil {
		return false
	}
	pid, _ := strconv.Atoi(strings.TrimSpace(string(b)))
	return pid > 0 && syscall.Kill(pid, 0) == nil
}

// start carries on from where it was stopped, or boots it if it can't.
func (w windowsQEMU) start(ctx context.Context, v *vm) error {
	if err := w.carryOn(ctx, v); err == nil {
		return nil
	} else if !errors.Is(err, errNoState) {
		log.Printf("%s didn't carry on where it stopped, so it boots: %v", v.Name, err)
	}
	if err := w.launch(ctx, v, ""); err != nil {
		return err
	}
	// Ready once it answers over SSH: it signs in by itself.
	if err := w.answers(ctx, v, 4*time.Minute, 5*time.Second); err != nil {
		return fmt.Errorf("%s started but didn't answer in 4 minutes: look at it with screen", v.Name)
	}
	return nil
}

var errNoState = errors.New("no saved state")

// carryOn starts it from its saved state, which goes once loaded: Windows changes its disk from then on.
func (w windowsQEMU) carryOn(ctx context.Context, v *vm) error {
	state := winFile(v, "state")
	if _, err := os.Stat(state); err != nil {
		return errNoState
	}
	defer w.forget(v)
	id, _ := os.ReadFile(winFile(v, "state.id"))
	if want := w.machine(ctx, v); want == "" || string(id) != want {
		return errors.New("it was saved by another QEMU, or at another size")
	}
	if err := w.launch(ctx, v, state); err != nil {
		w.halt(v)
		return err
	}
	for start := time.Now(); ; time.Sleep(100 * time.Millisecond) {
		status, err := monitor(v, "info status")
		if err != nil || !w.running(ctx, v) {
			w.halt(v)
			return fmt.Errorf("loading its state: %v", err)
		}
		if !strings.Contains(status, "inmigrate") {
			break
		}
		if time.Since(start) > 2*time.Minute {
			w.halt(v)
			return errors.New("loading its state took over 2 minutes")
		}
	}
	w.forget(v)
	if _, err := monitor(v, "cont"); err != nil {
		w.halt(v)
		return err
	}
	// Its SSH server answering is enough: a command over SSH takes seconds more, Windows starting PowerShell.
	if err := w.greets(ctx, v, time.Minute); err != nil {
		w.halt(v)
		return errors.New("it carried on but its SSH didn't answer in a minute")
	}
	go w.setClock(v)
	return nil
}

// greets waits for its SSH server's greeting.
func (windowsQEMU) greets(ctx context.Context, v *vm, limit time.Duration) error {
	ctx, cancel := context.WithTimeout(ctx, limit)
	defer cancel()
	for {
		if c, err := net.DialTimeout("tcp", "127.0.0.1:"+strconv.Itoa(v.Port), time.Second); err == nil {
			c.SetDeadline(time.Now().Add(2 * time.Second))
			b := make([]byte, 4)
			_, err = io.ReadFull(c, b)
			c.Close()
			if err == nil && string(b) == "SSH-" {
				return nil
			}
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(100 * time.Millisecond):
		}
	}
}

// forget deletes its saved state.
func (windowsQEMU) forget(v *vm) {
	os.Remove(winFile(v, "state"))
	os.Remove(winFile(v, "state.id"))
}

// answers waits for it to answer over SSH.
func (w windowsQEMU) answers(ctx context.Context, v *vm, limit, every time.Duration) error {
	ctx, cancel := context.WithTimeout(ctx, limit)
	defer cancel()
	for {
		if _, err := w.ssh(ctx, v, "echo ready"); err == nil {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(every):
		}
	}
}

// setClock sets Windows' clock to this machine's: one carrying on from a pause or a saved state is behind by as long
// as it was stopped. It takes seconds, which nothing waits for: the time goes once PowerShell asks for it.
func (w windowsQEMU) setClock(v *vm) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	script := "[Console]::Out.WriteLine('time?'); [Console]::Out.Flush(); " +
		"Set-Date -Date ([DateTimeOffset]::FromUnixTimeMilliseconds([long]@($input)[0]).LocalDateTime) | Out-Null"
	var utf16 []byte
	for _, r := range script {
		utf16 = append(utf16, byte(r), byte(r>>8))
	}
	cmd := exec.CommandContext(ctx, "ssh", append(w.sshArgs(v, "-p"), windowsUser+"@127.0.0.1",
		"powershell -NoProfile -NonInteractive -EncodedCommand "+base64.StdEncoding.EncodeToString(utf16))...)
	in, _ := cmd.StdinPipe()
	out, _ := cmd.StdoutPipe()
	err := cmd.Start()
	if err == nil {
		line, rerr := bufio.NewReader(out).ReadString('\n')
		if strings.TrimSpace(line) == "time?" {
			fmt.Fprintf(in, "%d\n", time.Now().UnixMilli())
		} else {
			err = fmt.Errorf("it said %q: %v", line, rerr)
		}
		in.Close()
		if werr := cmd.Wait(); err == nil {
			err = werr
		}
	}
	if err != nil {
		log.Printf("setting %s's clock: %v", v.Name, err)
	}
}

// launch starts its TPM, its network and QEMU, booting it, or with incoming, loading that saved state and waiting.
func (w windowsQEMU) launch(ctx context.Context, v *vm, incoming string) error {
	os.MkdirAll(winFile(v, "tpm"), 0o700)
	if _, err := runOutliving(ctx, "swtpm", "socket", "--tpm2", "--tpmstate", "dir="+winFile(v, "tpm"), "--ctrl", "type=unixio,path="+winFile(v, "tpm/sock"),
		"--terminate", "--daemon", "--log", "file="+winFile(v, "tpm/log")); err != nil {
		return err
	}
	// One passt for each start, which ends when QEMU does; one left by a start that failed goes first, with its port.
	if b, err := os.ReadFile(winFile(v, "passt.pid")); err == nil {
		if pid, _ := strconv.Atoi(strings.TrimSpace(string(b))); pid > 0 {
			syscall.Kill(pid, syscall.SIGTERM)
			time.Sleep(time.Second)
		}
	}
	sock := winFile(v, "passt.sock")
	os.Remove(sock)
	args := []string{"--one-off", "--no-map-gw", "--socket", sock, "--pid", winFile(v, "passt.pid"), "-t", fmt.Sprintf("127.0.0.1/%d:22", v.Port), "--quiet"}
	for _, dns := range upstreamDNS() {
		args = append(args, "-D", dns)
	}
	if _, err := runOutliving(ctx, "passt", args...); err != nil {
		return err
	}
	qemu := append([]string{"-name", "agent-devices-" + v.Name, "-daemonize", "-pidfile", winFile(v, "qemu.pid")}, w.hardware(v)...)
	if incoming != "" {
		qemu = append(qemu, "-incoming", "file:"+incoming, "-S")
	}
	_, err := runOutliving(ctx, "qemu-system-x86_64", qemu...)
	return err
}

// hardware is the machine QEMU makes for it, which a saved state must be for.
func (windowsQEMU) hardware(v *vm) []string {
	cpus, gb := max(v.CPUs, 2), max(v.GB, 4)
	return []string{"-machine", "q35,smm=on,accel=kvm", "-global", "driver=cfi.pflash01,property=secure,value=on",
		"-cpu", "host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time", "-smp", fmt.Sprintf("%d,sockets=1,cores=%d,threads=1", cpus, cpus), "-m", fmt.Sprintf("%dG", gb),
		"-drive", "if=pflash,format=raw,unit=0,file=" + ovmfCode + ",readonly=on",
		"-drive", "if=pflash,format=raw,unit=1,file=" + winFile(v, "OVMF_VARS.fd"),
		"-chardev", "socket,id=chrtpm,path=" + winFile(v, "tpm/sock"), "-tpmdev", "emulator,id=tpm0,chardev=chrtpm", "-device", "tpm-crb,tpmdev=tpm0",
		"-drive", "file=" + winFile(v, "disk.qcow2") + ",if=none,id=d0,discard=unmap", "-device", "ide-hd,drive=d0,bus=ide.0,serial=one,bootindex=0",
		"-netdev", "stream,id=n0,server=off,addr.type=unix,addr.path=" + winFile(v, "passt.sock"), "-device", "e1000e,netdev=n0",
		"-device", "qemu-xhci", "-device", "usb-tablet", "-vga", "std", "-rtc", "base=utc,clock=host",
		"-audio", "none", "-display", "none", "-vnc", "unix:" + winFile(v, "vnc.sock"), "-monitor", "unix:" + winFile(v, "monitor.sock") + ",server,nowait"}
}

// machine names QEMU's version and the machine it makes for v, wherever its folder is: a saved state loads only into
// the same.
func (w windowsQEMU) machine(ctx context.Context, v *vm) string {
	version, err := run(ctx, "qemu-system-x86_64", "--version")
	if err != nil {
		return ""
	}
	version, _, _ = strings.Cut(version, "\n")
	h := sha256.Sum256([]byte(version + "\x00" + strings.ReplaceAll(strings.Join(w.hardware(v), "\x00"), v.Dir, "")))
	return hex.EncodeToString(h[:])
}

// halt ends its QEMU at once, which ends its TPM and network.
func (w windowsQEMU) halt(v *vm) {
	monitor(v, "quit")
	for range 50 {
		if !w.running(context.Background(), v) {
			return
		}
		time.Sleep(100 * time.Millisecond)
	}
}

// save keeps what it's doing in its folder, for its next start to carry on from, and ends it.
func (w windowsQEMU) save(ctx context.Context, v *vm) error {
	part := winFile(v, "state.part")
	os.Remove(part)
	if _, err := monitor(v, "stop"); err != nil {
		return err
	}
	fail := func(err error) error {
		monitor(v, "migrate_cancel")
		monitor(v, "cont")
		os.Remove(part)
		return err
	}
	monitor(v, "migrate_set_parameter max-bandwidth 100G")
	if _, err := monitor(v, "migrate -d file:"+part); err != nil {
		return fail(err)
	}
	for start := time.Now(); ; time.Sleep(200 * time.Millisecond) {
		out, err := monitor(v, "info migrate")
		if err != nil {
			return fail(err)
		}
		if strings.Contains(out, "Migration status: completed") {
			break
		}
		if !strings.Contains(out, "Migration status: active") && !strings.Contains(out, "Migration status: setup") || time.Since(start) > 3*time.Minute {
			return fail(fmt.Errorf("saving it: %s", out))
		}
	}
	if err := os.WriteFile(winFile(v, "state.id"), []byte(w.machine(ctx, v)), 0o600); err != nil {
		return fail(err)
	}
	if err := os.Rename(part, winFile(v, "state")); err != nil {
		return fail(err)
	}
	w.halt(v)
	return nil
}

// stop keeps what it's doing and ends it, or if that fails, shuts Windows down, as its power button would, and pulls
// the plug if it hasn't in 3 minutes. Its base is taken again at the next copy.
func (w windowsQEMU) stop(ctx context.Context, v *vm) error {
	if !w.running(ctx, v) {
		return nil
	}
	if v.Of == "" {
		os.Remove(winFile(v, "base.qcow2"))
	}
	err := w.save(ctx, v)
	if err == nil {
		return nil
	}
	log.Printf("%s shuts down, as saving what it's doing failed: %v", v.Name, err)
	w.forget(v)
	monitor(v, "system_powerdown")
	for range 90 {
		if !w.running(ctx, v) {
			break
		}
		time.Sleep(2 * time.Second)
	}
	if w.running(ctx, v) {
		monitor(v, "quit")
	}
	return nil
}

func (windowsQEMU) pause(_ context.Context, v *vm) error {
	_, err := monitor(v, "stop")
	return err
}

func (w windowsQEMU) resume(ctx context.Context, v *vm) error {
	if _, err := monitor(v, "cont"); err != nil {
		return err
	}
	go w.setClock(v)
	return nil
}

func (w windowsQEMU) remove(ctx context.Context, v *vm) error {
	if w.running(ctx, v) {
		monitor(v, "quit")
		time.Sleep(time.Second)
	}
	return os.RemoveAll(v.Dir)
}

func (windowsQEMU) screen(ctx context.Context, v *vm) (image.Image, error) {
	f, err := os.CreateTemp("", "agent-devices-screen-*.png")
	if err != nil {
		return nil, err
	}
	f.Close()
	defer os.Remove(f.Name())
	if _, err := monitor(v, "screendump "+f.Name()+" -f png"); err != nil {
		return nil, err
	}
	return decodeFile(f.Name())
}

func (w windowsQEMU) act(ctx context.Context, v *vm, a action) error {
	return vncAct(winFile(v, "vnc.sock"), a)
}

func (w windowsQEMU) open(ctx context.Context, v *vm, url string) error {
	// Windows shows nothing an SSH session starts, so a task signed in as the person's account opens it.
	_, err := w.ssh(ctx, v, `$a = New-ScheduledTaskAction -Execute 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe' -Argument '--start-maximized `+psQuote(url)+`'
$p = New-ScheduledTaskPrincipal -UserId `+windowsUser+` -LogonType Interactive
Register-ScheduledTask -TaskName agent-devices-open -Action $a -Principal $p -Force | Out-Null
Start-ScheduledTask -TaskName agent-devices-open`)
	return err
}

func (w windowsQEMU) install(ctx context.Context, v *vm, app string) (string, error) {
	if app == "" {
		return "", errors.New("install what? a file, a URL or a winget id")
	}
	if !strings.ContainsAny(app, `/\`) {
		return w.ssh(ctx, v, "winget install --silent --accept-package-agreements --accept-source-agreements --id "+psQuote(app))
	}
	path, temp, err := fetch(ctx, app)
	if err != nil {
		return "", err
	}
	if temp {
		defer os.RemoveAll(filepath.Dir(path))
	}
	dest := `C:/Users/` + windowsUser + `/Downloads/` + filepath.Base(path)
	if _, err := run(ctx, "scp", append(w.sshArgs(v, "-P"), path, windowsUser+"@127.0.0.1:"+dest)...); err != nil {
		return "", err
	}
	switch strings.ToLower(filepath.Ext(path)) {
	case ".msi":
		return w.ssh(ctx, v, "Start-Process msiexec -Wait -ArgumentList '/i','"+dest+"','/qn'; 'Installed.'")
	case ".msix", ".msixbundle", ".appx":
		return w.ssh(ctx, v, "Add-AppxPackage -Path '"+dest+"'; 'Installed.'")
	}
	return w.ssh(ctx, v, "Start-Process -Wait '"+dest+"' -ArgumentList '/S'; 'Ran its installer with /S.'")
}

// upstreamDNS is the resolvers this machine asks, not a stub of its own on 127.0.0.53, which Windows can't reach.
func upstreamDNS() []string {
	var found []string
	for _, file := range []string{"/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"} {
		b, _ := os.ReadFile(file)
		for _, line := range strings.Split(string(b), "\n") {
			f := strings.Fields(line)
			if len(f) == 2 && f[0] == "nameserver" {
				if ip := net.ParseIP(f[1]); ip != nil && ip.To4() != nil && !ip.IsLoopback() && !slices.Contains(found, f[1]) {
					found = append(found, f[1])
				}
			}
		}
	}
	return found
}

// reach forwards the port inside Windows to this machine's own, over SSH from here, until Windows stops.
func (w windowsQEMU) reach(ctx context.Context, v *vm, port int) (string, error) {
	p := strconv.Itoa(port)
	reached := fmt.Sprintf("%s reaches this machine's port %d at 127.0.0.1:%d.", v.Name, port, port)
	// A port proxy reach once left there, to QEMU's 10.0.2.2, which is nothing now, holds the port: it goes.
	out, err := w.ssh(ctx, v, "netsh interface portproxy delete v4tov4 listenaddress=127.0.0.1 listenport="+p+" | Out-Null; "+
		"(Get-NetTCPConnection -LocalAddress 127.0.0.1 -LocalPort "+p+" -State Listen -ErrorAction SilentlyContinue | % { (Get-Process -Id $_.OwningProcess).Name }) -join ','")
	if err != nil {
		return "", err
	}
	switch strings.TrimSpace(out) {
	case "":
	case "sshd":
		return reached, nil
	default:
		return "", fmt.Errorf("%s has %s on its port %d already", v.Name, strings.TrimSpace(out), port)
	}
	name, args := outliving("ssh", append(w.sshArgs(v, "-p"), "-f", "-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=15",
		"-R", "127.0.0.1:"+p+":127.0.0.1:"+p, windowsUser+"@127.0.0.1")...)
	if _, err := run(ctx, name, args...); err != nil {
		return "", err
	}
	return reached, nil
}

func (w windowsQEMU) run(ctx context.Context, v *vm, command string) (string, error) {
	return w.ssh(ctx, v, command)
}

func (windowsQEMU) sshArgs(v *vm, portFlag string) []string {
	return []string{portFlag, strconv.Itoa(v.Port), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=" + winFile(v, "known_hosts"),
		"-o", "LogLevel=ERROR", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5"}
}

// ssh runs a PowerShell command in the device, as its account.
func (w windowsQEMU) ssh(ctx context.Context, v *vm, command string) (string, error) {
	return runIn(ctx, strings.NewReader(command), "ssh", append(w.sshArgs(v, "-p"), windowsUser+"@127.0.0.1", "powershell -NoProfile -NonInteractive -Command -")...)
}

func psQuote(s string) string { return strings.ReplaceAll(s, "'", "''") }

// monitor runs a command at a device's QEMU monitor and gives what it said.
func monitor(v *vm, command string) (string, error) {
	c, err := net.DialTimeout("unix", winFile(v, "monitor.sock"), 5*time.Second)
	if err != nil {
		return "", fmt.Errorf("%s's monitor: %w", v.Name, err)
	}
	defer c.Close()
	c.SetDeadline(time.Now().Add(2 * time.Minute))
	r := bufio.NewReader(c)
	prompt := func() (string, error) {
		var b strings.Builder
		for !strings.HasSuffix(b.String(), "(qemu) ") {
			ch, err := r.ReadByte()
			if err != nil {
				return b.String(), err
			}
			b.WriteByte(ch)
		}
		return b.String(), nil
	}
	if _, err := prompt(); err != nil {
		return "", err
	}
	if _, err := c.Write([]byte(command + "\n")); err != nil {
		return "", err
	}
	out, err := prompt()
	out = strings.TrimSuffix(out, "(qemu) ")
	// It echoes the command, with terminal escapes, on its first line.
	if _, rest, ok := strings.Cut(out, "\n"); ok {
		out = rest
	}
	out = strings.TrimSpace(strings.ReplaceAll(out, "\r", ""))
	if err == nil && (strings.HasPrefix(out, "Error") || strings.Contains(out, "unknown command")) {
		err = fmt.Errorf("%s: %s", strings.Fields(command)[0], out)
	}
	return out, err
}
