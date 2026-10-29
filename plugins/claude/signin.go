package claude

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"regexp"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// A login is a `claude auth login` waiting for the code its sign-in page
// shows.
type login struct {
	cmd      *exec.Cmd
	stdin    io.WriteCloser
	out      *buffer
	done     chan struct{}
	err      error
	previous string // the account claude-swap was on, to go back to
	cancel   context.CancelFunc
}

type buffer struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (b *buffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.b.Write(p)
}

func (b *buffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.b.String()
}

var authURL = regexp.MustCompile(`https://[^\s\x07\x1b]*oauth/authorize\?[^\s\x07\x1b]+`)

const signInFor = 15 * time.Minute

func (p *plugin) start(email string) (string, error) {
	if _, err := which("claude-swap"); err != nil {
		return "", errNoSwap
	}
	ctx, cancel := context.WithTimeout(context.Background(), signInFor)
	if err := adopt(ctx); err != nil {
		cancel()
		return "", err
	}
	args := []string{"auth", "login", "--claudeai"}
	if email != "" {
		args = append(args, "--email", email)
	}
	l := &login{out: &buffer{}, done: make(chan struct{}), previous: active(ctx), cancel: cancel}
	l.cmd = command(ctx, "claude", args...)
	l.cmd.Stdout, l.cmd.Stderr = l.out, l.out
	var err error
	if l.stdin, err = l.cmd.StdinPipe(); err != nil {
		cancel()
		return "", err
	}
	if err := l.cmd.Start(); err != nil {
		cancel()
		return "", err
	}
	go func() {
		l.err = l.cmd.Wait()
		close(l.done)
	}()
	for wait := time.After(30 * time.Second); ; {
		if url := authURL.FindString(l.out.String()); url != "" {
			p.mu.Lock()
			if p.login != nil {
				p.login.cancel()
			}
			p.login = l
			p.mu.Unlock()
			return url, nil
		}
		select {
		case <-l.done:
			cancel()
			return "", fmt.Errorf("claude auth login stopped without a sign-in link: %s", tail(l.out.String()))
		case <-wait:
			cancel()
			return "", fmt.Errorf("claude auth login gave no sign-in link: %s", tail(l.out.String()))
		case <-time.After(100 * time.Millisecond):
		}
	}
}

func (p *plugin) finish(ctx context.Context, code string) (string, error) {
	p.mu.Lock()
	l := p.login
	p.login = nil
	p.mu.Unlock()
	if l == nil {
		return "", errors.New("no sign-in is waiting here: start one with sign_in")
	}
	select {
	case <-l.done:
	default:
		if strings.TrimSpace(code) == "" {
			p.keep(l)
			return "", errors.New("give the code the sign-in page shows")
		}
	}
	defer l.cancel()
	select {
	case <-l.done:
	default:
		if _, err := io.WriteString(l.stdin, strings.TrimSpace(code)+"\n"); err != nil {
			return "", err
		}
		select {
		case <-l.done:
		case <-time.After(time.Minute):
			return "", fmt.Errorf("claude auth login didn't finish: %s", tail(l.out.String()))
		}
	}
	if l.err != nil {
		return "", fmt.Errorf("claude auth login: %v: %s", l.err, tail(l.out.String()))
	}
	email := signedIn(ctx)
	if email == "" {
		return "", fmt.Errorf("claude auth login finished but Claude Code isn't signed in: %s", tail(l.out.String()))
	}
	if _, err := run(ctx, "claude-swap", "add"); err != nil {
		return "", err
	}
	if l.previous != "" && l.previous != email {
		if _, err := run(ctx, "claude-swap", "switch", l.previous); err != nil {
			return email, fmt.Errorf("signed in to %s, but couldn't switch back to %s: %w", email, l.previous, err)
		}
	}
	return email, nil
}

// keep puts a login back as the one waiting, unless another started since.
func (p *plugin) keep(l *login) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.login == nil {
		p.login = l
	} else {
		l.cancel()
	}
}

// signIn starts signing in on each device asked for and gives the links.
func (p *plugin) signIn(call *sdk.Call, in signInIn) (sdk.Result, error) {
	devices, err := p.all(call)
	if err != nil {
		return sdk.Result{}, err
	}
	var targets []device
	for _, d := range devices {
		if len(in.Devices) > 0 && !slices.Contains(in.Devices, d.Name) || len(in.Devices) == 0 && !d.Coding {
			continue
		}
		if in.Email != "" && slices.ContainsFunc(d.Accounts, func(a account) bool { return strings.EqualFold(a.Email, in.Email) }) {
			continue
		}
		targets = append(targets, d)
	}
	if len(targets) == 0 {
		switch {
		case len(in.Devices) > 0 || slices.ContainsFunc(devices, func(d device) bool { return d.Coding }):
			return sdk.Text("Every device asked for is already signed in to " + in.Email + "."), nil
		default:
			return sdk.Result{}, errors.New("no coding devices: choose them with coding")
		}
	}
	links := make([]string, len(targets))
	var wg sync.WaitGroup
	for i, d := range targets {
		wg.Go(func() {
			var url string
			var err error
			switch {
			case !d.Online:
				err = errors.New("offline")
			case d.Name == p.me(call):
				url, err = p.start(in.Email)
			default:
				var res sdk.Result
				res, err = call.Invoke("claude", "start_sign_in", map[string]any{"device": d.Name, "email": in.Email})
				url = strings.TrimSpace(res.Text)
			}
			if err != nil {
				links[i] = fmt.Sprintf("%s: couldn't start: %v", d.Name, err)
				return
			}
			links[i] = d.Name + ": " + url
		})
	}
	wg.Wait()
	who := "your Claude account"
	if in.Email != "" {
		who = in.Email
	}
	return sdk.Text(fmt.Sprintf("Open each link and sign in as %s, then give the code the page shows to finish_sign_in on that device. The links work for %d minutes.\n\n%s", who, int(signInFor.Minutes()), strings.Join(links, "\n"))), nil
}
