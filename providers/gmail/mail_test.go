package main

import (
	"encoding/base64"
	"encoding/json"
	"io"
	"mime"
	"mime/quotedprintable"
	"net/mail"
	"strings"
	"testing"
)

func data(s string) string { return base64.URLEncoding.EncodeToString([]byte(s)) }

func leaf(mimeType, filename, body string) part {
	var p part
	p.MimeType, p.Filename, p.Body.Data, p.Body.Size = mimeType, filename, data(body), len(body)
	return p
}

func multi(mimeType string, parts ...part) part {
	return part{MimeType: mimeType, Parts: parts}
}

func TestBodyPrefersPlainText(t *testing.T) {
	m := multi("multipart/mixed",
		multi("multipart/alternative", leaf("text/plain", "", "Hello,\r\nsee you there.\r\n"), leaf("text/html", "", "<p>Hello,</p>")),
		leaf("text/plain", "notes.txt", "not the body"),
		leaf("application/pdf", "ticket.pdf", "%PDF"))
	if got, want := body(m), "Hello,\nsee you there."; got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
	as := attachments(m)
	if len(as) != 2 || as[0].Name != "notes.txt" || as[1].Name != "ticket.pdf" || as[1].Type != "application/pdf" {
		t.Fatalf("attachments: %+v", as)
	}
}

func TestBodyFromHTML(t *testing.T) {
	h := `<html><head><style>p{color:red}</style><title>x</title></head><body>
		<div>Hi&nbsp;Andre,</div><p>Your <b>order</b> &amp; receipt:</p>
		<ul><li>one</li><li>two</li></ul><br><script>track()</script>Thanks</body></html>`
	got := body(multi("multipart/alternative", leaf("text/plain", "", "  \r\n"), leaf("text/html", "", h)))
	want := "Hi Andre,\nYour order & receipt:\n\n- one\n- two\n\nThanks"
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestBodyDropsInvisiblePadding(t *testing.T) {
	if got := body(leaf("text/plain", "", "Keep going! \u200c \u200c \u200c\r\nChess\u200c.com")); got != "Keep going!\nChess.com" {
		t.Fatalf("got %q", got)
	}
	if got := snippet("Keep going! &amp; \u200c \u200c \u034f done"); got != "Keep going! & done" {
		t.Fatalf("got %q", got)
	}
}

func TestBodyOfSinglePart(t *testing.T) {
	if got := body(leaf("text/plain", "", "just text")); got != "just text" {
		t.Fatalf("got %q", got)
	}
}

func TestComposeIsAReadableEmail(t *testing.T) {
	raw := compose(draft{To: "a@example.com", Cc: "b@example.com", Subject: "Café plans\r\nBcc: evil@example.com",
		Body: "Line one\nLine two, with a long tail " + strings.Repeat("x", 100), InReplyTo: "<1@x>", References: "<0@x> <1@x>"})
	b, err := base64.URLEncoding.DecodeString(raw)
	if err != nil {
		t.Fatal(err)
	}
	m, err := mail.ReadMessage(strings.NewReader(string(b)))
	if err != nil {
		t.Fatal(err)
	}
	subject, err := new(mimeDecoder).DecodeHeader(m.Header.Get("Subject"))
	if err != nil {
		t.Fatal(err)
	}
	if subject != "Café plansBcc: evil@example.com" || m.Header.Get("Bcc") != "" {
		t.Fatalf("subject %q, bcc %q", subject, m.Header.Get("Bcc"))
	}
	if m.Header.Get("To") != "a@example.com" || m.Header.Get("In-Reply-To") != "<1@x>" {
		t.Fatalf("headers: %v", m.Header)
	}
	text, _ := io.ReadAll(qpReader(m.Body))
	if want := "Line one\r\nLine two, with a long tail " + strings.Repeat("x", 100); string(text) != want {
		t.Fatalf("body %q", text)
	}
}

func TestInputTakesListsAndNumbers(t *testing.T) {
	var in input
	if err := json.Unmarshal([]byte(`{"id": 123, "ids": "a, b", "add": ["STARRED"], "reply_to": "r1", "client_id": "c"}`), &in); err != nil {
		t.Fatal(err)
	}
	if in.ID != "123" || len(in.IDs) != 2 || in.IDs[1] != "b" || in.Add[0] != "STARRED" || in.ReplyTo != "r1" || in.ClientID != "c" {
		t.Fatalf("%+v", in)
	}
}

type mimeDecoder = mime.WordDecoder

func qpReader(r io.Reader) io.Reader { return quotedprintable.NewReader(r) }
