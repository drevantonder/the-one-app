package main

import (
	"bytes"
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

// longest is how much of a body read and thread give; the rest is cut.
const longest = 20000

type gmail struct{ accounts *accounts }

type input struct {
	Account, Query, Page, ID   word
	ReplyTo                    word `json:"reply_to"`
	To, Cc, Bcc, Subject, Body string
	Code                       string
	ClientID                   string `json:"client_id"`
	ClientSecret               string `json:"client_secret"`
	IDs, Add, Remove           words
	Limit                      int
}

// word is a string the CLI may have read as a number (an id of only digits).
type word string

func (w *word) UnmarshalJSON(b []byte) error {
	var s string
	if json.Unmarshal(b, &s) == nil {
		*w = word(s)
		return nil
	}
	*w = word(strings.TrimSpace(string(b)))
	return nil
}

// words is a list, given as an array or as one comma-separated string.
type words []string

func (ws *words) UnmarshalJSON(b []byte) error {
	var list []word
	if json.Unmarshal(b, &list) != nil {
		var one word
		if err := json.Unmarshal(b, &one); err != nil {
			return err
		}
		list = []word{one}
	}
	*ws = nil
	for _, w := range list {
		for _, s := range strings.Split(string(w), ",") {
			if s = strings.TrimSpace(s); s != "" {
				*ws = append(*ws, s)
			}
		}
	}
	return nil
}

func (g *gmail) run(ctx context.Context, op string, raw json.RawMessage) (any, error) {
	var in input
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, err
		}
	}
	switch op {
	case "accounts":
		return g.list(ctx)
	case "sign_in":
		return g.accounts.signIn(ctx, in.ClientID, in.ClientSecret, in.Code)
	case "sign_out":
		return g.accounts.signOut(string(in.Account))
	}
	acc, err := g.accounts.resolve(string(in.Account))
	if err != nil {
		return nil, err
	}
	c := client{g.accounts, acc}
	if in.ID != "" {
		in.IDs = append(words{string(in.ID)}, in.IDs...)
	}
	need := func(what string, v string) error {
		if strings.TrimSpace(v) == "" {
			return fmt.Errorf("give %s", what)
		}
		return nil
	}
	switch op {
	case "profile":
		return c.profile(ctx)
	case "search":
		return c.search(ctx, string(in.Query), string(in.Page), min(cmp.Or(in.Limit, 20), 100))
	case "read":
		if err := need("id", string(in.ID)); err != nil {
			return nil, err
		}
		return c.read(ctx, string(in.ID))
	case "thread":
		if err := need("id", string(in.ID)); err != nil {
			return nil, err
		}
		return c.thread(ctx, string(in.ID))
	case "labels":
		return c.labelList(ctx)
	case "label":
		if len(in.IDs) == 0 || len(in.Add)+len(in.Remove) == 0 {
			return nil, errors.New("give ids, and add or remove")
		}
		return c.label(ctx, in.IDs, in.Add, in.Remove)
	case "trash":
		if len(in.IDs) == 0 {
			return nil, errors.New("give ids")
		}
		return c.trash(ctx, in.IDs)
	case "drafts":
		return c.drafts(ctx, string(in.Page), min(cmp.Or(in.Limit, 20), 100))
	case "draft", "send":
		d, thread, err := c.compose(ctx, in)
		if err != nil {
			return nil, err
		}
		if op == "send" {
			return c.send(ctx, d, thread)
		}
		return c.saveDraft(ctx, string(in.ID), d, thread)
	case "send_draft":
		if err := need("id", string(in.ID)); err != nil {
			return nil, err
		}
		var sent struct{ ID string }
		if err := c.call(ctx, "POST", "drafts/send", nil, map[string]string{"id": string(in.ID)}, &sent); err != nil {
			return nil, err
		}
		return "Sent, as message " + sent.ID + ".", nil
	case "delete_draft":
		if err := need("id", string(in.ID)); err != nil {
			return nil, err
		}
		if err := c.call(ctx, "DELETE", "drafts/"+url.PathEscape(string(in.ID)), nil, nil, nil); err != nil {
			return nil, err
		}
		return "Deleted the draft.", nil
	}
	return nil, fmt.Errorf("gmail has no operation %s", op)
}

func (g *gmail) list(ctx context.Context) (string, error) {
	g.accounts.mu.Lock()
	all := slices.Clone(g.accounts.creds.Accounts)
	g.accounts.mu.Unlock()
	if len(all) == 0 {
		return "No account is signed in here: gmail_sign_in signs one in.", nil
	}
	var lines []string
	for i, acc := range all {
		line := acc.Email + ": signed in"
		if _, err := g.accounts.token(ctx, acc); err != nil {
			line = err.Error()
		}
		if i == 0 {
			line += " (the default)"
		}
		lines = append(lines, line)
	}
	return strings.Join(lines, "\n"), nil
}

