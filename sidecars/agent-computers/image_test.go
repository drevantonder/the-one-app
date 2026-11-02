package main

import (
	"strings"
	"testing"
	"time"
)

func TestImageWeekly(t *testing.T) {
	mon := time.Date(2026, 11, 2, 9, 0, 0, 0, time.UTC)
	if imageFor(mon) != imageFor(mon.Add(6*24*time.Hour)) {
		t.Error("a week has one image")
	}
	if imageFor(mon) == imageFor(mon.Add(7*24*time.Hour)) {
		t.Error("each week builds a new image, for Chrome's security fixes")
	}
	if !strings.HasPrefix(imageFor(mon), imageName+":") || !strings.HasSuffix(imageFor(mon), "-2026-45") {
		t.Errorf("image %s", imageFor(mon))
	}
}
