package main

import (
	"bytes"
	"cmp"
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/hkdf"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
)

// A browser profile is a Vault item of the person's: its profile folder, as a
// tarball without caches, is a blob encrypted with a key made from the item's
// secret field and a salt of its own, so each save has a fresh key. The app
// over this service names the item in its calls, and the node fills in the
// secret, so only this process ever holds it or the profile; the app records
// each save on the item (its blob, salt and sites), and nothing here keeps a
// profile once its device is gone.

// A profile is what a call says of the browser profile a device starts with.
type profile struct {
	Item string `json:"item"`           // the Vault item's id
	Name string `json:"name"`           // its title, the profile's folder in the device
	Key  string `json:"key"`            // its secret field, filled in by the node
	Blob string `json:"blob,omitempty"` // its last save; none for a new one
	Salt string `json:"salt,omitempty"`
}

// profileIn is the profile a call's input gives, or nil.
func profileIn(in map[string]any) (*profile, error) {
	raw, ok := in["profile"]
	if !ok || raw == nil {
		return nil, nil
	}
	b, _ := json.Marshal(raw)
	var p profile
	if err := json.Unmarshal(b, &p); err != nil || p.Item == "" {
		return nil, errors.New("profile is a browser profile as the app gives it")
	}
	p.Name = folder(p.Name)
	if strings.Contains(p.Key, "{{") || p.Key == "" {
		return nil, errors.New("the profile's key wasn't filled in: the app isn't switched on for it in the Vault")
	}
	return &p, nil
}

// folder is the profile's folder in a device, named as its item is, as far
// as the browser tools allow: letters, digits and dashes.
func folder(title string) string {
	b := []byte(strings.TrimSpace(title))
	for i, c := range b {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '-') {
			b[i] = '-'
		}
	}
	name := strings.Trim(string(b[:min(len(b), 40)]), "-")
	return cmp.Or(name, "profile")
}

// sealer is the cipher for a profile's save with salt.
func sealer(key, salt string) (cipher.AEAD, error) {
	k, err := hkdf.Key(sha256.New, []byte(key), []byte(salt), "browser profile", 32)
	if err != nil {
		return nil, err
	}
	block, err := aes.NewCipher(k)
	if err != nil {
		return nil, err
	}
	return cipher.NewGCM(block)
}

func seal(key, salt string, plain []byte) ([]byte, error) {
	gcm, err := sealer(key, salt)
	if err != nil {
		return nil, err
	}
	nonce := make([]byte, gcm.NonceSize())
	rand.Read(nonce)
	return gcm.Seal(nonce, nonce, plain, nil), nil
}

func unseal(key, salt string, sealed []byte) ([]byte, error) {
	gcm, err := sealer(key, salt)
	if err != nil {
		return nil, err
	}
	if len(sealed) < gcm.NonceSize() {
		return nil, errors.New("the saved profile is cut short")
	}
	plain, err := gcm.Open(nil, sealed[:gcm.NonceSize()], sealed[gcm.NonceSize():], nil)
	if err != nil {
		return nil, errors.New("the saved profile doesn't open with its key")
	}
	return plain, nil
}

// unpack puts the profile's last save in the device's browser profiles, or
// an empty profile for one never saved.
func (ds *devices) unpack(ctx context.Context, d device, p *profile) error {
	if p.Blob == "" {
		return sh(ctx, d, owned(p.Name))
	}
	sealed, err := ds.getBlob(ctx, p.Blob)
	if err != nil {
		return fmt.Errorf("fetching the %s profile: %w", p.Name, err)
	}
	plain, err := unseal(p.Key, p.Salt, sealed)
	if err != nil {
		return err
	}
	dir := profiles + p.Name
	if err := in(ctx, d, bytes.NewReader(plain), nil, "sh", "-c", `rm -rf "$1" && mkdir -p "$1" && tar -C "$1" -xzf - && `+owned(p.Name), "sh", dir); err != nil {
		return fmt.Errorf("unpacking the %s profile: %w", p.Name, err)
	}
	return nil
}

// A save is what saving a profile made: its blob, salt and sites, for the app
// to record on its item.
type save struct {
	Item  string `json:"item"`
	Blob  string `json:"blob"`
	Salt  string `json:"salt"`
	Sites string `json:"sites"`
}