// client calls Gmail's API as one account.
type client struct {
	accounts *accounts
	acc      account
}

func (c client) call(ctx context.Context, method, path string, q url.Values, in, out any) error {
	tok, err := c.accounts.token(ctx, c.acc)
	if err != nil {
		return err
	}
	var body io.Reader
	if in != nil {
		b, err := json.Marshal(in)
		if err != nil {
			return err
		}
		body = bytes.NewReader(b)
	}
	u := "https://gmail.googleapis.com/gmail/v1/users/me/" + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	req, err := http.NewRequestWithContext(ctx, method, u, body)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+tok)
	if in != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	if resp.StatusCode >= 300 {
		var e struct {
			Error struct{ Message string }
		}
		_ = json.Unmarshal(b, &e)
		return fmt.Errorf("Gmail: %s", cmp.Or(e.Error.Message, resp.Status))
	}
	if out == nil || len(b) == 0 {
		return nil
	}
	return json.Unmarshal(b, out)
}

type message struct {
	ID           string   `json:"id"`
	ThreadID     string   `json:"threadId"`
	LabelIDs     []string `json:"labelIds"`
	Snippet      string   `json:"snippet"`
	InternalDate string   `json:"internalDate"`
	Payload      part     `json:"payload"`
}

func (m message) when() string {
	ms, _ := strconv.ParseInt(m.InternalDate, 10, 64)
	t := time.UnixMilli(ms).Local()
	if t.Year() == time.Now().Year() {
		return t.Format("Mon 2 Jan 15:04")
	}
	return t.Format("2 Jan 2006 15:04")
}

func (c client) profile(ctx context.Context) (string, error) {
	var p struct {
		EmailAddress  string
		MessagesTotal int
		ThreadsTotal  int
	}
	if err := c.call(ctx, "GET", "profile", nil, nil, &p); err != nil {
		return "", err
	}
	return fmt.Sprintf("%s: %d messages in %d threads.", p.EmailAddress, p.MessagesTotal, p.ThreadsTotal), nil
}

// labelNames maps label ids to names, for showing user labels.
func (c client) labelNames(ctx context.Context) (map[string]string, error) {
	var r struct {
		Labels []struct{ ID, Name, Type string }
	}
	if err := c.call(ctx, "GET", "labels", nil, nil, &r); err != nil {
		return nil, err
	}
	names := map[string]string{}
	for _, l := range r.Labels {
		names[l.ID] = l.Name
	}
	return names, nil
}

func (c client) labelList(ctx context.Context) (string, error) {
	var r struct {
		Labels []struct{ ID, Name, Type string }
	}
	if err := c.call(ctx, "GET", "labels", nil, nil, &r); err != nil {
		return "", err
	}
	slices.SortFunc(r.Labels, func(a, b struct{ ID, Name, Type string }) int {
		return cmp.Or(cmp.Compare(a.Type, b.Type), cmp.Compare(strings.ToLower(a.Name), strings.ToLower(b.Name)))
	})
	var lines []string
	for _, l := range r.Labels {
		line := l.Name
		if l.ID != l.Name {
			line += "  (id " + l.ID + ")"
		}
		lines = append(lines, line)
	}
	return strings.Join(lines, "\n"), nil
}

// labels names a message's labels, leaving out Gmail's categories.
func labels(ids []string, names map[string]string) string {
	var out []string
	for _, id := range ids {
		if !strings.HasPrefix(id, "CATEGORY_") {
			out = append(out, cmp.Or(names[id], id))
		}
	}
	return strings.Join(out, ", ")
}

func (c client) search(ctx context.Context, query, page string, limit int) (string, error) {
	var r struct {
		Messages      []message
		NextPageToken string
	}
	q := url.Values{"maxResults": {strconv.Itoa(limit)}}
	if query != "" {
		q.Set("q", query)
	}
	if page != "" {
		q.Set("pageToken", page)
	}
	if err := c.call(ctx, "GET", "messages", q, nil, &r); err != nil {
		return "", err
	}
	if len(r.Messages) == 0 {
		return "Nothing found.", nil
	}
	names, err := c.labelNames(ctx)
	if err != nil {
		return "", err
	}
	ms, err := c.metadata(ctx, r.Messages)
	if err != nil {
		return "", err
	}
	var b strings.Builder
	for _, m := range ms {
		fmt.Fprintf(&b, "%s  %s  %s\n  %s\n", m.ID, m.when(), m.Payload.header("From"), m.Payload.header("Subject"))
		if l := labels(m.LabelIDs, names); l != "" {
			fmt.Fprintf(&b, "  [%s]\n", l)
		}
		if m.Snippet != "" {
			fmt.Fprintf(&b, "  %s\n", snippet(m.Snippet))
		}
	}
	if r.NextPageToken != "" {
		fmt.Fprintf(&b, "\nMore: give page %s", r.NextPageToken)
	}
	return strings.TrimSpace(b.String()), nil
}

