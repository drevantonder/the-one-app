package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

// A size is how much of its server a desktop may use, offered by name.
type size struct {
	Name string
	CPUs int
	GB   int // memory
}

var sizes = []size{{"small", 2, 4}, {"medium", 4, 8}, {"large", 8, 16}}

func sizeNamed(name string) (size, error) {
	if name == "" {
		return sizes[0], nil
	}
	var names []string
	for _, s := range sizes {
		if strings.EqualFold(s.Name, name) {
			return s, nil
		}
		names = append(names, s.Name)
	}
	return size{}, fmt.Errorf("no size %q: there's %s", name, strings.Join(names, ", "))
}

func (s size) limits() []string {
	return []string{"--cpus", fmt.Sprint(s.CPUs), "--memory", fmt.Sprintf("%dg", s.GB), "--memory-swap", fmt.Sprintf("%dg", 2*s.GB)}
}

func (s size) String() string { return fmt.Sprintf("%s (%d CPUs, %d GB)", s.Name, s.CPUs, s.GB) }

func sizesText() string {
	var names []string
	for _, s := range sizes {
		names = append(names, s.String())
	}
	return strings.Join(names, ", ")
}

// limit is what a desktop may use, as its container says.
type limit struct {
	CPUs    float64 // 0: no limit
	Memory  int64   // bytes; 0: no limit
	Restart string  // the restart policy
}

// sizeOf names a desktop's limits; desktops made before sizes, with only a
// memory limit, are named by their memory.
func (l limit) sizeOf() string {
	for _, s := range sizes {
		if l.Memory == int64(s.GB)<<30 && (l.CPUs == 0 || l.CPUs == float64(s.CPUs)) {
			return s.Name
		}
	}
	return "custom"
}

// limits are the desktops' limits, by name.
func limits(ctx context.Context, all []desktop) (map[string]limit, error) {
	out := map[string]limit{}
	if len(all) == 0 {
		return out, nil
	}
	args := []string{"inspect", "--format", "json"}
	for _, d := range all {
		args = append(args, prefix+d.Name)
	}
	raw, err := podman(ctx, args...)
	if err != nil {
		return nil, err
	}
	var got []struct {
		Name       string
		HostConfig struct {
			Memory        int64
			NanoCpus      int64
			RestartPolicy struct{ Name string }
		}
	}
	if err := json.Unmarshal(raw, &got); err != nil {
		return nil, err
	}
	for _, g := range got {
		out[strings.TrimPrefix(strings.TrimPrefix(g.Name, "/"), prefix)] = limit{CPUs: float64(g.HostConfig.NanoCpus) / 1e9, Memory: g.HostConfig.Memory, Restart: g.HostConfig.RestartPolicy.Name}
	}
	return out, nil
}

// resize changes what a running or stopped desktop may use, at once.
func resize(ctx context.Context, d desktop, s size) error {
	_, err := podman(ctx, append([]string{"update"}, append(s.limits(), prefix+d.Name)...)...)
	return err
}

// usage is what a desktop uses now: CPU in percent of one core, memory in
// bytes.
type usage struct {
	CPU    float64
	Memory int64
}

func stats(ctx context.Context) (map[string]usage, error) {
	out, err := podman(ctx, "stats", "--no-stream", "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		Name string `json:"name"`
		CPU  string `json:"cpu_percent"`
		Mem  string `json:"mem_usage"` // "88.25MB / 4.295GB"
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	all := map[string]usage{}
	for _, r := range raw {
		name, ok := strings.CutPrefix(r.Name, prefix)
		if !ok {
			continue
		}
		var cpu float64
		fmt.Sscan(strings.TrimSuffix(r.CPU, "%"), &cpu)
		mem, _, _ := strings.Cut(r.Mem, " /")
		all[name] = usage{CPU: cpu, Memory: bytesOf(mem)}
	}
	return all, nil
}

// bytesOf reads a size as Podman writes it, like 88.25MB.
func bytesOf(s string) int64 {
	s = strings.TrimSpace(s)
	i := strings.IndexFunc(s, func(r rune) bool { return (r < '0' || r > '9') && r != '.' })
	if i < 0 {
		i = len(s)
	}
	var n float64
	fmt.Sscan(s[:i], &n)
	scale := map[string]float64{"b": 1, "kb": 1e3, "mb": 1e6, "gb": 1e9, "tb": 1e12, "kib": 1 << 10, "mib": 1 << 20, "gib": 1 << 30}[strings.ToLower(s[i:])]
	if scale == 0 {
		scale = 1
	}
	return int64(n * scale)
}
