package client

import (
	"runtime"
	"sync"
)

// NewDeviceSecret is a new device's key, 32 random bytes: the app keeps it (in the OS's secret store), and hands it to
// NewThisDevice each start.
func NewDeviceSecret() []byte {
	var st callStatus
	rb := get().newDeviceSecret(&st)
	must(&st)
	r := reader{b: take(rb)}
	return r.bytes()
}

// DeviceJoin is what a sign-in for the device whose key is `secret` names it by, before it's made (the
// authorization's `join`), so the person's devices can let it in by its sign-in.
func DeviceJoin(secret []byte) (string, error) {
	var st callStatus
	rb := get().deviceJoin(data(secret), &st)
	if err := check(&st); err != nil {
		return "", err
	}
	return string(take(rb)), nil
}

// ClientScreen is one of the app's own screens (its pages, Services, a Vault request, Settings') as its page, from
// those built into the library. `params` is JSON.
func ClientScreen(name, params string) string {
	var st callStatus
	rb := get().clientScreen(str(name), str(params), &st)
	must(&st)
	return string(take(rb))
}

// Note is a note pushed to this device, opened.
type Note struct {
	ID, Title, Body, App, Screen, Params string
	Badge                                *int64
	Action, By                           *string
}

// OpenNote opens a note pushed to the device whose key is `secret` (`n` in the push, base64url).
func OpenNote(secret []byte, pushed string) (*Note, error) {
	var st callStatus
	rb := get().openNote(data(secret), str(pushed), &st)
	if err := check(&st); err != nil {
		return nil, err
	}
	r := reader{b: take(rb)}
	n := &Note{ID: r.str(), Title: r.str(), Body: r.str(), App: r.str(), Screen: r.str(), Params: r.str()}
	if r.i8() == 1 {
		b := r.i64()
		n.Badge = &b
	}
	if r.i8() == 1 {
		a := r.str()
		n.Action = &a
	}
	if r.i8() == 1 {
		b := r.str()
		n.By = &b
	}
	return n, nil
}

// handle is a uniffi object: each method consumes a handle, so each call is given a clone; the object's own goes when
// it's closed, or collected.
type handle struct {
	mu     sync.Mutex
	h      uint64
	cloneF func(uint64, *callStatus) uint64
	freeF  func(uint64, *callStatus)
}

func newHandle(h uint64, clone func(uint64, *callStatus) uint64, free func(uint64, *callStatus)) *handle {
	o := &handle{h: h, cloneF: clone, freeF: free}
	runtime.AddCleanup(o, func(h uint64) {
		var st callStatus
		free(h, &st)
	}, h)
	return o
}

func (o *handle) clone() uint64 {
	var st callStatus
	h := o.cloneF(o.h, &st)
	must(&st)
	return h
}

// ThisDevice is this app as a device of its person.
type ThisDevice struct{ o *handle }

// NewThisDevice is the device whose key is `secret`, of the install at `address`, keeping what it learns in `dir`.
func NewThisDevice(secret []byte, address, dir string) (*ThisDevice, error) {
	l := get()
	var st callStatus
	h := l.deviceNew(data(secret), str(address), str(dir), &st)
	if err := check(&st); err != nil {
		return nil, err
	}
	return &ThisDevice{newHandle(h, l.deviceClone, l.deviceFree)}, nil
}

// Key is its key, hex.
func (d *ThisDevice) Key() string {
	var st callStatus
	rb := get().deviceKey(d.o.clone(), &st)
	must(&st)
	return string(take(rb))
}

// Code is the code it shows to be approved: "ABCD EFGH".
func (d *ThisDevice) Code() string {
	var st callStatus
	rb := get().deviceCode(d.o.clone(), &st)
	must(&st)
	return string(take(rb))
}

// On says which device it's on (hex): an id of the device's own, hashed, that outlives signing out.
func (d *ThisDevice) On(device string) {
	var st callStatus
	get().deviceOn(d.o.clone(), str(device), &st)
	must(&st)
}

// Ask asks to be let in, as the person signed in with `token`, calling it `name`; someone new brings their invite's
// secret.
func (d *ThisDevice) Ask(token, name string, invite *string) error {
	inv := serialized(func(w *writer) {
		if invite == nil {
			w.i8(0)
			return
		}
		w.i8(1)
		w.str(*invite)
	})
	var st callStatus
	get().deviceAsk(d.o.clone(), str(token), str(name), inv, &st)
	return check(&st)
}

// Wait says whether it's been let in yet, asking again if its request waited too long.
func (d *ThisDevice) Wait(token string) (bool, error) {
	var st callStatus
	v := get().deviceWait(d.o.clone(), str(token), &st)
	return v != 0, check(&st)
}

// Removed says whether the install took this device out.
func (d *ThisDevice) Removed() bool {
	var st callStatus
	v := get().deviceRemoved(d.o.clone(), &st)
	must(&st)
	return v != 0
}

