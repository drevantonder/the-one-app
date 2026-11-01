package main

import (
	"os"
	"testing"
	"time"
)

func TestSpares(t *testing.T) {
	all := []desktop{{Name: "main", State: "running"}, {Name: "pool-a", State: "running", pool: true},
		{Name: "pool-b", State: "paused", pool: true}, {Name: "pool-c", State: "stopped", pool: true}, {Name: "pool-d", State: "running", pool: true}}
	got := spares(all, map[string]loan{"pool-d": {By: "x", For: "y"}})
	if len(got) != 2 || got[0].Name != "pool-a" || got[1].Name != "pool-b" {
		t.Errorf("unborrowed pool desktops that aren't stopped: %v", got)
	}
}

func TestSizes(t *testing.T) {
	if s, err := sizeNamed(""); err != nil || s.Name != "small" {
		t.Errorf("small by default: %v %v", s, err)
	}
	if s, err := sizeNamed("Medium"); err != nil || s.CPUs != 4 || s.GB != 8 {
		t.Errorf("by name: %v %v", s, err)
	}
	if _, err := sizeNamed("huge"); err == nil {
		t.Error("an unknown size must be an error")
	}
	if n := (limit{Memory: 4 << 30}).sizeOf(); n != "small" {
		t.Errorf("a desktop from before sizes is named by its memory: %s", n)
	}
	if n := (limit{Memory: 16 << 30, CPUs: 8}).sizeOf(); n != "large" {
		t.Errorf("%s", n)
	}
	if n := (limit{Memory: 3 << 30}).sizeOf(); n != "custom" {
		t.Errorf("%s", n)
	}
}

func TestBytesOf(t *testing.T) {
	for in, want := range map[string]int64{"88.25MB": 88250000, "1.5GiB": 3 << 29, "512kB": 512000, "0B": 0} {
		if got := bytesOf(in); got != want {
			t.Errorf("%s: %d, want %d", in, got, want)
		}
	}
}

func TestQuote(t *testing.T) {
	if q := quote("https://x.com/a?b='c'"); q != `'https://x.com/a?b='\''c'\'''` {
		t.Errorf("%s", q)
	}
}

func TestIdle(t *testing.T) {
	ds := newDesktops(t.TempDir(), "")
	if d := ds.idle("main"); d != 0 {
		t.Errorf("first seen counts as just used: %v", d)
	}
	ds.poolMu.Lock()
	ds.used["main"] = time.Now().Add(-time.Hour)
	ds.poolMu.Unlock()
	if d := ds.idle("main"); d < 59*time.Minute {
		t.Errorf("%v", d)
	}
}

func TestLoansKept(t *testing.T) {
	dir := t.TempDir()
	ds := newDesktops(dir, "")
	ds.setLoan("pool-a", &loan{By: "Messenger", For: "Reading chats"})
	if l := newDesktops(dir, "").loans()["pool-a"]; l.By != "Messenger" {
		t.Errorf("loans outlive the program: %v", l)
	}
	ds.setLoan("pool-a", nil)
	if _, ok := newDesktops(dir, "").loans()["pool-a"]; ok {
		t.Error("a given-back loan is forgotten")
	}
}

func TestAddedProfileListed(t *testing.T) {
	ds := newDesktops(t.TempDir(), "")
	if ds.keeps("PTC") {
		t.Fatal("no profile yet")
	}
	os.MkdirAll(ds.loginDir("PTC"), 0o700)
	got := ds.logins("linux")
	if !ds.keeps("PTC") || len(got) != 1 || got[0]["profile"] != "PTC" || got[0]["saved"] != nil {
		t.Errorf("an added profile is listed, unsaved: %v", got)
	}
}
