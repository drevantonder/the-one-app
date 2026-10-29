// Package shell is the built-in plugin that runs shell commands on the
// device it's called on. Every command is logged, so all devices share an
// audit trail. Its operations, like every plugin's, take a device argument
// to run on another device.
package shell

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

const maxOutput = 20000

type runIn struct {
	Command string `json:"command" jsonschema:"the command, run by sh -c (cmd /C on Windows)"`
	Dir     string `json:"dir,omitempty" jsonschema:"working directory; default is the home directory"`
	Timeout int    `json:"timeoutSeconds,omitempty" jsonschema:"default 60, at most 600"`
}

type ran struct {
	Host    string `json:"host"`
	Command string `json:"command"`
	Exit    int    `json:"exit"`
	Ms      int64  `json:"ms"`
}

type historyIn struct {
	Limit int `json:"limit,omitempty" jsonschema:"how many recent commands; default 20"`
}

var Plugin = sdk.Plugin{ID: "shell", Name: "Shell", Version: "0.1.0", Setup: setup}

func setup(c *sdk.Ctx) error {
	var log []sdk.Entry
	var details []ran
	c.Entry("ran", sdk.Available)
	c.OnReset(func() { log, details = nil, nil })
	sdk.Handle(c, "ran", func(e sdk.Entry, in ran) error {
		log, details = append(log, e), append(details, in)
		return nil
	})

	sdk.Op(c, "run", "Run a shell command on this device and return its exit code and output. Every command is logged.", sdk.Write, func(call *sdk.Call, in runIn) (sdk.Result, error) {
		if strings.TrimSpace(in.Command) == "" {
			return sdk.Result{}, errors.New("command is required")
		}
		timeout := time.Duration(min(max(in.Timeout, 1), 600)) * time.Second
		if in.Timeout == 0 {
			timeout = time.Minute
		}
		ctx, cancel := context.WithTimeout(call.Context, timeout)
		defer cancel()
		cmd := exec.CommandContext(ctx, "sh", "-c", in.Command)
		if runtime.GOOS == "windows" {
			cmd = exec.CommandContext(ctx, "cmd", "/C", in.Command)
		}
		cmd.Dir = in.Dir
		if cmd.Dir == "" {
			cmd.Dir, _ = os.UserHomeDir()
		}
		var stdout, stderr bytes.Buffer
		cmd.Stdout, cmd.Stderr = &stdout, &stderr
		start := time.Now()
		err := cmd.Run()
		exit := 0
		var ee *exec.ExitError
		switch {
		case errors.As(err, &ee):
			exit = ee.ExitCode()
		case err != nil:
			exit = -1
			stderr.WriteString(err.Error())
		}
		if ctx.Err() == context.DeadlineExceeded {
			stderr.WriteString(fmt.Sprintf("\n(stopped after %s)", timeout))
		}
		host, _ := os.Hostname()
		if _, err := call.Append("ran", ran{Host: host, Command: in.Command, Exit: exit, Ms: time.Since(start).Milliseconds()}); err != nil {
			return sdk.Result{}, err
		}
		var b strings.Builder
		fmt.Fprintf(&b, "exit %d on %s", exit, host)
		if s := clip(stdout.String()); s != "" {
			b.WriteString("\n--- stdout\n" + s)
		}
		if s := clip(stderr.String()); s != "" {
			b.WriteString("\n--- stderr\n" + s)
		}
		return sdk.Text(b.String()), nil
	})

	sdk.Op(c, "history", "List recent commands run on any device, newest first.", sdk.Read, func(_ *sdk.Call, in historyIn) (sdk.Result, error) {
		limit := in.Limit
		if limit <= 0 {
			limit = 20
		}
		if len(log) == 0 {
			return sdk.Text("No commands run yet."), nil
		}
		var lines []string
		for i := len(log) - 1; i >= 0 && len(lines) < limit; i-- {
			d := details[i]
			lines = append(lines, fmt.Sprintf("%s  %-20s exit %-3d %s  (by %s)", log[i].Time, d.Host, d.Exit, d.Command, log[i].Actor))
		}
		return sdk.Text(strings.Join(lines, "\n")), nil
	})
	return nil
}

func clip(s string) string {
	s = strings.TrimRight(s, "\n")
	if len(s) > maxOutput {
		return s[:maxOutput] + fmt.Sprintf("\n… (%d more bytes cut)", len(s)-maxOutput)
	}
	return s
}