// Connect reaches its nodes, if it's been let in: false if it hasn't.
func (d *ThisDevice) Connect() (bool, error) {
	var st callStatus
	v := get().deviceConnect(d.o.clone(), &st)
	return v != 0, check(&st)
}

// Header is one of a request's or response's headers.
type Header struct{ Name, Value string }

// Fetch is a request to its install's nodes' local API (`/v1/…`), as its person, with the keys it needs.
func (d *ThisDevice) Fetch(method, path string, headers []Header, body []byte) (*Response, error) {
	l := get()
	hs := serialized(func(w *writer) {
		w.i32(int32(len(headers)))
		for _, h := range headers {
			w.str(h.Name)
			w.str(h.Value)
		}
	})
	var st callStatus
	h := l.deviceFetch(d.o.clone(), str(method), str(path), hs, data(body), &st)
	if err := check(&st); err != nil {
		return nil, err
	}
	return &Response{newHandle(h, l.responseClone, l.responseFree)}, nil
}

// RegisterPush registers where a push service reaches this device: `via` apns, fcm, wns or webpush.
func (d *ThisDevice) RegisterPush(via, address, env, topic string) error {
	var st callStatus
	get().deviceRegisterPush(d.o.clone(), str(via), str(address), str(env), str(topic), &st)
	return check(&st)
}

// Developer says whether this device is in developer mode.
func (d *ThisDevice) Developer() bool {
	var st callStatus
	v := get().deviceDeveloper(d.o.clone(), &st)
	must(&st)
	return v != 0
}

// SetDeveloper turns developer mode on or off on this device.
func (d *ThisDevice) SetDeveloper(on bool) {
	var b int8
	if on {
		b = 1
	}
	var st callStatus
	get().deviceSetDeveloper(d.o.clone(), b, &st)
	must(&st)
}

// Traces is its latest requests' traces, as JSON: none out of developer mode.
func (d *ThisDevice) Traces() *string {
	var st callStatus
	rb := get().deviceTraces(d.o.clone(), &st)
	must(&st)
	r := reader{b: take(rb)}
	if r.i8() == 0 {
		return nil
	}
	s := r.str()
	return &s
}

// KeptAnswer is a call's last answer (its JSON) and when it was answered (ms since 1970).
type KeptAnswer struct {
	Answer []byte
	At     int64
}

// Kept is what an app's call answered last for `input` (JSON), if its operation only reads.
func (d *ThisDevice) Kept(app, op string, input []byte) *KeptAnswer {
	var st callStatus
	rb := get().deviceKept(d.o.clone(), str(app), str(op), data(input), &st)
	must(&st)
	r := reader{b: take(rb)}
	if r.i8() == 0 {
		return nil
	}
	return &KeptAnswer{Answer: r.bytes(), At: r.i64()}
}

// Reconnect starts again with a fresh connection.
func (d *ThisDevice) Reconnect() { d.void(get().deviceReconnect) }

// SignOut takes itself out of its person's devices, then closes its connections and forgets it was in.
func (d *ThisDevice) SignOut() { d.void(get().deviceSignOut) }

// Disconnect closes its connections, until it's asked again, still in.
func (d *ThisDevice) Disconnect() { d.void(get().deviceDisconnect) }

func (d *ThisDevice) void(f func(uint64, *callStatus)) {
	var st callStatus
	f(d.o.clone(), &st)
	must(&st)
}

// Response is a node's answer, its body read as it comes.
type Response struct{ o *handle }

// Status is its HTTP status.
func (r *Response) Status() int {
	var st callStatus
	v := get().responseStatus(r.o.clone(), &st)
	must(&st)
	return int(v)
}

// Headers are its headers, names lowercase.
func (r *Response) Headers() []Header {
	var st callStatus
	rb := get().responseHeaders(r.o.clone(), &st)
	must(&st)
	rd := reader{b: take(rb)}
	n := int(rd.i32())
	out := make([]Header, n)
	for i := range out {
		out[i] = Header{Name: rd.str(), Value: rd.str()}
	}
	return out
}

// Chunk is the body's next part as it arrives, or nil at its end.
func (r *Response) Chunk() ([]byte, error) {
	var st callStatus
	rb := get().responseChunk(r.o.clone(), &st)
	if err := check(&st); err != nil {
		return nil, err
	}
	rd := reader{b: take(rb)}
	if rd.i8() == 0 {
		return nil, nil
	}
	b := rd.bytes()
	if b == nil {
		b = []byte{}
	}
	return b, nil
}

// Bytes is the rest of the body, whole.
func (r *Response) Bytes() ([]byte, error) {
	var st callStatus
	rb := get().responseBytes(r.o.clone(), &st)
	if err := check(&st); err != nil {
		return nil, err
	}
	rd := reader{b: take(rb)}
	return rd.bytes(), nil
}

// Cancel stops reading the body: the node stops sending it.
func (r *Response) Cancel() {
	var st callStatus
	get().responseCancel(r.o.clone(), &st)
	must(&st)
}
