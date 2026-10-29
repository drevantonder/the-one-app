// Command one is TheOneApp: the node (one serve), the CLI for every plugin
// operation (one <plugin> <operation> ...), and the host for built-in plugins.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"mime"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/drevantonder/the-one-app/internal/bucket"
	"github.com/drevantonder/the-one-app/internal/core"
	"github.com/drevantonder/the-one-app/internal/mesh"
	"github.com/drevantonder/the-one-app/internal/store"
	"github.com/drevantonder/the-one-app/internal/surface"
	"github.com/drevantonder/the-one-app/plugins/agentsmd"
	"github.com/drevantonder/the-one-app/plugins/apple"
	"github.com/drevantonder/the-one-app/plugins/instructions"
	"github.com/drevantonder/the-one-app/plugins/memory"
	"github.com/drevantonder/the-one-app/plugins/messenger"
	"github.com/drevantonder/the-one-app/plugins/shell"
	"github.com/drevantonder/the-one-app/plugins/spaces"
	"github.com/drevantonder/the-one-app/plugins/whatsapp"
	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/sdk"
)

// version is set at build time; see `one update`.
var version = "dev"

const usage = `one: TheOneApp

  one serve [--addr ADDR] [--data DIR]   run this device's node
  one update                              update every device's node: from this checkout, or the latest release
  one release vX.Y.Z                      tag, publish to GitHub and update every device
  one mcp                                 MCP over stdio, relayed to this device's node
  one plugins [list]                      list plugins
  one plugins enable|disable|reload ID    manage a plugin
  one plugins link DIR                    load a plugin from a Go module, with hot reload
  one plugins add ID --mcp URL [--oauth google --scopes S]
                                          add a no-code plugin from an MCP server
  one plugins remove ID                   remove an added or published plugin
  one peers                               this user's other devices and their sync state
  one bucket setup-r2 NAME                back up to a Cloudflare R2 bucket (needs CLOUDFLARE_API_TOKEN)
  one bucket status|kit|import KIT        bucket state, recovery kit, restore from a kit
  one ops                                 list every operation
  one <plugin> <operation> [args...]      run any operation, e.g. one memory note "text"
  one call <plugin> <operation> [JSON]    run an operation with raw JSON input

Arguments fill an operation's fields in order; use --field value or field=value
for the rest. The node address comes from ONE_ADDR (default 127.0.0.1:4747).
`

