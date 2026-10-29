package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/internal/core"
)

const repo = "drevantonder/the-one-app"

// appAsset is the signed Mac app, zipped; Macs running TheOneApp.app take it
// instead of a bare binary.
const appAsset = "TheOneApp-macos.zip"

// remote is a running node, found on the tailnet.
type remote struct {
	name  string
	base  string
	build nodeBuild
}

// assetFor is the release file a node runs.
func assetFor(b nodeBuild) string {
	if b.App {
		return appAsset
	}
	name := "one-" + b.OS + "-" + b.Arch
	if b.OS == "windows" {
		name += ".exe"
	}
	return name
}

func update(args []string) error {
	fromRelease := len(args) == 1 && args[0] == "--release"
	if len(args) > 0 && !fromRelease {
		return errors.New("usage: one update [--release]")
	}
	ctx := context.Background()
	nodes, err := findNodes(ctx)
	if err != nil {
		return err
	}
	if len(nodes) == 0 {
		return errors.New("no nodes found on this tailnet")
	}
	dir, err := os.MkdirTemp("", "one-update-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)

	var ver string
	var files map[string]string
	if fromRelease {
		if ver, err = latestRelease(ctx); err != nil {
			return err
		}
		fmt.Printf("updating to %s, the latest release\n", ver)
		files, err = download(ctx, dir, ver, stale(nodes, ver))
	} else {
		var src, commit string
		if src, commit, ver, err = checkoutMain(ctx); err != nil {
			return err
		}
		need := map[string]bool{}
		for _, n := range stale(nodes, ver) {
			need[assetFor(n.build)] = true
		}
		fmt.Printf("building %s, the tip of main\n", ver)
		files, err = buildAssets(ctx, src, commit, dir, ver, need, nodes, false)
	}
	if err != nil {
		return err
	}
	return pushAll(ctx, nodes, ver, files)
}

// release tests the tip of main, signs it off, builds it for every platform,
// updates every device and publishes it as a GitHub release.
func release(args []string) error {
	if len(args) != 1 || !regexp.MustCompile(`^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$`).MatchString(args[0]) {
		return errors.New("usage: one release vX.Y.Z")
	}
	tag := args[0]
	ctx := context.Background()
	if err := step(".", "gh", "auth", "status"); err != nil {
		return errors.New("log in to GitHub first: gh auth login")
	}
	src, commit, _, err := checkoutMain(ctx)
	if err != nil {
		return err
	}
	if _, err := git(src, "rev-parse", "-q", "--verify", "refs/tags/"+tag); err == nil {
		return fmt.Errorf("%s already exists", tag)
	}
	fmt.Printf("testing %s\n", commit[:7])
	if err := step(src, "go", "test", "./..."); err != nil {
		return err
	}
	if err := step(src, "gh", "signoff", "create", "--commit", commit); err != nil {
		return fmt.Errorf("signing off (install gh-signoff with gh extension install basecamp/gh-signoff): %w", err)
	}

	nodes, err := findNodes(ctx)
	if err != nil {
		return err
	}
	dir, err := os.MkdirTemp("", "one-release-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)
	need := map[string]bool{appAsset: true}
	for _, t := range core.Targets {
		goos, goarch, _ := strings.Cut(t, "/")
		need[assetFor(nodeBuild{OS: goos, Arch: goarch})] = true
	}
	fmt.Printf("building %s\n", tag)
	files, err := buildAssets(ctx, src, commit, dir, tag, need, nodes, true)
	if err != nil {
		return err
	}
	if err := pushAll(ctx, nodes, tag, files); err != nil {
		fmt.Fprintln(os.Stderr, "warning:", err)
	}
	sums, err := checksums(dir, files)
	if err != nil {
		return err
	}
	assets := []string{sums}
	for _, f := range files {
		assets = append(assets, f)
	}
	sort.Strings(assets)
	fmt.Println("publishing to GitHub")
	return step(src, "gh", append([]string{"release", "create", tag, "--repo", repo, "--target", commit, "--generate-notes"}, assets...)...)
}

// checkoutMain brings a clean clone, kept in the cache, to the tip of main
// on GitHub: what runs on every device is only ever what is on main.
func checkoutMain(ctx context.Context) (dir, commit, ver string, err error) {
	cache, err := os.UserCacheDir()
	if err != nil {
		return "", "", "", err
	}
	dir = filepath.Join(cache, "one", "src")
	if out, err := exec.CommandContext(ctx, "sh", "-c", checkoutScript(`"`+dir+`"`, "")).CombinedOutput(); err != nil {
		return "", "", "", fmt.Errorf("fetching main: %w\n%s", err, out)
	}
	if commit, err = git(dir, "rev-parse", "HEAD"); err != nil {
		return "", "", "", err
	}
	ver, err = git(dir, "describe", "--tags", "--always")
	return dir, commit, ver, err
}

// checkoutScript is shell that brings a clone at dir to commit, or to the
// tip of main when commit is empty, cloning it the first time.
func checkoutScript(dir, commit string) string {
	if commit == "" {
		commit = "origin/main"
	}
	return fmt.Sprintf(`set -e; d=%s; [ -d "$d/.git" ] || git clone -q https://github.com/%s.git "$d"; cd "$d"; git fetch -q --tags --force origin; git checkout -q --detach --force %s; git clean -qfdx`, dir, repo, commit)
}

func git(dir string, args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	out, err := cmd.Output()
	return strings.TrimSpace(string(out)), err
}

// step runs a command in dir, showing its output.
func step(dir, name string, args ...string) error {
	cmd := exec.Command(name, args...)
	cmd.Dir = dir
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%s %s: %w", name, strings.Join(args, " "), err)
	}
	return nil
}

// findNodes asks every online machine of this Tailscale user, and this one,
// what node build it runs.
func findNodes(ctx context.Context) ([]remote, error) {
	var st struct {
		Self tsMachine
		Peer map[string]tsMachine
	}
	out, err := exec.CommandContext(ctx, "tailscale", "status", "--json").Output()
	if err != nil {
		return nil, fmt.Errorf("tailscale status: %w", err)
	}
	if err := json.Unmarshal(out, &st); err != nil {
		return nil, err
	}
	port := portOf(defaultAddr())
	candidates := []remote{{name: st.Self.name(), base: "http://" + defaultAddr()}}
	for _, p := range st.Peer {
		if p.UserID == st.Self.UserID && p.Online && len(p.TailscaleIPs) > 0 {
			candidates = append(candidates, remote{name: p.name(), base: "http://" + net.JoinHostPort(p.TailscaleIPs[0], port)})
		}
	}
	var mu sync.Mutex
	var wg sync.WaitGroup
	var nodes []remote
	for _, c := range candidates {
		wg.Add(1)
		go func() {
			defer wg.Done()
			b, err := nodeInfo(ctx, c.base, 3*time.Second)
			var old errOldNode
			switch {
			case errors.As(err, &old):
				fmt.Fprintf(os.Stderr, "%s runs a node too old to update itself; update it by hand once\n", c.name)
			case err == nil:
				c.build = b
				mu.Lock()
				nodes = append(nodes, c)
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	sort.Slice(nodes, func(i, j int) bool { return nodes[i].name < nodes[j].name })
	return nodes, nil
}

type tsMachine struct {
	DNSName      string
	TailscaleIPs []string
	UserID       int64
	Online       bool
}

func (m tsMachine) name() string { return strings.SplitN(m.DNSName, ".", 2)[0] }

type errOldNode struct{}

func (errOldNode) Error() string { return "node has no update endpoint" }

func nodeInfo(ctx context.Context, base string, timeout time.Duration) (nodeBuild, error) {
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	var b nodeBuild
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, base+"/v1/update", nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return b, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK || !strings.HasPrefix(resp.Header.Get("Content-Type"), "application/json") {
		return b, errOldNode{}
	}
	return b, json.NewDecoder(resp.Body).Decode(&b)
}

func stale(nodes []remote, ver string) []remote {
	var out []remote
	for _, n := range nodes {
		if n.build.Version != ver {
			out = append(out, n)
		}
	}
	return out
}

// buildAssets builds the needed release files into dir: binaries here, and
// the Mac app on a Mac, since only a Mac can build and sign it.
func buildAssets(ctx context.Context, src, commit, dir, ver string, need map[string]bool, nodes []remote, notarize bool) (map[string]string, error) {
	files := map[string]string{}
	var mu sync.Mutex
	var wg sync.WaitGroup
	var errs []error
	for asset := range need {
		wg.Add(1)
		go func() {
			defer wg.Done()
			path := filepath.Join(dir, asset)
			var err error
			if asset == appAsset {
				err = buildApp(ctx, commit, path, ver, nodes, notarize)
			} else {
				err = buildBinary(ctx, src, path, ver, asset)
			}
			mu.Lock()
			defer mu.Unlock()
			if err != nil {
				errs = append(errs, fmt.Errorf("%s: %w", asset, err))
				return
			}
			files[asset] = path
		}()
	}
	wg.Wait()
	return files, errors.Join(errs...)
}

func buildBinary(ctx context.Context, src, out, ver, asset string) error {
	goos, goarch, _ := strings.Cut(strings.TrimSuffix(strings.TrimPrefix(asset, "one-"), ".exe"), "-")
	cmd := exec.CommandContext(ctx, "go", "build", "-trimpath", "-ldflags", "-s -w -X main.version="+ver, "-o", out, "./cmd/one")
	cmd.Dir = src
	cmd.Env = append(os.Environ(), "GOOS="+goos, "GOARCH="+goarch, "CGO_ENABLED=0")
	if goarch == "arm" {
		cmd.Env = append(cmd.Env, "GOARM=7")
	}
	if b, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("%s", bytes.TrimSpace(b))
	}
	return nil
}

// macBuilder is the Mac node that builds TheOneApp.app: the one named by
// ONE_MAC, else the first running the app, else the first Mac. Nil means this
// machine, a Mac with no node of its own.
func macBuilder(nodes []remote) (*remote, error) {
	name := os.Getenv("ONE_MAC")
	for _, pick := range []func(remote) bool{
		func(n remote) bool { return name != "" && n.name == name },
		func(n remote) bool { return name == "" && n.build.App },
		func(n remote) bool { return name == "" && n.build.OS == "darwin" },
	} {
		for _, n := range nodes {
			if pick(n) {
				return &n, nil
			}
		}
	}
	if name == "" && runtime.GOOS == "darwin" {
		return nil, nil
	}
	return nil, errors.New("building the Mac app needs a Mac that runs a node (ONE_MAC names one)")
}

// buildApp has a Mac build the commit's TheOneApp.app, zipped. A Mac's node
// builds it rather than SSH, as only a process in the logged-in session can
// sign with the keychain.
func buildApp(ctx context.Context, commit, out, ver string, nodes []remote, notarize bool) error {
	mac, err := macBuilder(nodes)
	if err != nil {
		return err
	}
	f, err := os.Create(out)
	if err != nil {
		return err
	}
	defer f.Close()
	if mac == nil {
		return buildAppHere(ctx, commit, ver, notarize, f)
	}
	body, _ := json.Marshal(appRequest{Commit: commit, Version: ver, Notarize: notarize})
	req, _ := http.NewRequestWithContext(ctx, http.MethodPost, mac.base+"/v1/update/app", bytes.NewReader(body))
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("on %s: %s", mac.name, bytes.TrimSpace(msg))
	}
	_, err = io.Copy(f, resp.Body)
	return err
}

type appRequest struct {
	Commit   string `json:"commit"`
	Version  string `json:"version"`
	Notarize bool   `json:"notarize"`
}

// buildAppHere builds the commit's app with macos/build.sh in a clone on
// this Mac, and writes it zipped to w.
func buildAppHere(ctx context.Context, commit, ver string, notarize bool, w io.Writer) error {
	mode := "zip"
	if notarize {
		mode = "notarize"
	}
	// A login shell, for the PATH to go and swiftc that apps don't get.
	script := checkoutScript(`"$HOME/.cache/one-build"`, commit) +
		fmt.Sprintf(`; VERSION=%s exec "${SHELL:-/bin/zsh}" -lc 'macos/build.sh %s >&2 && cat build/%s'`, ver, mode, appAsset)
	cmd := exec.CommandContext(ctx, "sh", "-c", script)
	var stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = w, &stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%w\n%s", err, lastLines(stderr.String(), 15))
	}
	return nil
}

func lastLines(s string, n int) string {
	lines := strings.Split(strings.TrimSpace(s), "\n")
	return strings.Join(lines[max(0, len(lines)-n):], "\n")
}

func latestRelease(ctx context.Context) (string, error) {
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, "https://api.github.com/repos/"+repo+"/releases/latest", nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	var r struct {
		TagName string `json:"tag_name"`
	}
	if resp.StatusCode != http.StatusOK || json.NewDecoder(resp.Body).Decode(&r) != nil || r.TagName == "" {
		return "", fmt.Errorf("no release of %s found (%s)", repo, resp.Status)
	}
	return r.TagName, nil
}

// download fetches the release files the nodes need, checked against the
// release's checksums.
func download(ctx context.Context, dir, tag string, nodes []remote) (map[string]string, error) {
	get := func(name string) ([]byte, error) {
		req, _ := http.NewRequestWithContext(ctx, http.MethodGet, "https://github.com/"+repo+"/releases/download/"+tag+"/"+name, nil)
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return nil, err
		}
		defer resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			return nil, fmt.Errorf("download %s: %s", name, resp.Status)
		}
		return io.ReadAll(resp.Body)
	}
	raw, err := get("checksums.txt")
	if err != nil {
		return nil, err
	}
	sums := map[string]string{}
	for line := range strings.Lines(string(raw)) {
		if sum, name, ok := strings.Cut(strings.TrimSpace(line), "  "); ok {
			sums[name] = sum
		}
	}
	files := map[string]string{}
	for _, n := range nodes {
		asset := assetFor(n.build)
		if files[asset] != "" {
			continue
		}
		data, err := get(asset)
		if err != nil {
			return nil, err
		}
		if sum := sha256.Sum256(data); hex.EncodeToString(sum[:]) != sums[asset] {
			return nil, fmt.Errorf("%s doesn't match the release's checksum", asset)
		}
		path := filepath.Join(dir, asset)
		if err := os.WriteFile(path, data, 0o644); err != nil {
			return nil, err
		}
		files[asset] = path
	}
	return files, nil
}

