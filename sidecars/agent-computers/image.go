package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// Computers run Cua's image with Google Chrome in place of its Chromium, so
// the person can sign in to the browser as their Google account. Chrome's
// terms don't let us pass it on, so each server builds the image itself,
// downloading Chrome from Google, and builds it again each week for Chrome's
// security fixes. Cua's image turns browser sign-in and sync off for
// Chromium; Chrome's policies here leave them to the person.
const recipe = `FROM ` + cuaImage + `
RUN curl -fsSLo /tmp/chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_$(dpkg --print-architecture).deb \
 && apt-get update && apt-get install -y /tmp/chrome.deb \
 && apt-get purge -y chromium chromium-common && apt-get autoremove -y \
 && rm -rf /tmp/chrome.deb /var/lib/apt/lists/* /etc/apt/sources.list.d/google-chrome.list /etc/chromium /etc/chromium.d \
 && mkdir -p /etc/opt/chrome/policies/managed \
 && echo '{"BrowserSignin":1,"PromotionsEnabled":false,"DefaultBrowserSettingEnabled":false,"MetricsReportingEnabled":false,"PrivacySandboxPromptEnabled":false,"TranslateEnabled":false,"CommandLineFlagSecurityWarningsEnabled":false}' \
  > /etc/opt/chrome/policies/managed/one.json
`

const imageName = "localhost/one-agent-computer"

// imageFor is the week's image: a new recipe or a new week is a new one.
func imageFor(now time.Time) string {
	sum := sha256.Sum256([]byte(recipe))
	y, w := now.ISOWeek()
	return fmt.Sprintf("%s:%s-%d-%02d", imageName, hex.EncodeToString(sum[:4]), y, w)
}

var building sync.Mutex

// ensureImage builds this week's image if this server hasn't yet, then
// deletes the older ones no computer uses.
func ensureImage(ctx context.Context) (string, error) {
	building.Lock()
	defer building.Unlock()
	img := imageFor(time.Now())
	if rt.hasImage(ctx, img) {
		return img, nil
	}
	dir, err := os.MkdirTemp("", "agent-computer-image")
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(dir)
	if err := os.WriteFile(filepath.Join(dir, "Containerfile"), []byte(recipe), 0o600); err != nil {
		return "", err
	}
	log.Printf("building %s, with Chrome from Google", img)
	start := time.Now()
	if err := rt.build(context.WithoutCancel(ctx), img, dir); err != nil {
		return "", fmt.Errorf("building the computers' image: %w", err)
	}
	log.Printf("built %s in %s", img, time.Since(start).Round(time.Second))
	old, err := rt.images(ctx)
	if err != nil {
		log.Print(err)
	}
	for _, o := range old {
		if strings.HasPrefix(o, imageName+":") && o != img {
			rt.removeImage(ctx, o) // refused while a computer runs on it
		}
	}
	return img, nil
}
