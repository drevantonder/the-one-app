package board

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

const newsMax = 20 // posts one piece of news prints

// News prints the posts made after since, and the time of the latest.
func (b *Board) News(since time.Time) (string, time.Time) {
	var lines []string
	latest := since
	for _, r := range b.posts {
		if !r.at.After(since) {
			continue
		}
		lines = append(lines, show(r.topic.path)+" "+r.topic.log.Line(r.i))
		if r.at.After(latest) {
			latest = r.at
		}
	}
	if len(lines) == 0 {
		return "", latest
	}
	head := "New on the Agent Board since you last looked:"
	if n := len(lines) - newsMax; n > 0 {
		lines = lines[n:]
		head = fmt.Sprintf("New on the Agent Board since you last looked (%d older ones not shown; board_read their topics):", n)
	}
	return head + "\n" + strings.Join(lines, "\n"), latest
}

// Latest is the time of the newest post.
func (b *Board) Latest() time.Time {
	var t time.Time
	for _, r := range b.posts {
		if r.at.After(t) {
			t = r.at
		}
	}
	return t
}

// sessions remembers, on this device, the newest post each agent session
// has been shown. It is not in the log: sessions live on one device, and
// forgetting one only means its agent is told nothing until the next post.
type sessions struct {
	mu   sync.Mutex
	file string
	seen map[string]time.Time
}

// loadSessions reads what dir remembers; with no dir, nothing is kept.
func loadSessions(dir string) *sessions {
	s := &sessions{seen: map[string]time.Time{}}
	if dir == "" {
		return s
	}
	s.file = filepath.Join(dir, "sessions.json")
	if b, err := os.ReadFile(s.file); err == nil {
		_ = json.Unmarshal(b, &s.seen)
	}
	return s
}

// news is what session hasn't seen yet. A session's first call only marks
// the board's newest post, since agents read the board when they start.
func (s *sessions) news(b *Board, session string) (text string, first bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	since, ok := s.seen[session]
	latest := b.Latest()
	if ok {
		text, latest = b.News(since)
	}
	if !ok || latest.After(since) {
		s.seen[session] = latest
		s.save()
	}
	return text, !ok
}

func (s *sessions) save() {
	old := time.Now().Add(-30 * 24 * time.Hour)
	for k, t := range s.seen {
		if t.Before(old) {
			delete(s.seen, k)
		}
	}
	if s.file == "" {
		return
	}
	if b, err := json.Marshal(s.seen); err == nil {
		_ = os.WriteFile(s.file, b, 0o600)
	}
}
