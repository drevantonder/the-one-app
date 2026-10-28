// Command jobs is an example of the two single-writer features: a job that
// runs once per interval across all of a user's devices, and consistent
// entries (a task can only be claimed once, even from two devices at once).
//
//	one plugins link ./examples/jobs && one plugins publish jobs
package main

import (
	"fmt"
	"os"
	"sort"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

type beat struct {
	Host string `json:"host"`
}

type claim struct {
	Task string `json:"task" jsonschema:"what to claim"`
	By   string `json:"by,omitempty"`
}

type noIn struct{}

func main() {
	sdk.Run(sdk.Plugin{ID: "jobs", Version: "0.1.0", Setup: func(c *sdk.Ctx) error {
		var beats []sdk.Entry
		hosts := map[string]int{}
		claims := map[string]string{}
		c.Entry("beat", sdk.Available)
		c.Entry("claim", sdk.Consistent)
		c.OnReset(func() { beats, hosts, claims = nil, map[string]int{}, map[string]string{} })
		sdk.Handle(c, "beat", func(e sdk.Entry, in beat) error {
			beats = append(beats, e)
			hosts[in.Host]++
			return nil
		})
		sdk.Handle(c, "claim", func(_ sdk.Entry, in claim) error {
			if _, taken := claims[in.Task]; !taken {
				claims[in.Task] = in.By
			}
			return nil
		})

		c.Every("beat", 30*time.Second, func(call *sdk.Call) error {
			host, _ := os.Hostname()
			_, err := call.Append("beat", beat{Host: host})
			return err
		})

		sdk.Op(c, "beats", "Count job runs, and which devices ran them.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
			var names []string
			for h := range hosts {
				names = append(names, h)
			}
			sort.Strings(names)
			var b strings.Builder
			fmt.Fprintf(&b, "%d runs", len(beats))
			for _, h := range names {
				fmt.Fprintf(&b, "\n  %s: %d", h, hosts[h])
			}
			if len(beats) > 1 {
				var gaps []string
				for i := 1; i < len(beats); i++ {
					t0, _ := time.Parse(time.RFC3339, beats[i-1].Time)
					t1, _ := time.Parse(time.RFC3339, beats[i].Time)
					gaps = append(gaps, t1.Sub(t0).String())
				}
				fmt.Fprintf(&b, "\ngaps: %s", strings.Join(gaps, " "))
			}
			return sdk.Text(b.String()), nil
		})
		sdk.Op(c, "claim", "Claim a task. Only the first claim wins, across every device.", sdk.Write, func(call *sdk.Call, in claim) (sdk.Result, error) {
			if by, taken := claims[in.Task]; taken {
				return sdk.Text(fmt.Sprintf("%q is already claimed by %s.", in.Task, by)), nil
			}
			host, _ := os.Hostname()
			if _, err := call.Append("claim", claim{Task: in.Task, By: host}); err != nil {
				return sdk.Result{}, err
			}
			return sdk.Text(fmt.Sprintf("Claimed %q on %s.", in.Task, host)), nil
		})
		return nil
	}})
}
