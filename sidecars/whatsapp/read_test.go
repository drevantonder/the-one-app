package main

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// Reading must never tell WhatsApp anything: no read receipts, no presence.
func TestNothingTellsWhatsAppMessagesWereRead(t *testing.T) {
	files, _ := filepath.Glob("*.go")
	told := regexp.MustCompile(`\.(MarkRead|SendPresence|SendChatPresence|SendReceipt)\(`)
	for _, file := range files {
		if strings.HasSuffix(file, "_test.go") {
			continue
		}
		src, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		if told.Match(src) {
			t.Errorf("%s tells WhatsApp the user read or is online: %s", file, told.Find(src))
		}
	}
}
