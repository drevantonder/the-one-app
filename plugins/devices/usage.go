package devices

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	sampleEvery = 3 * time.Second
	keepSamples = 40
)

// Usage is one device's load. CPU and GPU are percent busy, oldest first,
// one sample every sampleEvery; GPU is the busiest GPU, absent if none
// reports.
type Usage struct {
	Memory *Space `json:"memory,omitempty"`
	Disk   *Space `json:"disk,omitempty"`
	CPU    []int  `json:"cpu,omitempty"`
	GPU    []int  `json:"gpu,omitempty"`
}

// Space is in bytes. Used memory is what the OS can't hand to a new program
// without swapping: total minus available on Linux, Activity Monitor's
// "Memory Used" on macOS. Disk total is used plus what's free to the user.
type Space struct {
	Used  uint64 `json:"used"`
	Total uint64 `json:"total"`
}

func (s *Space) percent() int { return int(s.Used * 100 / max(s.Total, 1)) }

func (u Usage) String() string {
	var parts []string
	if m := u.Memory; m != nil {
		parts = append(parts, fmt.Sprintf("RAM %d%% (%s of %s GB)", m.percent(), gb(m.Used), gb(m.Total)))
	}
	if len(u.CPU) > 0 {
		parts = append(parts, fmt.Sprintf("CPU %d%%", u.CPU[len(u.CPU)-1]))
	}
	if len(u.GPU) > 0 {
		parts = append(parts, fmt.Sprintf("GPU %d%%", u.GPU[len(u.GPU)-1]))
	}
	if d := u.Disk; d != nil {
		parts = append(parts, fmt.Sprintf("disk %d%% (%s GB free)", d.percent(), gb(d.Total-d.Used)))
	}
	return strings.Join(parts, " · ")
}

func gb(b uint64) string {
	return strconv.FormatFloat(float64(b)/(1<<30), 'f', 1, 64)
}

// sampler keeps the last few minutes of CPU and GPU load, which only make
// sense as a trend.
type sampler struct {
	once     sync.Once
	mu       sync.Mutex
	cpu, gpu []int
}

func (s *sampler) start() {
	s.once.Do(func() {
		if runtime.GOOS == "darwin" {
			go s.iostat()
		}
		go s.run()
	})
}

func (s *sampler) usage() Usage {
	var u Usage
	if m, err := memory(); err == nil {
		u.Memory = &m
	}
	if d, err := disk(); err == nil {
		u.Disk = &d
	}
	s.mu.Lock()
	u.CPU, u.GPU = slices.Clone(s.cpu), slices.Clone(s.gpu)
	s.mu.Unlock()
	return u
}

func (s *sampler) add(to *[]int, percent int) {
	s.mu.Lock()
	*to = append(*to, min(max(percent, 0), 100))
	if len(*to) > keepSamples {
		*to = (*to)[len(*to)-keepSamples:]
	}
	s.mu.Unlock()
}

func (s *sampler) run() {
	var prev cpuTimes
	for ; ; time.Sleep(sampleEvery) {
		if runtime.GOOS == "linux" {
			if t, err := procStat(); err == nil {
				if prev.total > 0 && t.total > prev.total {
					s.add(&s.cpu, int(100*(t.busy-prev.busy)/(t.total-prev.total)))
				}
				prev = t
			}
		}
		if p, ok := gpuBusy(); ok {
			s.add(&s.gpu, p)
		}
	}
}

// iostat reads macOS's CPU load from one long-running iostat, skipping its
// first line, the average since boot.
func (s *sampler) iostat() {
	for ; ; time.Sleep(time.Minute) {
		cmd := exec.Command("iostat", "-n", "0", "-w", strconv.Itoa(int(sampleEvery.Seconds())))
		out, err := cmd.StdoutPipe()
		if err != nil || cmd.Start() != nil {
			continue
		}
		first := true
		sc := bufio.NewScanner(out)
		for sc.Scan() {
			if idle, ok := iostatIdle(sc.Text()); ok {
				if !first {
					s.add(&s.cpu, 100-idle)
				}
				first = false
			}
		}
		_ = cmd.Wait()
	}
}

// iostatIdle parses a line of `iostat -n 0`: us sy id, then load averages.
func iostatIdle(line string) (int, bool) {
	f := strings.Fields(line)
	if len(f) != 6 {
		return 0, false
	}
	idle, err := strconv.Atoi(f[2])
	return idle, err == nil
}

type cpuTimes struct{ busy, total uint64 }

func procStat() (cpuTimes, error) {
	f, err := os.Open("/proc/stat")
	if err != nil {
		return cpuTimes{}, err
	}
	defer f.Close()
	line, _ := bufio.NewReader(f).ReadString('\n')
	return parseProcStat(line)
}

