package main

import (
	"context"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

// TestSelfUpdate pushes a new build to a running node, which installs it
// over its own binary and restarts as it, in the same process.
func TestSelfUpdate(t *testing.T) {
	if testing.Short() {
		t.Skip("builds and runs the binary")
	}
	dir := t.TempDir()
	build := func(ver, out string) {
		if b, err := exec.Command("go", "build", "-ldflags", "-X main.version="+ver, "-o", out, ".").CombinedOutput(); err != nil {
			t.Fatalf("build: %v\n%s", err, b)
		}
	}
	bin, next := filepath.Join(dir, "one"), filepath.Join(dir, "next")
	build("v1", bin)
	build("v2", next)

	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close()
	serve := exec.Command(bin, "serve", "--local", "--addr", addr, "--data", filepath.Join(dir, "data"))
	serve.Stderr = os.Stderr
	serve.Dir = dir
	serve.Env = nodeEnv(t, filepath.Join(dir, "home"))
	if err := serve.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = serve.Process.Signal(os.Interrupt); _ = serve.Wait() })
	waitFor(t, "http://"+addr+"/healthz")

	ctx := context.Background()
	n := remote{name: "test", base: "http://" + addr}
	if n.build, err = nodeInfo(ctx, n.base, 5*time.Second); err != nil || n.build.Version != "v1" {
		t.Fatalf("node reports %+v, %v; want v1", n.build, err)
	}

	bad := filepath.Join(dir, "bad")
	if err := os.WriteFile(bad, []byte("#!/bin/sh\necho v3\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := push(ctx, n, "v2", bad); err == nil || !strings.Contains(err.Error(), "doesn't run here as v2") {
		t.Fatalf("pushing a build that isn't v2: %v", err)
	}

	if err := push(ctx, n, "v2", next); err != nil {
		t.Fatal(err)
	}
	if out, _ := exec.Command(bin, "version").Output(); strings.TrimSpace(string(out)) != "v2" {
		t.Fatalf("binary on disk is %q, want v2", out)
	}
	if resp, err := http.Get("http://" + addr + "/healthz"); err != nil {
		t.Fatal(err)
	} else {
		resp.Body.Close()
	}
	if err := serve.Process.Signal(syscall.Signal(0)); err != nil {
		t.Fatal("the node exited instead of restarting in place")
	}
}

// TestSettle clears the staging dirs installs cut short leave behind, and
// puts a build that runs from one back where it belongs.
func TestSettle(t *testing.T) {
	dir := t.TempDir()
	write := func(path, data string) {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(data), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	bin := filepath.Join(dir, "one")
	write(bin, "old")
	write(filepath.Join(dir, ".one-update-1", "upload"), "cut short")
	stray := filepath.Join(dir, ".one-update-2", "one")
	write(stray, "new")

	exe, err := settle(stray)
	if err != nil || exe != bin {
		t.Fatalf("settle(stray) = %q, %v; want %q", exe, err, bin)
	}
	if b, _ := os.ReadFile(bin); string(b) != "new" {
		t.Fatalf("home holds %q, want the stray build", b)
	}
	if _, err := os.Stat(filepath.Join(dir, ".one-update-1")); !os.IsNotExist(err) {
		t.Fatal("a stale staging dir survived")
	}
	if _, err := os.Stat(stray); err != nil {
		t.Fatal("settle removed the build it runs from")
	}

	if exe, err := settle(bin); err != nil || exe != bin {
		t.Fatalf("settle(home) = %q, %v", exe, err)
	}
	if left, _ := filepath.Glob(filepath.Join(dir, ".one-update-*")); len(left) > 0 {
		t.Fatalf("staging dirs left: %v", left)
	}
}
