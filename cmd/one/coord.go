package main

import (
	"context"
	"encoding/json"
	"time"

	"github.com/drevantonder/the-one-app/internal/bucket"
	"github.com/drevantonder/the-one-app/internal/mesh"
	"github.com/drevantonder/the-one-app/protocol"
)

// coordinator joins the bucket (leases) and the mesh (forwarding). Until a
// bucket is configured there is nothing to coordinate through, so this
// device acts alone: it holds every lease and runs every job itself.
type coordinator struct {
	b      *bucket.Bucket
	m      *mesh.Mesh
	device string
}

func (c coordinator) Acquire(ctx context.Context, lease string) (string, bool, error) {
	if !c.b.Configured() {
		return c.device, true, nil
	}
	return c.b.Acquire(ctx, lease)
}

var localRuns = map[string]time.Time{}

func (c coordinator) ClaimRun(ctx context.Context, lease string, every time.Duration) (bool, error) {
	if !c.b.Configured() {
		if time.Since(localRuns[lease]) < every {
			return false, nil
		}
		localRuns[lease] = time.Now()
		return true, nil
	}
	return c.b.ClaimRun(ctx, lease, every)
}

func (c coordinator) Flush(ctx context.Context) error { return c.b.Flush(ctx) }

func (c coordinator) Forward(ctx context.Context, device, plugin, op string, input json.RawMessage, caller protocol.Caller) (protocol.Result, error) {
	return c.m.Forward(ctx, device, plugin, op, input, caller)
}
