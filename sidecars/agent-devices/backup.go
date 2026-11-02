package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

// A backup is a copy of a device's home folder, without caches, as a blob in
// the install: any server can fetch it, so a device can be restored on
// another (move). Each server keeps an index of its own backups on disk, and
// each backup is an entry in the user's log too. These aren't kept off-site:
// a blob lives on the servers that have fetched it, and a backup to a bucket
// isn't built yet.

type backupRecord struct {
	Device string    `json:"device"`
	Server   string    `json:"server"`
	Blob     string    `json:"blob"`
	Bytes    int64     `json:"bytes"`
	Size     string    `json:"size"` // the device's
	At       time.Time `json:"at"`
}

const keepBackups = 5

func (ds *devices) backupIndex() string { return filepath.Join(ds.dir, "backups.json") }

func (ds *devices) backupsHere() []backupRecord {
	var all []backupRecord
	if b, err := os.ReadFile(ds.backupIndex()); err == nil {
		json.Unmarshal(b, &all)
	}
	return all
}

// backup packs the device's home folder into a blob and records it, keeping
// the newest few in the index.
func (ds *devices) backup(ctx context.Context, d device) (backupRecord, error) {
	me, _, err := ds.servers(ctx)
	if err != nil {
		return backupRecord{}, err
	}
	lims, _ := limits(ctx, []device{d})
	pr, pw := io.Pipe()
	packed := make(chan error, 1)
	go func() {
		args := append([]string{"tar", "-C", home, "--warning=no-file-changed"}, excludes("."+strings.TrimPrefix(profiles, home)+"*", "./.cache")...)
		err := in(ctx, d, nil, pw, append(args, "-czf", "-", ".")...)
		if tarred(err) {
			err = nil
		}
		pw.CloseWithError(err)
		packed <- err
	}()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, ds.node+"/v1/blobs", pr)
	if err != nil {
		return backupRecord{}, err
	}
	req.Header.Set("Content-Type", "application/octet-stream")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return backupRecord{}, err
	}
	defer resp.Body.Close()
	var put struct {
		Blob  string
		Size  int64
		Error string
	}
	json.NewDecoder(resp.Body).Decode(&put)
	if err := <-packed; err != nil {
		return backupRecord{}, fmt.Errorf("packing %s: %w", d.Name, err)
	}
	if resp.StatusCode != http.StatusOK || put.Blob == "" {
		return backupRecord{}, fmt.Errorf("storing the backup: %s", cmpOr(put.Error, resp.Status))
	}
	rec := backupRecord{Device: d.Name, Server: me, Blob: put.Blob, Bytes: put.Size, Size: lims[d.Name].sizeOf(), At: time.Now().UTC()}
	ds.backupMu.Lock()
	defer ds.backupMu.Unlock()
	all := append(ds.backupsHere(), rec)
	var kept []backupRecord
	for i, r := range all {
		newer := 0
		for _, o := range all[i+1:] {
			if o.Device == r.Device {
				newer++
			}
		}
		if newer < keepBackups {
			kept = append(kept, r)
		}
	}
	b, _ := json.MarshalIndent(kept, "", "  ")
	if err := os.WriteFile(ds.backupIndex(), b, 0o600); err != nil {
		return rec, err
	}
	log.Printf("backed up %s: blob %s, %d bytes", d.Name, rec.Blob, rec.Bytes)
	return rec, nil
}

// backupsAll are every server's backups, of one device or all, newest first.
func (ds *devices) backupsAll(ctx context.Context, device string) ([]backupRecord, map[string]string) {
	all := ds.backupsHere()
	got, failed := ds.everywhere(ctx, "backups", nil)
	for _, r := range got {
		var theirs struct{ Backups []backupRecord }
		json.Unmarshal(r, &theirs)
		all = append(all, theirs.Backups...)
	}
	out := []backupRecord{}
	for _, r := range all {
		if device == "" || r.Device == device {
			out = append(out, r)
		}
	}
	slices.SortFunc(out, func(a, b backupRecord) int { return b.At.Compare(a.At) })
	return out, failed
}

// restore makes a device from a backup: a new one, its home folder unpacked
// from the blob, fetched from whichever server has it.
func (ds *devices) restore(ctx context.Context, name string, rec backupRecord, sz size) (device, error) {
	if _, err := find(ctx, name); err == nil {
		return device{}, fmt.Errorf("device %s already exists", name)
	}
	var resp *http.Response
	// Another server serves the blob once the entry naming it reaches it,
	// which takes a moment after a backup.
	for try := 0; ; try++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, ds.node+"/v1/blobs/"+rec.Blob, nil)
		if err != nil {
			return device{}, err
		}
		if resp, err = http.DefaultClient.Do(req); err != nil {
			return device{}, err
		}
		if resp.StatusCode == http.StatusOK {
			break
		}
		msg, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		if try == 5 {
			return device{}, fmt.Errorf("fetching the backup: %s: %s", resp.Status, msg)
		}
		time.Sleep(2 * time.Second)
	}
	defer resp.Body.Close()
	d, err := create(ctx, name, sz, false, true)
	if err != nil {
		return d, err
	}
	fail := func(err error) (device, error) {
		ds.drop(d.Name)
		remove(context.WithoutCancel(ctx), d)
		return device{}, err
	}
	if err := in(ctx, d, resp.Body, nil, "tar", "-C", home, "-xzf", "-"); err != nil {
		return fail(fmt.Errorf("unpacking the backup: %w", err))
	}
	if err := rt.restart(ctx, d); err != nil {
		return fail(err)
	}
	if err := ready(ctx, d); err != nil {
		return fail(err)
	}
	return d, nil
}

// findBackup is the backup with the blob given, or else the newest of the
// device.
func (ds *devices) findBackup(ctx context.Context, device, blob string) (backupRecord, error) {
	all, failed := ds.backupsAll(ctx, "")
	for _, r := range all {
		if blob != "" && r.Blob == blob || blob == "" && r.Device == device {
			return r, nil
		}
	}
	if blob != "" {
		return backupRecord{Device: device, Blob: blob}, nil // a blob from elsewhere
	}
	msg := fmt.Sprintf("no backup of device %s", device)
	if len(failed) > 0 {
		msg += fmt.Sprintf(" (couldn't ask %d servers)", len(failed))
	}
	return backupRecord{}, errors.New(msg)
}
