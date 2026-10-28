package bucket

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"sync"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/service/s3"
)

// Leases give one device at a time the right to write a plugin's consistent
// data or run a job. A lease is an object in the bucket changed only with
// conditional writes, so two devices can never both win it; each takeover
// bumps its epoch. A job's lease also records its last run, so a run is
// claimed in the same conditional write that proves the lease.
const (
	leaseTTL   = 30 * time.Second
	renewAfter = 10 * time.Second // renew once this much of the TTL is used
	takeover   = 5 * time.Second  // wait past expiry before taking over (clock skew)
)

type lease struct {
	Holder    string `json:"holder"`
	Epoch     int64  `json:"epoch"`
	ExpiresMs int64  `json:"expiresMs"`
	LastRunMs int64  `json:"lastRunMs,omitempty"`
}

type cachedLease struct {
	lease   lease
	etag    string
	exists  bool
	fetched time.Time
}

type leases struct {
	mu    sync.Mutex
	cache map[string]cachedLease
}

func leaseKey(name string) string { return "leases/" + name + ".json" }

func (b *Bucket) readLease(ctx context.Context, name string) (cachedLease, error) {
	client, cfg := b.conn()
	if client == nil {
		return cachedLease{}, errors.New("no bucket configured")
	}
	out, err := client.GetObject(ctx, &s3.GetObjectInput{Bucket: &cfg.Bucket, Key: aws.String(leaseKey(name))})
	if err != nil {
		var re interface{ HTTPStatusCode() int }
		if errors.As(err, &re) && re.HTTPStatusCode() == 404 {
			return cachedLease{fetched: time.Now()}, nil
		}
		return cachedLease{}, err
	}
	defer out.Body.Close()
	var l lease
	if err := json.NewDecoder(out.Body).Decode(&l); err != nil {
		return cachedLease{}, err
	}
	return cachedLease{lease: l, etag: aws.ToString(out.ETag), exists: true, fetched: time.Now()}, nil
}

// writeLease replaces the lease only if it is unchanged since read.
func (b *Bucket) writeLease(ctx context.Context, name string, prev cachedLease, next lease) (cachedLease, bool, error) {
	client, cfg := b.conn()
	body, _ := json.Marshal(next)
	in := &s3.PutObjectInput{Bucket: &cfg.Bucket, Key: aws.String(leaseKey(name)), Body: bytes.NewReader(body), ContentType: aws.String("application/json")}
	if prev.exists {
		in.IfMatch = aws.String(prev.etag)
	} else {
		in.IfNoneMatch = aws.String("*")
	}
	out, err := client.PutObject(ctx, in)
	if err != nil {
		if isPreconditionFailed(err) {
			return cachedLease{}, false, nil // someone else changed it first
		}
		return cachedLease{}, false, err
	}
	return cachedLease{lease: next, etag: aws.ToString(out.ETag), exists: true, fetched: time.Now()}, true, nil
}

func (b *Bucket) cached(name string) (cachedLease, bool) {
	b.leases.mu.Lock()
	defer b.leases.mu.Unlock()
	c, ok := b.leases.cache[name]
	return c, ok
}

func (b *Bucket) remember(name string, c cachedLease) {
	b.leases.mu.Lock()
	defer b.leases.mu.Unlock()
	if b.leases.cache == nil {
		b.leases.cache = map[string]cachedLease{}
	}
	b.leases.cache[name] = c
}

// claim reads the lease and, when this device may hold it, writes next(l)
// conditionally. It reports the lease after the attempt and whether this
// device holds it. tookOver is true when the lease came from another device.
func (b *Bucket) claim(ctx context.Context, name string, next func(want, old lease, mine bool) (lease, bool)) (lease, bool, bool, error) {
	me := b.store.Device()
	for range 3 {
		cur, err := b.readLease(ctx, name)
		if err != nil {
			return lease{}, false, false, err
		}
		now := time.Now().UnixMilli()
		l := cur.lease
		mine := cur.exists && l.Holder == me && l.ExpiresMs > now
		free := !cur.exists || l.ExpiresMs+takeover.Milliseconds() < now
		if !mine && !free {
			b.remember(name, cur)
			return l, false, false, nil
		}
		want := l
		if !mine {
			want = lease{Holder: me, Epoch: l.Epoch + 1, LastRunMs: l.LastRunMs}
		}
		want.ExpiresMs = now + leaseTTL.Milliseconds()
		want, write := next(want, l, mine)
		if !write && mine {
			return l, true, false, nil
		}
		got, ok, err := b.writeLease(ctx, name, cur, want)
		if err != nil {
			return lease{}, false, false, err
		}
		if ok {
			b.remember(name, got)
			return got.lease, true, !mine && cur.exists && l.Holder != me, nil
		}
	}
	return lease{}, false, false, errors.New("lease contended; try again")
}

// Acquire implements core.Coordinator.
func (b *Bucket) Acquire(ctx context.Context, name string) (string, bool, error) {
	me := b.store.Device()
	if c, ok := b.cached(name); ok && c.exists && c.lease.Holder == me &&
		time.Until(time.UnixMilli(c.lease.ExpiresMs)) > leaseTTL-renewAfter {
		return me, true, nil // recently written by us: still well within its TTL
	}
	l, mine, tookOver, err := b.claim(ctx, name, func(want, _ lease, _ bool) (lease, bool) { return want, true })
	if err != nil {
		return "", false, err
	}
	if tookOver {
		// The previous holder flushed every acknowledged write to the bucket:
		// catch up before writing anything ourselves.
		if err := b.downloadAll(ctx); err != nil {
			return "", false, err
		}
	}
	if mine {
		return me, true, nil
	}
	return l.Holder, false, nil
}

// ClaimRun implements core.Coordinator.
func (b *Bucket) ClaimRun(ctx context.Context, name string, every time.Duration) (bool, error) {
	ran := false
	_, mine, _, err := b.claim(ctx, name, func(want, old lease, mine bool) (lease, bool) {
		now := time.Now().UnixMilli()
		if now-want.LastRunMs >= every.Milliseconds() {
			want.LastRunMs, ran = now, true
			return want, true
		}
		// Not due: only write to take the lease over, or to renew it once
		// renewAfter has passed since it was last written.
		written := old.ExpiresMs - leaseTTL.Milliseconds()
		return want, !mine || now-written >= renewAfter.Milliseconds()
	})
	return mine && ran, err
}

// Flush implements core.Coordinator: upload this device's entries now.
func (b *Bucket) Flush(ctx context.Context) error {
	if !b.Configured() {
		return nil
	}
	b.mu.Lock()
	listed := !b.lastDown.IsZero()
	b.mu.Unlock()
	if !listed {
		if err := b.downloadAll(ctx); err != nil {
			return err
		}
	}
	vec, err := b.store.Vector()
	if err != nil {
		return err
	}
	return b.upload(ctx, map[string]int64{b.store.Device(): vec[b.store.Device()]})
}