// pack closes the device's browsers, so the profile is written out, and
// saves it as a new blob, encrypted with a fresh salt; the app records it.
func (ds *devices) pack(ctx context.Context, d device, p *profile) (save, error) {
	if err := sh(ctx, d, "test -d "+profiles+p.Name); err != nil {
		return save{}, fmt.Errorf("device %s's browser has no %s profile", d.Name, p.Name)
	}
	closeBrowsers(ctx, d, true)
	args := append([]string{"tar", "-C", profiles + p.Name, "--warning=no-file-changed"}, excludes(".")...)
	var tarball bytes.Buffer
	if err := in(ctx, d, nil, &tarball, append(args, "-czf", "-", ".")...); !tarred(err) {
		return save{}, err
	}
	salt := make([]byte, 16)
	rand.Read(salt)
	s := save{Item: p.Item, Salt: hex.EncodeToString(salt), Sites: sites(ctx, d, p.Name)}
	sealed, err := seal(p.Key, s.Salt, tarball.Bytes())
	if err != nil {
		return save{}, err
	}
	if s.Blob, err = ds.putBlob(ctx, sealed); err != nil {
		return save{}, err
	}
	log.Printf("saved the %s profile from %s: %d bytes", p.Name, d.Name, len(sealed))
	return s, nil
}

// saved has the app record a save on the profile's item. A save made while a
// call of the app's waits (save_profile) is recorded the same way.
func (ds *devices) saved(ctx context.Context, s save) error {
	if _, err := ds.callApp(ctx, "_saved", s); err != nil {
		return fmt.Errorf("recording the save: %w", err)
	}
	return nil
}

// history reads a Chrome profile's History: each site's registrable domain
// (example.com, example.com.au) with the day it was last used, newest first,
// at most 30. Python's sqlite3 is in every device, so this needs nothing
// new; a short list of second-level labels stands in for the public suffix
// list.
const history = `
import sqlite3, sys, datetime, shutil, tempfile
from urllib.parse import urlsplit
db = tempfile.mktemp()
shutil.copy(sys.argv[1], db)
second = {"com", "co", "org", "net", "gov", "edu", "ac", "id", "asn"}
last = {}
for url, t in sqlite3.connect(db).execute("select url, last_visit_time from urls where url like 'http%'"):
    host = (urlsplit(url).hostname or "").lower()
    labels = [l for l in host.split(".") if l]
    if len(labels) < 2 or host.replace(".", "").isdigit():
        continue
    n = 3 if len(labels) > 2 and len(labels[-1]) == 2 and labels[-2] in second else 2
    site = ".".join(labels[-n:])
    last[site] = max(last.get(site, 0), t)
day = lambda t: (datetime.datetime(1601, 1, 1) + datetime.timedelta(microseconds=t)).strftime("%Y-%m-%d")
print(", ".join(f"{s} {day(t)}" for s, t in sorted(last.items(), key=lambda x: -x[1])[:30]))
`

// sites are the sites the profile was used on, as history reads them; "" if
// it has no history yet.
func sites(ctx context.Context, d device, name string) string {
	var out bytes.Buffer
	file := profiles + name + "/Default/History"
	if err := in(ctx, d, strings.NewReader(history), &out, "sh", "-c", `test -f "$1" && python3 - "$1" || true`, "sh", file); err != nil {
		log.Printf("reading the %s profile's sites: %v", name, err)
		return ""
	}
	return strings.TrimSpace(out.String())
}

func (ds *devices) putBlob(ctx context.Context, b []byte) (string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, ds.node+"/v1/blobs", bytes.NewReader(b))
	if err != nil {
		return "", err
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	var put struct {
		Blob  string `json:"blob"`
		Error string `json:"error"`
	}
	json.NewDecoder(resp.Body).Decode(&put)
	if resp.StatusCode != http.StatusOK || put.Blob == "" {
		return "", fmt.Errorf("putting the blob: %s %s", resp.Status, put.Error)
	}
	return put.Blob, nil
}

func (ds *devices) getBlob(ctx context.Context, hash string) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, ds.node+"/v1/blobs/"+hash, nil)
	if err != nil {
		return nil, err
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s: %s", resp.Status, b)
	}
	return b, nil
}
