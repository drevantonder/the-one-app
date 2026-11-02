package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"image"
	_ "image/png"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// windowsQEMU runs Windows 11 in QEMU on a Linux x86 machine with KVM: UEFI Secure Boot (OVMF) and a TPM (swtpm), as
// Windows 11 needs. A device's folder has its disk, its UEFI variables and its TPM; the machine reaches it through
// QEMU's monitor (pictures, mouse and keys), and through SSH forwarded to a port here (commands), as the Windows
// account one that trusts this machine's key. A copy is a disk on top of a lasting device's base: a copy of its
// disk taken when it's copied, kept for the next copy until the lasting device next stops.
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
	for _, c := range []string{"qemu-system-x86_64", "qemu-img", "swtpm"} {
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
	v.CPUs, v.GB = 4, 4
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
	} else if _, err := run(ctx, "cp", "--reflink=auto", winFile(from, "disk.qcow2"), tmp); err != nil {
		return "", err
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

func (w windowsQEMU) start(ctx context.Context, v *vm) error {
	os.MkdirAll(winFile(v, "tpm"), 0o700)
	if _, err := runOutliving(ctx, "swtpm", "socket", "--tpm2", "--tpmstate", "dir="+winFile(v, "tpm"), "--ctrl", "type=unixio,path="+winFile(v, "tpm/sock"),
		"--terminate", "--daemon", "--log", "file="+winFile(v, "tpm/log")); err != nil {
		return err
	}
	cpus, gb := max(v.CPUs, 2), max(v.GB, 4)
	_, err := runOutliving(ctx, "qemu-system-x86_64", "-name", "agent-devices-"+v.Name, "-daemonize", "-pidfile", winFile(v, "qemu.pid"),
		"-machine", "q35,smm=on,accel=kvm", "-global", "driver=cfi.pflash01,property=secure,value=on",
		"-cpu", "host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time", "-smp", fmt.Sprintf("%d,sockets=1,cores=%d,threads=1", cpus, cpus), "-m", fmt.Sprintf("%dG", gb),
		"-drive", "if=pflash,format=raw,unit=0,file="+ovmfCode+",readonly=on",
		"-drive", "if=pflash,format=raw,unit=1,file="+winFile(v, "OVMF_VARS.fd"),
		"-chardev", "socket,id=chrtpm,path="+winFile(v, "tpm/sock"), "-tpmdev", "emulator,id=tpm0,chardev=chrtpm", "-device", "tpm-crb,tpmdev=tpm0",
		"-drive", "file="+winFile(v, "disk.qcow2")+",if=none,id=d0,discard=unmap", "-device", "nvme,drive=d0,serial=one,bootindex=0",
		"-netdev", fmt.Sprintf("user,id=n0,hostfwd=tcp:127.0.0.1:%d-:22", v.Port), "-device", "e1000e,netdev=n0",
		"-device", "qemu-xhci", "-device", "usb-tablet", "-vga", "std", "-rtc", "base=utc,clock=host",
		"-audio", "none", "-display", "none", "-vnc", fmt.Sprintf("127.0.0.1:%d", v.Display), "-monitor", "unix:"+winFile(v, "monitor.sock")+",server,nowait")
	if err != nil {
		return err
	}
	// Ready once it answers over SSH: it signs in by itself.
	ctx, cancel := context.WithTimeout(ctx, 4*time.Minute)
	defer cancel()
	for {
		if _, err := w.ssh(ctx, v, "echo ready"); err == nil {
			return nil
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("%s started but didn't answer in 4 minutes: look at it with screen", v.Name)
		case <-time.After(5 * time.Second):
		}
	}
}

// stop shuts Windows down, as its power button would, and pulls the plug if it hasn't in 3 minutes. Its base is
// taken again at the next copy.
func (w windowsQEMU) stop(ctx context.Context, v *vm) error {
	if !w.running(ctx, v) {
		return nil
	}
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
	if v.Of == "" {
		os.Remove(winFile(v, "base.qcow2"))
	}
	return nil
}

func (windowsQEMU) pause(_ context.Context, v *vm) error {
	_, err := monitor(v, "stop")
	return err
}

func (windowsQEMU) resume(_ context.Context, v *vm) error {
	_, err := monitor(v, "cont")
	return err
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
	return vncAct(5900+v.Display, a)
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