func checksums(dir string, files map[string]string) (string, error) {
	var names []string
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)
	var b strings.Builder
	for _, name := range names {
		data, err := os.ReadFile(files[name])
		if err != nil {
			return "", err
		}
		sum := sha256.Sum256(data)
		fmt.Fprintf(&b, "%s  %s\n", hex.EncodeToString(sum[:]), name)
	}
	path := filepath.Join(dir, "checksums.txt")
	return path, os.WriteFile(path, []byte(b.String()), 0o644)
}

// pushAll sends each stale node its file and waits for it to come back on
// the new build.
func pushAll(ctx context.Context, nodes []remote, ver string, files map[string]string) error {
	var wg sync.WaitGroup
	var mu sync.Mutex
	var failed []string
	out := bufio.NewWriter(os.Stdout)
	for _, n := range nodes {
		wg.Add(1)
		go func() {
			defer wg.Done()
			start := time.Now()
			err := push(ctx, n, ver, files[assetFor(n.build)])
			mu.Lock()
			defer mu.Unlock()
			switch {
			case err != nil:
				failed = append(failed, n.name)
				fmt.Fprintf(out, "  %-12s %s: %v\n", n.name, n.build.Version, err)
			case n.build.Version == ver:
				fmt.Fprintf(out, "  %-12s %s, up to date\n", n.name, ver)
			default:
				fmt.Fprintf(out, "  %-12s %s → %s (%s)\n", n.name, n.build.Version, ver, time.Since(start).Round(time.Second))
			}
			out.Flush()
		}()
	}
	wg.Wait()
	if len(failed) > 0 {
		return fmt.Errorf("not updated: %s", strings.Join(failed, ", "))
	}
	return nil
}

func push(ctx context.Context, n remote, ver, file string) error {
	if n.build.Version == ver {
		return nil
	}
	if file == "" {
		return fmt.Errorf("no %s was built", assetFor(n.build))
	}
	f, err := os.Open(file)
	if err != nil {
		return err
	}
	defer f.Close()
	req, _ := http.NewRequestWithContext(ctx, http.MethodPut, n.base+"/v1/update", f)
	req.Header.Set("X-One-Version", ver)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	body, _ := io.ReadAll(resp.Body)
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("%s", bytes.TrimSpace(body))
	}
	deadline := time.Now().Add(2 * time.Minute)
	for time.Now().Before(deadline) {
		time.Sleep(time.Second)
		if b, err := nodeInfo(ctx, n.base, 2*time.Second); err == nil && b.Version == ver {
			return nil
		}
	}
	return errors.New("installed, but the node hasn't come back")
}
