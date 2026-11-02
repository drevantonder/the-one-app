package main

import (
	"bufio"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"strings"
	"time"
)

// vncAct gives a device input through its VNC server, as a person watching it would: the pointer goes exactly
// where it's sent, in the screen's pixels, and keys go as X keysyms. The connection is shared, leaving anyone
// else watching connected. Its server is a socket in the device's folder, which only this machine's user can open: on
// a port, any program reaching this machine's 127.0.0.1, one in another device among them, could drive it.
func vncAct(socket string, a action) error {
	c, err := net.DialTimeout("unix", socket, 5*time.Second)
	if err != nil {
		return fmt.Errorf("its screen: %w", err)
	}
	defer c.Close()
	c.SetDeadline(time.Now().Add(time.Minute))
	r := bufio.NewReader(c)
	if err := vncHello(r, c); err != nil {
		return err
	}
	pointer := func(mask byte, x, y float64) {
		b := []byte{5, mask, 0, 0, 0, 0}
		binary.BigEndian.PutUint16(b[2:], uint16(max(x, 0)))
		binary.BigEndian.PutUint16(b[4:], uint16(max(y, 0)))
		c.Write(b)
	}
	key := func(sym uint32, down bool) {
		b := []byte{4, 0, 0, 0, 0, 0, 0, 0}
		if down {
			b[1] = 1
		}
		binary.BigEndian.PutUint32(b[4:], sym)
		c.Write(b)
		time.Sleep(15 * time.Millisecond)
	}
	click := func(button byte) {
		pointer(0, a.X, a.Y)
		time.Sleep(40 * time.Millisecond)
		pointer(button, a.X, a.Y)
		time.Sleep(60 * time.Millisecond)
		pointer(0, a.X, a.Y)
	}
	switch a.Do {
	case "click":
		click(1)
	case "right_click":
		click(4)
	case "double_click":
		click(1)
		click(1)
	case "scroll":
		wheel := byte(16) // down
		if a.ToY > a.Y {
			wheel = 8
		}
		for range max(1, int(max(a.Y-a.ToY, a.ToY-a.Y)/100)) {
			pointer(wheel, a.X, a.Y)
			pointer(0, a.X, a.Y)
		}
	case "swipe":
		pointer(1, a.X, a.Y)
		for i := 1; i <= 10; i++ {
			f := float64(i) / 10
			time.Sleep(20 * time.Millisecond)
			pointer(1, a.X+(a.ToX-a.X)*f, a.Y+(a.ToY-a.Y)*f)
		}
		pointer(0, a.ToX, a.ToY)
	case "type":
		for _, ch := range a.Text {
			key(keysym(ch), true)
			key(keysym(ch), false)
		}
	case "key":
		var syms []uint32
		for _, part := range strings.FieldsFunc(strings.ToLower(a.Key), func(r rune) bool { return r == '-' || r == '+' }) {
			sym, ok := namedKeys[part]
			if !ok && len([]rune(part)) == 1 {
				sym = keysym([]rune(part)[0])
				ok = true
			}
			if !ok {
				return fmt.Errorf("no key %q", part)
			}
			syms = append(syms, sym)
		}
		for _, s := range syms {
			key(s, true)
		}
		for i := len(syms) - 1; i >= 0; i-- {
			key(syms[i], false)
		}
	default:
		return fmt.Errorf("no action %q", a.Do)
	}
	time.Sleep(50 * time.Millisecond) // for the server to read it before the connection closes
	return nil
}

// vncHello opens an RFB session with no password, shared.
func vncHello(r *bufio.Reader, w io.Writer) error {
	version := make([]byte, 12)
	if _, err := io.ReadFull(r, version); err != nil {
		return err
	}
	w.Write([]byte("RFB 003.008\n"))
	n, err := r.ReadByte()
	if err != nil || n == 0 {
		return errors.New("its screen refused the connection")
	}
	types := make([]byte, n)
	io.ReadFull(r, types)
	if !strings.Contains(string(types), "\x01") {
		return errors.New("its screen wants a password")
	}
	w.Write([]byte{1})
	result := make([]byte, 4)
	if _, err := io.ReadFull(r, result); err != nil || binary.BigEndian.Uint32(result) != 0 {
		return errors.New("its screen refused the connection")
	}
	w.Write([]byte{1}) // shared
	init := make([]byte, 24)
	if _, err := io.ReadFull(r, init); err != nil {
		return err
	}
	name := make([]byte, binary.BigEndian.Uint32(init[20:]))
	_, err = io.ReadFull(r, name)
	return err
}

var namedKeys = map[string]uint32{
	"enter": 0xff0d, "return": 0xff0d, "backspace": 0xff08, "tab": 0xff09, "escape": 0xff1b, "esc": 0xff1b, "delete": 0xffff, "del": 0xffff,
	"home": 0xff50, "left": 0xff51, "up": 0xff52, "right": 0xff53, "down": 0xff54, "page_up": 0xff55, "pgup": 0xff55, "page_down": 0xff56,
	"pgdn": 0xff56, "end": 0xff57, "insert": 0xff63, "space": 0x20, "shift": 0xffe1, "ctrl": 0xffe3, "control": 0xffe3, "alt": 0xffe9,
	"win": 0xffeb, "windows": 0xffeb, "super": 0xffeb, "meta": 0xffeb, "cmd": 0xffeb,
	"f1": 0xffbe, "f2": 0xffbf, "f3": 0xffc0, "f4": 0xffc1, "f5": 0xffc2, "f6": 0xffc3, "f7": 0xffc4, "f8": 0xffc5, "f9": 0xffc6, "f10": 0xffc7, "f11": 0xffc8, "f12": 0xffc9,
}

// keysym is the key that types ch. QEMU presses or lets go of shift for it as the character needs.
func keysym(ch rune) uint32 {
	switch {
	case ch == '\n':
		return 0xff0d
	case ch == '\t':
		return 0xff09
	case ch < 0x100:
		return uint32(ch)
	}
	return 0x01000000 + uint32(ch)
}
