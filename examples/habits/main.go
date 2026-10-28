// Command habits is an example plugin: check in on habits and see counts in wake.
//
//	one plugins link ./examples/habits
package main

import (
	"fmt"
	"sort"
	"strings"

	"github.com/drevantonder/the-one-app/sdk"
)

type checkIn struct {
	Habit string `json:"habit" jsonschema:"the habit, e.g. run"`
}

type noIn struct{}

func main() {
	sdk.Run(sdk.Plugin{ID: "habits", Version: "0.1.0", Setup: func(c *sdk.Ctx) error {
		counts := map[string]int{}
		c.Entry("checkIn", sdk.Available)
		c.OnReset(func() { counts = map[string]int{} })
		sdk.Handle(c, "checkIn", func(_ sdk.Entry, in checkIn) error {
			counts[in.Habit]++
			return nil
		})
		sdk.Op(c, "check_in", "Record that a habit was done.", sdk.Write, func(call *sdk.Call, in checkIn) (sdk.Result, error) {
			habit := strings.ToLower(strings.TrimSpace(in.Habit))
			if habit == "" {
				return sdk.Result{}, fmt.Errorf("habit is required")
			}
			if _, err := call.Append("checkIn", checkIn{Habit: habit}); err != nil {
				return sdk.Result{}, err
			}
			return sdk.Text(fmt.Sprintf("%s: %d check-ins.", habit, counts[habit])), nil
		})
		sdk.Op(c, "counts", "Show check-in counts per habit.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
			return sdk.Text(summary(counts)), nil
		})
		return nil
	}})
}

func summary(counts map[string]int) string {
	if len(counts) == 0 {
		return "No check-ins yet."
	}
	var names []string
	for h := range counts {
		names = append(names, h)
	}
	sort.Strings(names)
	var lines []string
	for _, h := range names {
		lines = append(lines, fmt.Sprintf("%s: %d", h, counts[h]))
	}
	return strings.Join(lines, "\n")
}
