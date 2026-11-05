package main

import (
	"log"
	"os"
	"time"
)

// probes tries each native gap a shell must fill beyond MyGo's own: a menu bar or tray item (tray.go), push
// registration and a passkey (per platform, probe_*.go). Each says what it found in the log; ONE_PROBES=0 skips them.
func probes() {
	if os.Getenv("ONE_PROBES") == "0" {
		return
	}
	trayProbe()
	go func() {
		// A passkey's sheet hangs from a window: the home window, once it's there.
		for homeWin == nil {
			time.Sleep(100 * time.Millisecond)
		}
		time.Sleep(time.Second)
		log.Printf("probe passkey: %s", passkeyProbe())
		log.Printf("probe push: %s", pushProbe())
	}()
}
