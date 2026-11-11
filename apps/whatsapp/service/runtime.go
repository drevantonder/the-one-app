package main

import (
	"cmp"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

// toRuntime sends what a client asks the app runtime with the token in the app runtime's home
// (ONE_HOME, else ~/.local/share/<ONE_CLI>), which only this machine's user
// can read: the app runtime takes nothing else as one of this machine's own programs
// (src/api.rs). http.DefaultClient uses it, and so must every other client
// that asks the app runtime; http.DefaultTransport stays as it is, which libraries
// (whatsmeow) take to be an *http.Transport.
var toRuntime http.RoundTripper = http.DefaultTransport

func runtimeAddr() string {
	addr := cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	home, _ := os.UserHomeDir()
	data := cmp.Or(os.Getenv("XDG_DATA_HOME"), filepath.Join(home, ".local", "share"))
	dir := cmp.Or(os.Getenv("ONE_HOME"), filepath.Join(data, cmp.Or(os.Getenv("ONE_CLI"), "one")))
	toRuntime = asThisMachine{addr, filepath.Join(dir, "local-token"), http.DefaultTransport}
	http.DefaultClient.Transport = toRuntime
	return addr
}

// asThisMachine reads the token for each request, so the app runtime may start after
// the service.
type asThisMachine struct {
	addr, token string
	next        http.RoundTripper
}

func (a asThisMachine) RoundTrip(r *http.Request) (*http.Response, error) {
	if r.URL.Host == a.addr {
		if token, err := os.ReadFile(a.token); err == nil {
			r = r.Clone(r.Context())
			r.Header.Set("x-one-local-token", strings.TrimSpace(string(token)))
		}
	}
	return a.next.RoundTrip(r)
}
