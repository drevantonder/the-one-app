// Package client is the Rust client (client/ffi) from Go: this device's key, being let in, and every request to the
// install's nodes over iroh. Its Go API (ThisDevice, Response, the free functions) doesn't say how the library is
// reached: `lib` is filled by a loader, purego's (loader_purego.go, macOS, Linux and Windows, built with CGO_ENABLED=0)
// or cgo's for an iPhone, which links it statically (loader_cgo.go, `-tags cgoffi`). Each call waits for its answer,
// so call from a goroutine of its own, never a UI thread.
//
// The C ABI is uniffi's (0.31): objects are u64 handles that a method consumes (so each call clones first), strings go
// as RustBuffers of their bytes, and everything else as RustBuffers in uniffi's big-endian serialization.
package client

import (
	"encoding/binary"
	"errors"
	"fmt"
	"sync"
	"unsafe"
)

// rustBuffer is uniffi's RustBuffer, 24 bytes, passed and returned by value.
type rustBuffer struct {
	capacity uint64
	len      uint64
	data     *byte // Rust's memory, never Go's
}

// callStatus is uniffi's RustCallStatus: 0 ok, 1 an error the function declares (in errorBuf), 2 a panic.
type callStatus struct {
	code     int8
	errorBuf rustBuffer
}

// lib is every function of client/ffi this package calls, as Go functions of their C signatures. A loader fills it.
type lib struct {
	alloc func(size uint64, st *callStatus) rustBuffer
	free  func(b rustBuffer, st *callStatus)

	newDeviceSecret func(st *callStatus) rustBuffer
	deviceJoin      func(secret rustBuffer, st *callStatus) rustBuffer
	clientScreen    func(name, params rustBuffer, st *callStatus) rustBuffer
	openNote        func(secret, pushed rustBuffer, st *callStatus) rustBuffer

	deviceNew          func(secret, address, dir rustBuffer, st *callStatus) uint64
	deviceClone        func(h uint64, st *callStatus) uint64
	deviceFree         func(h uint64, st *callStatus)
	deviceKey          func(h uint64, st *callStatus) rustBuffer
	deviceCode         func(h uint64, st *callStatus) rustBuffer
	deviceOn           func(h uint64, device rustBuffer, st *callStatus)
	deviceAsk          func(h uint64, token, name, invite rustBuffer, st *callStatus)
	deviceWait         func(h uint64, token rustBuffer, st *callStatus) int8
	deviceRemoved      func(h uint64, st *callStatus) int8
	deviceConnect      func(h uint64, st *callStatus) int8
	deviceFetch        func(h uint64, method, path, headers, body rustBuffer, st *callStatus) uint64
	deviceRegisterPush func(h uint64, via, address, env, topic rustBuffer, st *callStatus)
	deviceDeveloper    func(h uint64, st *callStatus) int8
	deviceSetDeveloper func(h uint64, on int8, st *callStatus)
	deviceTraces       func(h uint64, st *callStatus) rustBuffer
	deviceKept         func(h uint64, app, op, input rustBuffer, st *callStatus) rustBuffer
	deviceReconnect    func(h uint64, st *callStatus)
	deviceSignOut      func(h uint64, st *callStatus)
	deviceDisconnect   func(h uint64, st *callStatus)

	responseClone   func(h uint64, st *callStatus) uint64
	responseFree    func(h uint64, st *callStatus)
	responseStatus  func(h uint64, st *callStatus) uint16
	responseHeaders func(h uint64, st *callStatus) rustBuffer
	responseChunk   func(h uint64, st *callStatus) rustBuffer
	responseBytes   func(h uint64, st *callStatus) rustBuffer
	responseCancel  func(h uint64, st *callStatus)
}

// symbols names each of lib's functions by its C symbol, for loaders that look them up.
func (l *lib) symbols() map[string]any {
	const p = "uniffi_one_client_ffi_fn_"
	return map[string]any{
		"ffi_one_client_ffi_rustbuffer_alloc": &l.alloc,
		"ffi_one_client_ffi_rustbuffer_free":  &l.free,

		p + "func_new_device_secret": &l.newDeviceSecret,
		p + "func_device_join":       &l.deviceJoin,
		p + "func_client_screen":     &l.clientScreen,
		p + "func_open_note":         &l.openNote,

		p + "constructor_thisdevice_new":      &l.deviceNew,
		p + "clone_thisdevice":                &l.deviceClone,
		p + "free_thisdevice":                 &l.deviceFree,
		p + "method_thisdevice_key":           &l.deviceKey,
		p + "method_thisdevice_code":          &l.deviceCode,
		p + "method_thisdevice_on":            &l.deviceOn,
		p + "method_thisdevice_ask":           &l.deviceAsk,
		p + "method_thisdevice_wait":          &l.deviceWait,
		p + "method_thisdevice_removed":       &l.deviceRemoved,
		p + "method_thisdevice_connect":       &l.deviceConnect,
		p + "method_thisdevice_fetch":         &l.deviceFetch,
		p + "method_thisdevice_register_push": &l.deviceRegisterPush,
		p + "method_thisdevice_developer":     &l.deviceDeveloper,
		p + "method_thisdevice_set_developer": &l.deviceSetDeveloper,
		p + "method_thisdevice_traces":        &l.deviceTraces,
		p + "method_thisdevice_kept":          &l.deviceKept,
		p + "method_thisdevice_reconnect":     &l.deviceReconnect,
		p + "method_thisdevice_sign_out":      &l.deviceSignOut,
		p + "method_thisdevice_disconnect":    &l.deviceDisconnect,

		p + "clone_response":          &l.responseClone,
		p + "free_response":           &l.responseFree,
		p + "method_response_status":  &l.responseStatus,
		p + "method_response_headers": &l.responseHeaders,
		p + "method_response_chunk":   &l.responseChunk,
		p + "method_response_bytes":   &l.responseBytes,
		p + "method_response_cancel":  &l.responseCancel,
	}
}

