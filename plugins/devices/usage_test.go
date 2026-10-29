package devices

import (
	"strings"
	"testing"
)

func TestParseVMStat(t *testing.T) {
	vm := `Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                                    11770.
Pages wired down:                             232264.
Pages purgeable:                                9035.
Anonymous pages:                              339790.
Pages occupied by compressor:                 290212.
`
	m, err := parseVMStat(vm, "34359738368")
	if err != nil {
		t.Fatal(err)
	}
	if want := uint64(339790-9035+232264+290212) * 16384; m.Used != want || m.Total != 34359738368 {
		t.Fatalf("got %+v, want used %d", m, want)
	}
}

func TestParseMeminfo(t *testing.T) {
	m, err := parseMeminfo(strings.NewReader("MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:   12000000 kB\n"))
	if err != nil || m.Used != 4000000*1024 || m.Total != 16000000*1024 {
		t.Fatalf("got %+v, %v", m, err)
	}
}

func TestParseProcStat(t *testing.T) {
	c, err := parseProcStat("cpu  100 10 50 800 40 0 0 0 0 0\n")
	if err != nil || c.busy != 160 || c.total != 1000 {
		t.Fatalf("got %+v, %v", c, err)
	}
}

func TestIostatIdle(t *testing.T) {
	for line, want := range map[string]int{"      cpu    load average": -1, " us sy id   1m   5m   15m": -1, " 20 11 69  2.85 3.36 3.24": 69} {
		idle, ok := iostatIdle(line)
		if (want < 0) == ok || ok && idle != want {
			t.Errorf("%q: got %d %v", line, idle, ok)
		}
	}
}

func TestIoregUtilization(t *testing.T) {
	out := `+-o AGXAcceleratorG15X  <class AGXAcceleratorG15X>
    "PerformanceStatistics" = {"Tiler Utilization %"=12,"Device Utilization %"=46,"Renderer Utilization %"=44}`
	if got := ioregUtilization(out); len(got) != 1 || got[0] != 46 {
		t.Fatalf("got %v", got)
	}
}