func main() {
	if len(os.Args) < 2 {
		fmt.Print(usage)
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "serve":
		bin := self()
		if err = serve(os.Args[2:], bin); errors.Is(err, errRestart) {
			err = reexec(bin)
		}
	case "update":
		err = update(os.Args[2:])
	case "release":
		err = release(os.Args[2:])
	case "mcp":
		err = mcpStdio()
	case "plugin-host":
		err = pluginHost(os.Args[2:])
	case "version", "--version":
		fmt.Println(version)
	case "help", "-h", "--help":
		fmt.Print(usage)
	default:
		err = client(os.Args[1:])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func defaultData() string {
	if d := os.Getenv("ONE_HOME"); d != "" {
		return d
	}
	if d := os.Getenv("XDG_DATA_HOME"); d != "" {
		return filepath.Join(d, "one")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".local", "share", "one")
}

func defaultAddr() string {
	if a := os.Getenv("ONE_ADDR"); a != "" {
		return a
	}
	return "127.0.0.1:4747"
}

func serve(args []string, bin string) error {
	fs := flag.NewFlagSet("serve", flag.ExitOnError)
	addr := fs.String("addr", defaultAddr(), "listen address")
	data := fs.String("data", defaultData(), "data directory")
	local := fs.Bool("local", false, "stay off the tailnet: no mesh, loopback only")
	_ = fs.Parse(args)

	log := slog.New(slog.NewTextHandler(os.Stderr, nil))
	if err := os.MkdirAll(*data, 0o700); err != nil {
		return err
	}
	st, err := store.Open(filepath.Join(*data, "one.db"))
	if err != nil {
		return err
	}
	defer st.Close()

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	var restarting atomic.Bool
	restart := func() { restarting.Store(true); stop() }

	ln, err := net.Listen("tcp", *addr)
	if err != nil {
		return err
	}
	node, err := core.New(st, *data, builtinIDs, log)
	if err != nil {
		return err
	}
	b := bucket.New(node, st, log)
	if err := b.Start(ctx); err != nil {
		return err
	}
	m := mesh.New(node, st, portOf(*addr), log)
	m.OnPeer = b.AdoptFromPeer
	m.Version = version
	node.SetCoordinator(coordinator{b, m, st.Device()})
	node.SetDeclaredHost([]string{exe(), "plugin-host", "--declared"})
	os.Setenv("ONE_ADDR", *addr) // plugins build their hook URLs from it
	node.SetBlobs(b)
	node.Start(ctx)
	defer node.Stop()
	mux := http.NewServeMux()
	mux.Handle("/", surface.Handler(node, version))
	m.Routes(mux)
	b.Routes(mux)
	updateRoutes(mux, bin, restart)
	if !*local {
		if err := m.Start(ctx, mux); err != nil {
			return err
		}
	}
	srv := &http.Server{Handler: mux, ReadHeaderTimeout: 10 * time.Second}
	go func() {
		<-ctx.Done()
		sctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(sctx)
	}()
	log.Info("TheOneApp node running", "device", st.Device(), "api", "http://"+ln.Addr().String(), "mcp", "http://"+ln.Addr().String()+"/mcp", "data", *data)
	if err := srv.Serve(ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	if restarting.Load() {
		node.Stop()
		return errRestart
	}
	return nil
}

func portOf(addr string) string {
	if _, port, err := net.SplitHostPort(addr); err == nil {
		return port
	}
	return "4747"
}

// builtins are the plugins bundled in this binary, in the order their
// transforms run.
var builtins = []sdk.Plugin{instructions.Plugin, agentsmd.Plugin, memory.Plugin, shell.Plugin, apple.Reminders, apple.Calendar, apple.Contacts, apple.IMessage, spaces.Plugin, messenger.Plugin, whatsapp.Plugin}

var builtinIDs = func() []string {
	var ids []string
	for _, p := range builtins {
		ids = append(ids, p.ID)
	}
	return ids
}()

func exe() string {
	p, _ := os.Executable()
	return p
}

func pluginHost(args []string) error {
	if len(args) == 1 && args[0] == "--declared" {
		p, err := declaredPlugin()
		if err != nil {
			return err
		}
		sdk.Run(p)
		return nil
	}
	if len(args) != 1 {
		return errors.New("usage: one plugin-host <id>")
	}
	var p sdk.Plugin
	ok := false
	for _, b := range builtins {
		if b.ID == args[0] {
			p, ok = b, true
		}
	}
	if !ok {
		return fmt.Errorf("no built-in plugin %q", args[0])
	}
	sdk.Run(p)
	return nil
}

// ---- client ----------------------------------------------------------------

func client(args []string) error {
	ops, err := fetchOps()
	if err != nil {
		return err
	}
	switch args[0] {
	case "ops":
		for _, o := range ops {
			fmt.Printf("%-28s %s\n", cliName(o), o.Description)
		}
		return nil
	case "call":
		if len(args) < 3 {
			return errors.New("usage: one call <plugin> <operation> [JSON]")
		}
		input := "{}"
		if len(args) > 3 {
			input = args[3]
		}
		return run(args[1], args[2], json.RawMessage(input))
	case "bucket":
		if len(args) >= 3 && args[1] == "setup-r2" {
			return setupR2(args[2])
		}
	case "plugins":
		if len(args) == 1 {
			args = append(args, "list")
		}
		if args[1] == "add" {
			return addPlugin(args[2:])
		}
		if args[1] == "link" && len(args) > 2 {
			if abs, err := filepath.Abs(args[2]); err == nil {
				args[2] = abs
			}
		}
	}
	op, rest, ok := resolve(ops, args)
	if !ok {
		return fmt.Errorf("unknown command %q; run `one ops` to list operations", strings.Join(args, " "))
	}
	input, err := buildInput(op, rest)
	if err != nil {
		return err
	}
	return run(op.Plugin, op.Name, input)
}

// cliName is how an operation is typed: core ops by their own name with
// underscores as spaces (one plugins list), plugin ops as one <plugin> <op>.
func cliName(o core.Op) string {
	if o.Plugin == core.CoreID {
		return strings.ReplaceAll(o.Name, "_", " ")
	}
	return o.Plugin + " " + o.Name
}

func resolve(ops []core.Op, args []string) (core.Op, []string, bool) {
	best, n := core.Op{}, 0
	for _, o := range ops {
		words := strings.Fields(cliName(o))
		if len(words) <= len(args) && len(words) > n && strings.Join(args[:len(words)], " ") == strings.Join(words, " ") {
			best, n = o, len(words)
		}
	}
	return best, args[n:], n > 0
}

type prop struct {
	name string
	typ  string
}

// properties returns an input schema's properties in declaration order.
func properties(schema json.RawMessage) ([]prop, error) {
	var s struct {
		Properties json.RawMessage `json:"properties"`
	}
	if err := json.Unmarshal(schema, &s); err != nil || len(s.Properties) == 0 {
		return nil, err
	}
	dec := json.NewDecoder(bytes.NewReader(s.Properties))
	if _, err := dec.Token(); err != nil {
		return nil, err
	}
	var out []prop
	for dec.More() {
		t, err := dec.Token()
		if err != nil {
			return nil, err
		}
		var p struct {
			Type any `json:"type"`
		}
		if err := dec.Decode(&p); err != nil {
			return nil, err
		}
		typ, _ := p.Type.(string)
		if list, ok := p.Type.([]any); ok && len(list) > 0 {
			typ, _ = list[0].(string)
		}
		out = append(out, prop{name: t.(string), typ: typ})
	}
	return out, nil
}

func buildInput(op core.Op, args []string) (json.RawMessage, error) {
	props, err := properties(op.InputSchema)
	if err != nil {
		return nil, err
	}
	byName := map[string]prop{}
	for _, p := range props {
		byName[p.name] = p
	}
	in := map[string]any{}
	set := func(name, val string) error {
		p, ok := byName[name]
		if !ok {
			return fmt.Errorf("%s has no field %q", cliName(op), name)
		}
		switch p.typ {
		case "integer":
			v, err := strconv.ParseInt(val, 10, 64)
			if err != nil {
				return fmt.Errorf("%s must be a whole number", name)
			}
			in[name] = v
		case "number":
			v, err := strconv.ParseFloat(val, 64)
			if err != nil {
				return fmt.Errorf("%s must be a number", name)
			}
			in[name] = v
		case "boolean":
			v, err := strconv.ParseBool(val)
			if err != nil {
				return fmt.Errorf("%s must be true or false", name)
			}
			in[name] = v
		case "array": // a,b,c or, when items hold commas or aren't strings, JSON
			if strings.HasPrefix(val, "[") {
				var v []any
				if err := json.Unmarshal([]byte(val), &v); err != nil {
					return fmt.Errorf("%s must be a JSON array or comma separated", name)
				}
				in[name] = v
				break
			}
			in[name] = strings.Split(val, ",")
		case "object":
			var v map[string]any
			if err := json.Unmarshal([]byte(val), &v); err != nil {
				return fmt.Errorf("%s must be a JSON object", name)
			}
			in[name] = v
		default:
			in[name] = val
		}
		return nil
	}
	next := 0
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case strings.HasPrefix(a, "--") && strings.Contains(a, "="):
			k, v, _ := strings.Cut(a[2:], "=")
			if err := set(k, v); err != nil {
				return nil, err
			}
		case strings.HasPrefix(a, "--"):
			if i+1 >= len(args) {
				return nil, fmt.Errorf("%s needs a value", a)
			}
			if err := set(a[2:], args[i+1]); err != nil {
				return nil, err
			}
			i++
		case strings.Contains(a, "=") && byName[strings.SplitN(a, "=", 2)[0]].name != "":
			k, v, _ := strings.Cut(a, "=")
			if err := set(k, v); err != nil {
				return nil, err
			}
		default:
			for next < len(props) {
				if _, taken := in[props[next].name]; !taken {
					break
				}
				next++
			}
			if next >= len(props) {
				return nil, fmt.Errorf("too many arguments for %s", cliName(op))
			}
			if err := set(props[next].name, a); err != nil {
				return nil, err
			}
			next++
		}
	}
	return json.Marshal(in)
}

var httpClient = &http.Client{Timeout: 5 * time.Minute}

func fetchOps() ([]core.Op, error) {
	resp, err := httpClient.Get("http://" + defaultAddr() + "/v1/operations")
	if err != nil {
		return nil, fmt.Errorf("cannot reach the node at %s (is `one serve` running?): %w", defaultAddr(), err)
	}
	defer resp.Body.Close()
	var ops []core.Op
	return ops, json.NewDecoder(resp.Body).Decode(&ops)
}

func run(plugin, op string, input json.RawMessage) error {
	resp, err := httpClient.Post("http://"+defaultAddr()+"/v1/call/"+plugin+"/"+op, "application/json", bytes.NewReader(input))
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		var e struct{ Error string }
		if json.Unmarshal(body, &e) == nil && e.Error != "" {
			return errors.New(e.Error)
		}
		return fmt.Errorf("%s: %s", resp.Status, body)
	}
	var res protocol.Result
	if err := json.Unmarshal(body, &res); err != nil {
		return err
	}
	switch {
	case res.Text != "":
		fmt.Println(res.Text)
	case res.Data != nil || len(res.Images) == 0:
		out, _ := json.MarshalIndent(res.Data, "", "  ")
		fmt.Println(string(out))
	}
	for _, img := range res.Images { // a terminal can't show images, so save them
		ext := ".png"
		if exts, _ := mime.ExtensionsByType(img.MIMEType); len(exts) > 0 {
			ext = exts[len(exts)-1]
		}
		f, err := os.CreateTemp("", "one-*"+ext)
		if err != nil {
			return err
		}
		_, err = f.Write(img.Data)
		f.Close()
		if err != nil {
			return err
		}
		fmt.Println("image:", f.Name())
	}
	return nil
}