var (
	the     *lib
	loading sync.Once
	loadErr error
)

// Load reaches the library at `path` (a loader's own default if empty: beside the program, then target/debug). Every
// other function loads it on first use.
func Load(path string) error {
	loading.Do(func() {
		l := &lib{}
		loadErr = load(l, path)
		if loadErr == nil {
			the = l
		}
	})
	return loadErr
}

func get() *lib {
	if err := Load(""); err != nil {
		panic(fmt.Sprintf("the client library: %v", err))
	}
	return the
}

// Error is client/ffi's OneError: Failed (its reason in a person's words, and whether the request may have reached a
// node) or NotLetIn.
type Error struct {
	Reason   string
	Sent     bool
	NotLetIn bool
}

func (e *Error) Error() string {
	if e.NotLetIn {
		return "This device hasn't been let in yet."
	}
	return e.Reason
}

// ErrNotLetIn: the install doesn't list this device, yet or any more.
var ErrNotLetIn = &Error{NotLetIn: true}

func (e *Error) Is(target error) bool {
	t, ok := target.(*Error)
	return ok && t.NotLetIn && e.NotLetIn
}

// check turns a call's status into an error, freeing its buffer.
func check(st *callStatus) error {
	switch st.code {
	case 0:
		return nil
	case 1:
		r := reader{b: take(st.errorBuf)}
		switch r.i32() {
		case 1:
			reason := r.str()
			return &Error{Reason: reason, Sent: r.i8() != 0}
		case 2:
			return ErrNotLetIn
		}
		return errors.New("the client failed in a way it doesn't say")
	default:
		msg := string(take(st.errorBuf))
		if msg == "" {
			msg = "the client stopped unexpectedly"
		}
		return errors.New(msg)
	}
}

// must is check for calls that declare no error: a panic in them is a bug.
func must(st *callStatus) {
	if err := check(st); err != nil {
		panic(err)
	}
}

// buf copies bytes into a RustBuffer that Rust allocated, which the call it's given to frees.
func buf(b []byte) rustBuffer {
	var st callStatus
	rb := get().alloc(uint64(len(b)), &st)
	must(&st)
	if len(b) > 0 {
		copy(unsafe.Slice(rb.data, len(b)), b)
	}
	rb.len = uint64(len(b))
	return rb
}

// take copies a RustBuffer Rust returned, and frees it.
func take(rb rustBuffer) []byte {
	var out []byte
	if rb.len > 0 && rb.data != nil {
		out = append([]byte(nil), unsafe.Slice(rb.data, rb.len)...)
	}
	if rb.data != nil {
		var st callStatus
		get().free(rb, &st)
	}
	return out
}

// writer is uniffi's serialization: big-endian, i32 lengths.
type writer struct{ b []byte }

func (w *writer) i8(v int8)   { w.b = append(w.b, byte(v)) }
func (w *writer) i32(v int32) { w.b = binary.BigEndian.AppendUint32(w.b, uint32(v)) }
func (w *writer) bytes(v []byte) {
	w.i32(int32(len(v)))
	w.b = append(w.b, v...)
}
func (w *writer) str(v string) { w.bytes([]byte(v)) }

type reader struct {
	b   []byte
	off int
}

func (r *reader) next(n int) []byte {
	if r.off+n > len(r.b) {
		panic("the client's answer was shorter than it says")
	}
	v := r.b[r.off : r.off+n]
	r.off += n
	return v
}
func (r *reader) i8() int8   { return int8(r.next(1)[0]) }
func (r *reader) i32() int32 { return int32(binary.BigEndian.Uint32(r.next(4))) }
func (r *reader) i64() int64 { return int64(binary.BigEndian.Uint64(r.next(8))) }
func (r *reader) bytes() []byte {
	return append([]byte(nil), r.next(int(r.i32()))...)
}
func (r *reader) str() string { return string(r.bytes()) }

// serialized wraps a serialized value as the RustBuffer an argument of a non-string type is.
func serialized(f func(w *writer)) rustBuffer {
	var w writer
	f(&w)
	return buf(w.b)
}

func str(s string) rustBuffer { return buf([]byte(s)) }

func data(b []byte) rustBuffer { return serialized(func(w *writer) { w.bytes(b) }) }