// parseProcStat reads /proc/stat's first line: user nice system idle iowait
// irq softirq steal, in ticks since boot.
func parseProcStat(line string) (cpuTimes, error) {
	f := strings.Fields(line)
	if len(f) < 5 || f[0] != "cpu" {
		return cpuTimes{}, errors.New("unexpected /proc/stat")
	}
	var t cpuTimes
	for i, v := range f[1:min(len(f), 9)] {
		n, err := strconv.ParseUint(v, 10, 64)
		if err != nil {
			return cpuTimes{}, err
		}
		t.total += n
		if i != 3 && i != 4 { // idle, iowait
			t.busy += n
		}
	}
	return t, nil
}

func gpuBusy() (int, bool) {
	best, found := 0, false
	see := func(p int) { best, found = max(best, p), true }
	switch runtime.GOOS {
	case "darwin":
		out, err := exec.Command("ioreg", "-r", "-d", "1", "-c", "IOAccelerator").Output()
		if err == nil {
			for _, p := range ioregUtilization(string(out)) {
				see(p)
			}
		}
	case "linux":
		if out, err := exec.Command("nvidia-smi", "--query-gpu=utilization.gpu", "--format=csv,noheader,nounits").Output(); err == nil {
			for _, l := range strings.Fields(string(out)) {
				if p, err := strconv.Atoi(l); err == nil {
					see(p)
				}
			}
		}
		paths, _ := filepath.Glob("/sys/class/drm/card*/device/gpu_busy_percent")
		for _, path := range paths {
			if b, err := os.ReadFile(path); err == nil {
				if p, err := strconv.Atoi(strings.TrimSpace(string(b))); err == nil {
					see(p)
				}
			}
		}
	}
	return best, found
}

func ioregUtilization(out string) []int {
	const key = `"Device Utilization %"=`
	var ps []int
	for rest := out; ; {
		_, after, ok := strings.Cut(rest, key)
		if !ok {
			return ps
		}
		end := strings.IndexFunc(after, func(r rune) bool { return r < '0' || r > '9' })
		if end < 0 {
			end = len(after)
		}
		if p, err := strconv.Atoi(after[:end]); err == nil {
			ps = append(ps, p)
		}
		rest = after
	}
}

func memory() (Space, error) {
	switch runtime.GOOS {
	case "linux":
		f, err := os.Open("/proc/meminfo")
		if err != nil {
			return Space{}, err
		}
		defer f.Close()
		return parseMeminfo(f)
	case "darwin":
		total, err := exec.Command("sysctl", "-n", "hw.memsize").Output()
		if err != nil {
			return Space{}, err
		}
		vm, err := exec.Command("vm_stat").Output()
		if err != nil {
			return Space{}, err
		}
		return parseVMStat(string(vm), strings.TrimSpace(string(total)))
	}
	return Space{}, errors.ErrUnsupported
}

func parseMeminfo(r io.Reader) (Space, error) {
	kb := map[string]uint64{}
	sc := bufio.NewScanner(r)
	for sc.Scan() {
		key, rest, ok := strings.Cut(sc.Text(), ":")
		if !ok {
			continue
		}
		if n, err := strconv.ParseUint(strings.TrimSuffix(strings.TrimSpace(rest), " kB"), 10, 64); err == nil {
			kb[key] = n
		}
	}
	total, avail := kb["MemTotal"], kb["MemAvailable"]
	if total == 0 || avail > total {
		return Space{}, errors.New("unexpected /proc/meminfo")
	}
	return Space{Used: (total - avail) * 1024, Total: total * 1024}, nil
}

// parseVMStat computes Activity Monitor's "Memory Used": app memory
// (anonymous minus purgeable), wired and compressed.
func parseVMStat(vm, total string) (Space, error) {
	t, err := strconv.ParseUint(total, 10, 64)
	if err != nil {
		return Space{}, fmt.Errorf("hw.memsize: %w", err)
	}
	lines := strings.Split(vm, "\n")
	var page uint64
	if _, err := fmt.Sscanf(lines[0], "Mach Virtual Memory Statistics: (page size of %d bytes)", &page); err != nil {
		return Space{}, fmt.Errorf("vm_stat: %w", err)
	}
	pages := map[string]uint64{}
	for _, l := range lines[1:] {
		key, rest, ok := strings.Cut(l, ":")
		if !ok {
			continue
		}
		if n, err := strconv.ParseUint(strings.TrimSuffix(strings.TrimSpace(rest), "."), 10, 64); err == nil {
			pages[key] = n
		}
	}
	app := pages["Anonymous pages"] - min(pages["Pages purgeable"], pages["Anonymous pages"])
	used := (app + pages["Pages wired down"] + pages["Pages occupied by compressor"]) * page
	return Space{Used: min(used, t), Total: t}, nil
}