// metadata fetches each message's headers, a few at a time, in order.
func (c client) metadata(ctx context.Context, ms []message) ([]message, error) {
	out := make([]message, len(ms))
	errs := make([]error, len(ms))
	sem := make(chan struct{}, 8)
	var wg sync.WaitGroup
	for i, m := range ms {
		wg.Go(func() {
			sem <- struct{}{}
			defer func() { <-sem }()
			q := url.Values{"format": {"metadata"}, "metadataHeaders": {"From", "To", "Subject", "Date"}}
			errs[i] = c.call(ctx, "GET", "messages/"+url.PathEscape(m.ID), q, nil, &out[i])
		})
	}
	wg.Wait()
	return out, errors.Join(errs...)
}

func (c client) read(ctx context.Context, id string) (string, error) {
	var m message
	if err := c.call(ctx, "GET", "messages/"+url.PathEscape(id), url.Values{"format": {"full"}}, nil, &m); err != nil {
		return "", err
	}
	names, err := c.labelNames(ctx)
	if err != nil {
		return "", err
	}
	return show(m, names), nil
}

func (c client) thread(ctx context.Context, id string) (string, error) {
	var t struct{ Messages []message }
	if err := c.call(ctx, "GET", "threads/"+url.PathEscape(id), url.Values{"format": {"full"}}, nil, &t); err != nil {
		var m message // or a message's id, whose thread this is
		if c.call(ctx, "GET", "messages/"+url.PathEscape(id), url.Values{"format": {"minimal"}}, nil, &m) != nil || m.ThreadID == id {
			return "", err
		}
		return c.thread(ctx, m.ThreadID)
	}
	names, err := c.labelNames(ctx)
	if err != nil {
		return "", err
	}
	var parts []string
	for _, m := range t.Messages {
		parts = append(parts, show(m, names))
	}
	return strings.Join(parts, "\n\n----------\n\n"), nil
}

// show is a message as text: its headers, labels, attachments, then body.
func show(m message, names map[string]string) string {
	var b strings.Builder
	for _, h := range []string{"Subject", "From", "To", "Cc", "Reply-To"} {
		if v := m.Payload.header(h); v != "" {
			fmt.Fprintf(&b, "%s: %s\n", h, v)
		}
	}
	fmt.Fprintf(&b, "Date: %s\nId: %s (thread %s)\n", m.when(), m.ID, m.ThreadID)
	if l := labels(m.LabelIDs, names); l != "" {
		fmt.Fprintf(&b, "Labels: %s\n", l)
	}
	for _, a := range attachments(m.Payload) {
		fmt.Fprintf(&b, "Attachment: %s (%s, %s)\n", a.Name, a.Type, size(a.Size))
	}
	text := body(m.Payload)
	if len(text) > longest {
		text = text[:longest] + "\n\n[cut: the message goes on]"
	}
	return b.String() + "\n" + text
}

func size(n int) string {
	switch {
	case n >= 1<<20:
		return fmt.Sprintf("%.1f MB", float64(n)/(1<<20))
	case n >= 1<<10:
		return fmt.Sprintf("%d KB", n>>10)
	}
	return fmt.Sprintf("%d bytes", n)
}

func (c client) label(ctx context.Context, ids, add, remove []string) (string, error) {
	var r struct {
		Labels []struct{ ID, Name string }
	}
	if err := c.call(ctx, "GET", "labels", nil, nil, &r); err != nil {
		return "", err
	}
	resolve := func(given []string) ([]string, error) {
		var out []string
		for _, g := range given {
			i := slices.IndexFunc(r.Labels, func(l struct{ ID, Name string }) bool {
				return l.ID == g || strings.EqualFold(l.Name, g)
			})
			if i < 0 {
				return nil, fmt.Errorf("no label %q: gmail_labels lists them", g)
			}
			out = append(out, r.Labels[i].ID)
		}
		return out, nil
	}
	addIDs, err := resolve(add)
	if err != nil {
		return "", err
	}
	removeIDs, err := resolve(remove)
	if err != nil {
		return "", err
	}
	body := map[string]any{"ids": ids, "addLabelIds": addIDs, "removeLabelIds": removeIDs}
	if err := c.call(ctx, "POST", "messages/batchModify", nil, body, nil); err != nil {
		return "", err
	}
	return fmt.Sprintf("Changed the labels of %d messages.", len(ids)), nil
}

