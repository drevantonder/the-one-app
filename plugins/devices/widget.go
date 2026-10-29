package devices

import (
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk/widget"
)

// Device outlines, drawn in a 24-point box.
var icons = map[string]string{
	"laptop":    "M6 6h12a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM3 18h18",
	"server":    "M5 4h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM5 13h14a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1zM8 7.5h.01M8 16.5h.01",
	"phone":     "M9 3h6a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM11 18h2",
	"desktop":   "M4 4h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM9 20h6M12 16v4",
	"container": "M3.5 6h17a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM6.5 9v6M10 9v6M14 9v6M17.5 9v6",
}

// Columns' widths, shared by the header and every row so they line up.
const ramW, cpuW, gpuW, diskW, statGap = 18, 36, 36, 30, 9

func iconFor(d map[string]any) widget.Node {
	os, _ := d["os"].(string)
	name := map[string]string{"macOS": "laptop", "linux": "server", "iOS": "phone", "android": "phone"}[os]
	if name == "" {
		name = "desktop"
	}
	if _, guest := d["host"].(string); guest {
		name = "container"
	}
	logo := logos[map[string]string{"macOS": "apple", "iOS": "apple", "linux": "linux", "android": "android", "windows": "windows"}[os]]
	return widget.Icon(icons[name], true, 22, logo)
}

func level(used, total uint64) string {
	switch p := float64(used) / float64(total); {
	case p >= .85:
		return "red"
	case p >= .7:
		return "orange"
	}
	return "green"
}

func floats(v []int) []float64 {
	out := make([]float64, len(v))
	for i, x := range v {
		out[i] = float64(x)
	}
	return out
}

func cell(w float64, n widget.Node) widget.Node {
	return widget.Col(widget.Stack{W: w, Align: "center"}, n)
}

func stats(u *Usage) widget.Node {
	var ram, disk widget.Node = widget.Spacer(0), widget.Spacer(0)
	if m := u.Memory; m != nil && m.Total > 0 {
		ram = widget.Gauge(float64(m.Used)/float64(m.Total), level(m.Used, m.Total), 18)
	}
	if d := u.Disk; d != nil && d.Total > 0 {
		disk = widget.Progress(float64(d.Used)/float64(d.Total), "", diskW, level(d.Used, d.Total))
	}
	return widget.Row(widget.Stack{Gap: statGap},
		cell(ramW, ram),
		cell(cpuW, widget.Spark(floats(u.CPU), 100, "blue", cpuW, 16)),
		cell(gpuW, widget.Spark(floats(u.GPU), 100, "purple", gpuW, 16)),
		cell(diskW, disk))
}

func header() widget.Node {
	label := func(w float64, s string) widget.Node {
		return cell(w, widget.Text(s, widget.Style{Size: "xs", Weight: "medium", Tone: "muted"}))
	}
	return widget.Row(widget.Stack{Gap: 10},
		widget.Col(widget.Stack{Grow: true}, widget.Text("Devices", widget.Style{Size: "sm", Weight: "bold", Tone: "muted"})),
		widget.Row(widget.Stack{Gap: statGap}, label(ramW, "RAM"), label(cpuW, "CPU"), label(gpuW, "GPU"), label(diskW, "DISK")))
}

func row(d map[string]any) widget.Node {
	name, _ := d["name"].(string)
	online, _ := d["online"].(bool)
	small := widget.Style{Size: "sm", Tone: "muted", Lines: 1}
	var detail widget.Node
	if online {
		var parts []string
		if self, _ := d["self"].(bool); self {
			parts = append(parts, "This device")
		}
		if host, ok := d["host"].(string); ok {
			parts = append(parts, "On "+host)
		}
		if v, _ := d["version"].(string); v != "" {
			parts = append(parts, "v"+strings.TrimPrefix(v, "v"))
		}
		detail = widget.Text(strings.Join(parts, " · "), small)
	} else if seen, err := time.Parse(time.RFC3339, str(d["lastSeen"])); err == nil {
		detail = widget.Row(widget.Stack{Gap: 3}, widget.Text("Last seen", small), widget.Time(seen, "relative", small))
	} else {
		detail = widget.Text("Offline", small)
	}
	var end widget.Node
	var u Usage
	if online && recode(d["usage"], &u) == nil && (u.Memory != nil || u.Disk != nil || len(u.CPU) > 0) {
		end = stats(&u)
	} else if online {
		end = widget.Dot("green")
	}
	children := []widget.Node{
		iconFor(d),
		widget.Col(widget.Stack{Grow: true, Gap: 1}, widget.Text(name, widget.Style{Lines: 1, Size: "sm"}), detail),
	}
	if end != nil {
		children = append(children, end)
	}
	return widget.Row(widget.Stack{Gap: 10, Dim: !online}, children...)
}

func str(v any) string {
	s, _ := v.(string)
	return s
}

func devicesWidget(devices []map[string]any) widget.Node {
	rows := []widget.Node{header()}
	for _, d := range devices {
		rows = append(rows, row(d))
	}
	return widget.Col(widget.Stack{Gap: 8}, rows...)
}
