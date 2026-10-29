// Package optmem is OptMem's memory model (github.com/VictorTaelin/OptMem),
// rewritten for any plugin: an append-only log of one-line entries, and a
// binary tree of one-line summaries over aligned power-of-two blocks,
// written by the agent itself.
//
// One change for the mesh: entries from several devices merge into one
// order, and a late arrival can shift positions. So each summary records a
// digest of exactly which entries it covers, and only counts while that
// digest still matches. Stale summaries are ignored and simply asked for
// again, so the tree heals itself after any merge.
package optmem

import (
	"fmt"
	"hash/fnv"
	"iter"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

const (
	WakeLines  = 96    // lines wake prints by default (~8k tokens)
	EntryChars = 280   // longest entry or summary, in bytes
	PartChars  = 20000 // output paging: largest part, in bytes
	PartLines  = 500   // output paging: largest part, in lines
	RawMax     = 16    // blocks up to this size are compressed from raw entries
	hashBase   = 1099511628211
)

// Words is how a log names itself and its operations in what it prints.
type Words struct {
	One, Many string                    // an entry and entries: "memory", "memories"
	Log       string                    // the whole log: "the memory"
	Write     string                    // the verb for adding an entry: "note"
	Title     string                    // heads each part of a paged wake: "Your memory"
	Empty     string                    // wake's text for an empty log
	Again     string                    // how to wake again: "call memory_wake again"
	Nap       func(block string) string // how to answer a compression
	Next      func(part, at int) string // how to read the next part of a wake
}

type entry struct {
	ID   string
	Date string
	Text string
}

type summary struct {
	Digest string
	Text   string
}

type Block struct{ Lo, Hi int } // [Lo, Hi)

func (b Block) String() string { return fmt.Sprintf("%d-%d", b.Lo, b.Hi-1) }

// Log is the state rebuilt from one log.
type Log struct {
	Words  Words
	Budget int // lines wake prints
	items  []entry
	prefix []uint64 // prefix[i] = rolling hash of items[0:i] ids
	pow    []uint64
	sums   map[Block][]summary
}

func New(w Words) *Log {
	return &Log{Words: w, Budget: WakeLines, prefix: []uint64{0}, pow: []uint64{1}, sums: map[Block][]summary{}}
}

func (m *Log) T() int { return len(m.items) }

func idHash(id string) uint64 {
	h := fnv.New64a()
	h.Write([]byte(id))
	return h.Sum64() | 1
}

func (m *Log) Add(id, date, text string) {
	m.items = append(m.items, entry{ID: id, Date: date, Text: text})
	m.prefix = append(m.prefix, m.prefix[len(m.prefix)-1]*hashBase+idHash(id))
	m.pow = append(m.pow, m.pow[len(m.pow)-1]*hashBase)
}

// Digest identifies exactly which entries, in which order, fill [lo, hi).
func (m *Log) Digest(b Block) string {
	if b.Hi > m.T() {
		return ""
	}
	d := m.prefix[b.Hi] - m.prefix[b.Lo]*m.pow[b.Hi-b.Lo]
	return strconv.FormatUint(d, 36)
}

func (m *Log) PutSummary(b Block, digest, text string) {
	m.sums[b] = append(m.sums[b], summary{Digest: digest, Text: text})
}

// Forget drops a summary and every summary built on top of it.
func (m *Log) Forget(b Block) int {
	n := 0
	for size := b.Hi - b.Lo; size <= max(m.T(), b.Hi-b.Lo); size *= 2 {
		lo := (b.Lo / size) * size
		k := Block{lo, lo + size}
		n += len(m.sums[k])
		delete(m.sums, k)
	}
	return n
}

// Summary returns the block's summary if one still matches its entries.
func (m *Log) Summary(b Block) (string, bool) {
	d := m.Digest(b)
	if d == "" {
		return "", false
	}
	list := m.sums[b]
	for i := len(list) - 1; i >= 0; i-- {
		if list[i].Digest == d {
			return list[i].Text, true
		}
	}
	return "", false
}

// Headline is the log in one line: its widest settled summary, the latest
// of those, or else its latest entry. It is "" for an empty log.
func (m *Log) Headline() string {
	size := 1
	for size*2 <= m.T() {
		size *= 2
	}
	for ; size >= 2; size /= 2 {
		for k := m.T()/size - 1; k >= 0; k-- {
			if s, ok := m.Summary(Block{k * size, (k + 1) * size}); ok {
				return s
			}
		}
	}
	if m.T() == 0 {
		return ""
	}
	return m.items[m.T()-1].Text
}

// Pending lists blocks that can be compressed and have not been, smallest
// first. A block above RawMax is compressed from its two halves, so it only
// appears once both halves are settled.
func (m *Log) Pending(limit int) []Block {
	T := m.T()
	var todo []Block
	for size := 2; size <= T; size *= 2 {
		for k := 0; k < T/size; k++ {
			b := Block{k * size, (k + 1) * size}
			if _, ok := m.Summary(b); ok {
				continue
			}
			if size > RawMax {
				mid := (b.Lo + b.Hi) / 2
				if _, ok := m.Summary(Block{b.Lo, mid}); !ok {
					continue
				}
				if _, ok := m.Summary(Block{mid, b.Hi}); !ok {
					continue
				}
			}
			todo = append(todo, b)
			if limit > 0 && len(todo) >= limit {
				return todo
			}
		}
	}
	return todo
}

// Line is entry i as every output prints it.
func (m *Log) Line(i int) string {
	e := m.items[i]
	return fmt.Sprintf("#%d %s %s", i, e.Date, e.Text)
}

func (m *Log) NapPrompt(b Block, left int) string {
	var body []string
	if b.Hi-b.Lo <= RawMax {
		for i := b.Lo; i < b.Hi; i++ {
			body = append(body, "  "+m.Line(i))
		}
	} else {
		mid := (b.Lo + b.Hi) / 2
		for _, h := range []Block{{b.Lo, mid}, {mid, b.Hi}} {
			s, _ := m.Summary(h)
			body = append(body, fmt.Sprintf("  #%s %s", h, s))
		}
	}
	tail := ""
	if left == 1 {
		tail = "\n1 compression remains after this one."
	} else if left > 1 {
		tail = fmt.Sprintf("\n%d compressions remain after this one.", left)
	}
	return fmt.Sprintf("Compress %s #%s into one line of at most %d bytes.\n"+
		"Keep what has lasting effect, drop what does not. Invent nothing.\n\n%s\n%s\nThen call %s.",
		m.Words.Many, b, EntryChars, strings.Join(body, "\n"), tail, m.Words.Nap(b.String()))
}

// NextNap is the prompt for the next compression, or "" if none is due.
func (m *Log) NextNap() string {
	todo := m.Pending(1)
	if len(todo) == 0 {
		return ""
	}
	return m.NapPrompt(todo[0], len(m.Pending(0))-1)
}

// cover tiles [0,T) with aligned power-of-two blocks; a block stays whole iff
// its size is at most alpha times its age. Bigger alpha = coarser.
func cover(T int, alpha float64) []Block {
	root := 1
	for root < T {
		root *= 2
	}
	var out []Block
	stack := []Block{{0, root}}
	for len(stack) > 0 {
		b := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		if b.Lo >= T {
			continue
		}
		size := b.Hi - b.Lo
		if size > 1 && (b.Hi > T || float64(size) > alpha*float64(T-b.Lo)) {
			mid := (b.Lo + b.Hi) / 2
			stack = append(stack, Block{mid, b.Hi}, Block{b.Lo, mid})
		} else {
			out = append(out, b)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Lo < out[j].Lo })
	return out
}

// Cover is what wake prints: at most budget blocks, finest near T. Detail
// decays with age; if everything fits, nothing is compressed at all.
func Cover(T, budget int) []Block {
	if T <= 0 {
		return nil
	}
	if T <= budget {
		out := make([]Block, T)
		for i := range out {
			out[i] = Block{i, i + 1}
		}
		return out
	}
	lo, hi := 0.0, 1.0
	for range 60 {
		mid := (lo + hi) / 2
		if len(cover(T, mid)) > budget {
			lo = mid
		} else {
			hi = mid
		}
	}
	out := cover(T, hi)
	// Sizes jump in powers of two, so alpha can undershoot the budget. Spend
	// what is left on the present, where detail is worth most.
	for len(out) < budget {
		i := -1
		for j := len(out) - 1; j >= 0; j-- {
			if out[j].Hi-out[j].Lo > 1 {
				i = j
				break
			}
		}
		if i < 0 {
			break
		}
		b := out[i]
		mid := (b.Lo + b.Hi) / 2
		out = append(out[:i], append([]Block{{b.Lo, mid}, {mid, b.Hi}}, out[i+1:]...)...)
	}
	return out
}

func paginate(lines []string) [][]string {
	var parts [][]string
	var cur []string
	size := 0
	for _, l := range lines {
		n := len(l) + 1
		if len(cur) > 0 && (len(cur) >= PartLines || size+n > PartChars) {
			parts = append(parts, cur)
			cur, size = nil, 0
		}
		cur = append(cur, l)
		size += n
	}
	if len(cur) > 0 {
		parts = append(parts, cur)
	}
	return parts
}

// Wake renders the log's context, ending its last part with end. part is
// 1-based; at is the snapshot T from part 1, so later parts line up even if
// entries arrive in between.
func (m *Log) Wake(part, at int, end string) (string, error) {
	T := m.T()
	if part < 1 {
		part = 1
	}
	if at > 0 {
		if at > T {
			return "", fmt.Errorf("at=%d, but %s holds %d", at, m.Words.Log, T)
		}
		T = at
	}
	if T == 0 {
		return m.Words.Empty + "\n" + end, nil
	}
	view := m.view(T)
	var lines []string
	for _, b := range Cover(T, m.Budget) {
		if b.Hi-b.Lo == 1 {
			lines = append(lines, m.Line(b.Lo))
			continue
		}
		s, ok := view.Summary(b)
		if !ok {
			nap := view.NextNap()
			return fmt.Sprintf("Cannot wake: %s context needs #%s, which is not compressed yet.\n"+
				"Do the %d compressions below, then %s.\n\n%s", m.Words.Log, b, len(view.Pending(0)), m.Words.Again, nap), nil
		}
		lines = append(lines, fmt.Sprintf("#%s %s", b, s))
	}
	parts := paginate(lines)
	if part > len(parts) {
		return "", fmt.Errorf("no part %d: %s has %d parts", part, m.Words.Log, len(parts))
	}
	var b strings.Builder
	if len(parts) > 1 {
		fmt.Fprintf(&b, "%s, part %d of %d, oldest first (%d %s).\n", m.Words.Title, part, len(parts), T, m.Words.Many)
	}
	b.WriteString(strings.Join(parts[part-1], "\n"))
	if part < len(parts) {
		fmt.Fprintf(&b, "\nNot awake yet. %s.", m.Words.Next(part+1, T))
	} else {
		b.WriteString("\n" + end)
		if nap := m.NextNap(); nap != "" {
			b.WriteString("\n\n" + nap)
		}
	}
	return b.String(), nil
}

// view is the log as of its first T entries (for paged reads).
func (m *Log) view(T int) *Log {
	if T == m.T() {
		return m
	}
	return &Log{Words: m.Words, Budget: m.Budget, items: m.items[:T], prefix: m.prefix[:T+1], pow: m.pow[:T+1], sums: m.sums}
}

// Zoom opens a block into its two halves.
func (m *Log) Zoom(b Block) (string, error) {
	if b.Lo >= m.T() {
		return "", fmt.Errorf("#%s is beyond %s: it holds %d", b, m.Words.Log, m.T())
	}
	mid := (b.Lo + b.Hi) / 2
	var out []string
	for _, h := range []Block{{b.Lo, mid}, {mid, b.Hi}} {
		if h.Lo >= m.T() {
			continue
		}
		if h.Hi-h.Lo == 1 {
			out = append(out, m.Line(h.Lo))
			continue
		}
		s, ok := m.Summary(h)
		if !ok {
			s = "not compressed yet"
		}
		out = append(out, fmt.Sprintf("#%s %s", h, s))
	}
	return strings.Join(out, "\n"), nil
}

// Lines yields every entry's line, oldest first.
func (m *Log) Lines() iter.Seq[string] {
	return func(yield func(string) bool) {
		for i := range m.items {
			if !yield(m.Line(i)) {
				return
			}
		}
	}
}

// Recall searches every entry, keeping the newest matches that fit a part.
func (m *Log) Recall(pattern string) (string, error) { return Search(pattern, m.Lines()) }

// Search runs a case-insensitive regular expression over lines, oldest
// first, keeping the newest matches that fit a part.
func Search(pattern string, lines iter.Seq[string]) (string, error) {
	re, err := regexp.Compile("(?i)" + pattern)
	if err != nil {
		return "", fmt.Errorf("bad regex: %w", err)
	}
	var out []string
	hits, size := 0, 0
	for l := range lines {
		if !re.MatchString(l) {
			continue
		}
		hits++
		out = append(out, l)
		size += len(l) + 1
		for size > PartChars {
			size -= len(out[0]) + 1
			out = out[1:]
		}
	}
	if hits == 0 {
		return "No match.", nil
	}
	s := strings.Join(out, "\n")
	if len(out) < hits {
		return s + fmt.Sprintf("\nNewest %d of %d matches. Narrow the regex.", len(out), hits), nil
	}
	if hits == 1 {
		return s + "\n1 match.", nil
	}
	return s + fmt.Sprintf("\n%d matches.", hits), nil
}

var blockID = regexp.MustCompile(`^(\d+)-(\d+)$`)

// ParseBlock reads a block id as wake prints it: "lo-hi", inclusive.
func ParseBlock(s string) (Block, error) {
	m := blockID.FindStringSubmatch(strings.TrimPrefix(strings.TrimSpace(s), "#"))
	if m == nil {
		return Block{}, fmt.Errorf("%q is not a block id; copy it as printed, like 16-31", s)
	}
	lo, _ := strconv.Atoi(m[1])
	hi, _ := strconv.Atoi(m[2])
	hi++
	n := hi - lo
	if n < 2 || n&(n-1) != 0 || lo%n != 0 {
		return Block{}, fmt.Errorf("%s is not a block; copy the id printed by wake, like 16-31", s)
	}
	return Block{lo, hi}, nil
}

// Check validates one entry or summary line.
func (w Words) Check(text string) (string, error) {
	text = strings.TrimSpace(text)
	switch {
	case text == "":
		return "", fmt.Errorf("empty: a %s is one line of text", w.One)
	case strings.ContainsAny(text, "\r\n"):
		return "", fmt.Errorf("%d lines: a %s is one line; merge them, or %s them separately", strings.Count(text, "\n")+1, w.One, w.Write)
	case len(text) > EntryChars:
		return "", fmt.Errorf("too long: %d bytes, limit %d (accented characters cost 2 bytes); compress it further", len(text), EntryChars)
	}
	return text, nil
}