func (c client) trash(ctx context.Context, ids []string) (string, error) {
	for i, id := range ids {
		if err := c.call(ctx, "POST", "messages/"+url.PathEscape(id)+"/trash", nil, nil, nil); err != nil {
			return "", fmt.Errorf("trashed %d of %d, then: %w", i, len(ids), err)
		}
	}
	return fmt.Sprintf("Moved %d messages to the trash.", len(ids)), nil
}

func (c client) drafts(ctx context.Context, page string, limit int) (string, error) {
	var r struct {
		Drafts []struct {
			ID      string
			Message message
		}
		NextPageToken string
	}
	q := url.Values{"maxResults": {strconv.Itoa(limit)}}
	if page != "" {
		q.Set("pageToken", page)
	}
	if err := c.call(ctx, "GET", "drafts", q, nil, &r); err != nil {
		return "", err
	}
	if len(r.Drafts) == 0 {
		return "No drafts.", nil
	}
	var ms []message
	for _, d := range r.Drafts {
		ms = append(ms, d.Message)
	}
	ms, err := c.metadata(ctx, ms)
	if err != nil {
		return "", err
	}
	var b strings.Builder
	for i, m := range ms {
		fmt.Fprintf(&b, "%s  %s  to %s\n  %s\n", r.Drafts[i].ID, m.when(), cmp.Or(m.Payload.header("To"), "nobody yet"), cmp.Or(m.Payload.header("Subject"), "(no subject)"))
	}
	if r.NextPageToken != "" {
		fmt.Fprintf(&b, "\nMore: give page %s", r.NextPageToken)
	}
	return strings.TrimSpace(b.String()), nil
}

// compose makes the email in, threaded under reply_to when it's a reply:
// to its sender, with its subject, unless in says otherwise.
func (c client) compose(ctx context.Context, in input) (draft, string, error) {
	d := draft{To: in.To, Cc: in.Cc, Bcc: in.Bcc, Subject: in.Subject, Body: in.Body}
	thread := ""
	if in.ReplyTo != "" {
		var m message
		q := url.Values{"format": {"metadata"}, "metadataHeaders": {"From", "Reply-To", "Subject", "Message-ID", "References"}}
		if err := c.call(ctx, "GET", "messages/"+url.PathEscape(string(in.ReplyTo)), q, nil, &m); err != nil {
			return d, "", err
		}
		thread = m.ThreadID
		id := m.Payload.header("Message-ID")
		d.InReplyTo, d.References = id, strings.TrimSpace(m.Payload.header("References")+" "+id)
		d.To = cmp.Or(d.To, m.Payload.header("Reply-To"), m.Payload.header("From"))
		if d.Subject == "" {
			d.Subject = m.Payload.header("Subject")
			if !strings.HasPrefix(strings.ToLower(d.Subject), "re:") {
				d.Subject = "Re: " + d.Subject
			}
		}
	}
	return d, thread, nil
}

func (c client) send(ctx context.Context, d draft, thread string) (string, error) {
	if d.To == "" && d.Cc == "" && d.Bcc == "" {
		return "", errors.New("give to")
	}
	var sent struct{ ID string }
	if err := c.call(ctx, "POST", "messages/send", nil, outgoing(d, thread), &sent); err != nil {
		return "", err
	}
	return "Sent, as message " + sent.ID + ".", nil
}

func (c client) saveDraft(ctx context.Context, id string, d draft, thread string) (string, error) {
	body := map[string]any{"message": outgoing(d, thread)}
	var saved struct{ ID string }
	if id == "" {
		if err := c.call(ctx, "POST", "drafts", nil, body, &saved); err != nil {
			return "", err
		}
		return "Saved draft " + saved.ID + ".", nil
	}
	if err := c.call(ctx, "PUT", "drafts/"+url.PathEscape(id), nil, body, &saved); err != nil {
		return "", err
	}
	return "Updated draft " + saved.ID + ".", nil
}

// outgoing is the raw email Gmail wants, in thread when it's a reply.
func outgoing(d draft, thread string) map[string]string {
	m := map[string]string{"raw": compose(d)}
	if thread != "" {
		m["threadId"] = thread
	}
	return m
}
