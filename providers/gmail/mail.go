package main

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"html"
	"mime"
	"mime/quotedprintable"
	"regexp"
	"strconv"
	"strings"
)

// part is a message, or a part of one, as Gmail's API gives it (format=full).
type part struct {
	MimeType string `json:"mimeType"`
	Filename string `json:"filename"`
	Headers  []struct {
		Name  string `json:"name"`
		Value string `json:"value"`
	} `json:"headers"`
	Body struct {
		Size         int    `json:"size"`
		Data         string `json:"data"`
		AttachmentID string `json:"attachmentId"`
	} `json:"body"`
	Parts []part `json:"parts"`
}

func (p part) header(name string) string {
	for _, h := range p.Headers {
		if strings.EqualFold(h.Name, name) {
			return h.Value
		}
	}
	return ""
}

type attachment struct {
	Name, Type, ID string
	Size           int
}

// body is a message's text: its first plain-text part, else its first HTML
// part made plain. Parts with a filename are attachments, not text.
func body(p part) string {
	var plain, htm string
	var walk func(part)
	walk = func(p part) {
		if p.Filename != "" {
			return
		}
		switch {
		case strings.HasPrefix(p.MimeType, "multipart/"):
			for _, c := range p.Parts {
				walk(c)
			}
		case p.MimeType == "text/plain" && plain == "":
			plain = decode(p.Body.Data)
		case p.MimeType == "text/html" && htm == "":
			htm = decode(p.Body.Data)
		}
	}
	walk(p)
	if strings.TrimSpace(plain) != "" {
		plain = filler.ReplaceAllString(strings.ReplaceAll(plain, "\r\n", "\n"), "")
		return strings.TrimSpace(trailing.ReplaceAllString(plain, "\n"))
	}
	return text(htm)
}

func attachments(p part) []attachment {
	var out []attachment
	if p.Filename != "" {
		out = append(out, attachment{p.Filename, p.MimeType, p.Body.AttachmentID, p.Body.Size})
	}
	for _, c := range p.Parts {
		out = append(out, attachments(c)...)
	}
	return out
}

func decode(data string) string {
	b, err := base64.URLEncoding.DecodeString(data)
	if err != nil {
		b, _ = base64.RawURLEncoding.DecodeString(data)
	}
	return string(b)
}

var (
	filler    = regexp.MustCompile(`[\x{200b}-\x{200d}\x{2060}\x{feff}\x{034f}\x{00ad}]`) // invisible padding marketing emails use
	trailing  = regexp.MustCompile(`[ \t\x{a0}]+\n`)
	invisible = regexp.MustCompile(`(?is)<(head|style|script|title)\b.*?</(head|style|script|title)\s*>`)
	breaks    = regexp.MustCompile(`(?i)<br\s*/?>|</(p|div|tr|h[1-6]|table|ul|ol|blockquote)\s*>`)
	items     = regexp.MustCompile(`(?i)<li\b[^>]*>`)
	tags      = regexp.MustCompile(`(?s)<[^>]*>`)
	spaces    = regexp.MustCompile(`[ \t\x{a0}]+`)
	blanks    = regexp.MustCompile(`\n\s*\n\s*\n+`)
)

// text makes HTML readable as plain text: no tags, a line per block.
func text(h string) string {
	h = invisible.ReplaceAllString(h, "")
	h = breaks.ReplaceAllString(h, "\n")
	h = items.ReplaceAllString(h, "\n- ")
	h = tags.ReplaceAllString(h, "")
	h = filler.ReplaceAllString(html.UnescapeString(h), "")
	h = spaces.ReplaceAllString(h, " ")
	lines := strings.Split(h, "\n")
	for i, l := range lines {
		lines[i] = strings.TrimSpace(l)
	}
	return strings.TrimSpace(blanks.ReplaceAllString(strings.Join(lines, "\n"), "\n\n"))
}

// draft is a plain-text email to compose.
type draft struct {
	To, Cc, Bcc, Subject, Body string
	InReplyTo, References      string // Message-IDs, when replying
	Files                      []attached
}

// attached is a file sent with an email.
type attached struct {
	Name, Type string
	Data       []byte
}

// compose writes d as an RFC 5322 message, base64url-encoded as Gmail wants
// it. Gmail adds From, Date and Message-ID.
func compose(d draft) string {
	return base64.URLEncoding.EncodeToString(raw(d))
}

// raw writes d as an RFC 5322 message: plain text, or with files, multipart/mixed with the text first.
func raw(d draft) []byte {
	var b bytes.Buffer
	line := strings.NewReplacer("\r", "", "\n", "")
	for _, h := range [][2]string{{"To", d.To}, {"Cc", d.Cc}, {"Bcc", d.Bcc},
		{"Subject", mime.QEncoding.Encode("utf-8", line.Replace(d.Subject))},
		{"In-Reply-To", d.InReplyTo}, {"References", d.References}} {
		if h[1] != "" {
			fmt.Fprintf(&b, "%s: %s\r\n", h[0], line.Replace(h[1]))
		}
	}
	b.WriteString("MIME-Version: 1.0\r\n")
	text := func() {
		b.WriteString("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n")
		w := quotedprintable.NewWriter(&b)
		w.Write([]byte(strings.ReplaceAll(strings.ReplaceAll(d.Body, "\r\n", "\n"), "\n", "\r\n")))
		w.Close()
	}
	if len(d.Files) == 0 {
		text()
		return b.Bytes()
	}
	boundary := "one-" + strconv.FormatInt(int64(len(d.Body))*7919+int64(len(d.Files)), 36) + "-mixed"
	fmt.Fprintf(&b, "Content-Type: multipart/mixed; boundary=%q\r\n\r\n--%s\r\n", boundary, boundary)
	text()
	for _, f := range d.Files {
		name := mime.QEncoding.Encode("utf-8", line.Replace(f.Name))
		fmt.Fprintf(&b, "\r\n--%s\r\nContent-Type: %s; name=\"%s\"\r\nContent-Disposition: attachment; filename=\"%s\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
			boundary, f.Type, name, name)
		enc := base64.StdEncoding.EncodeToString(f.Data)
		for len(enc) > 76 {
			b.WriteString(enc[:76] + "\r\n")
			enc = enc[76:]
		}
		b.WriteString(enc + "\r\n")
	}
	fmt.Fprintf(&b, "--%s--\r\n", boundary)
	return b.Bytes()
}

// snippet is Gmail's preview of a message, made plain and on one line.
func snippet(s string) string {
	s = filler.ReplaceAllString(html.UnescapeString(s), "")
	return strings.Join(strings.Fields(s), " ")
}
