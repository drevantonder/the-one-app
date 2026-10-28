package memory

// The memory model is OptMem's (github.com/VictorTaelin/OptMem), rewritten:
// an append-only log of one-line memories, and a binary tree of one-line
// summaries over aligned power-of-two blocks, written by the agent itself.
//
// One change for the mesh: memories from several devices merge into one
// order, and a late arrival can shift positions. So each summary records a
// digest of exactly which memories it covers, and only counts while that
// digest still matches. Stale summaries are ignored and simply asked for
// again, so the tree heals itself after any merge.

import (
	"fmt"
	"hash/fnv"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

const (
	WakeLines  = 96    // lines wake prints (~8k tokens)
	EntryChars = 280   // longest memory or summary, in bytes
	PartChars  = 20000 // output paging: largest part, in bytes
	PartLines  = 500   // output paging: largest part, in lines
	RawMax     = 16    // blocks up to this size are compressed from raw memories
	hashBase   = 1099511628211
)

type mem struct {
	ID   string
	Date string
	Text string
}

type summary struct {
	Digest string
	Text   string
}

type block struct{ Lo, Hi int } // [Lo, Hi)

func (b block) String() string { return fmt.Sprintf("%d-%d", b.Lo, b.Hi-1) }

// Memory is the state rebuilt from the log.
type Memory struct {
	mems   []mem
	prefix []uint64 // prefix[i] = rolling hash of mems[0:i] ids
	pow    []uint64
	sums   map[block][]summary
}

func New() *Memory {
	return &Memory{prefix: []uint64{0}, pow: []uint64{1}, sums: map[block][]summary{}}
}

func (m *Memory) T() int { return len(m.mems) }

func idHash(id string) uint64 {
	h := fnv.New64a()
	h.Write([]byte(id))
	return h.Sum64() | 1
}

func (m *Memory) Add(id, date, text string) {
	m.mems = append(m.mems, mem{ID: id, Date: date, Text: text})
	m.prefix = append(m.prefix, m.prefix[len(m.prefix)-1]*hashBase+idHash(id))
	m.pow = append(m.pow, m.pow[len(m.pow)-1]*hashBase)
}

// Digest identifies exactly which memories, in which order, fill [lo, hi).
func (m *Memory) Digest(b block) string {
	if b.Hi > m.T() {
		return ""
	}
	d := m.prefix[b.Hi] - m.prefix[b.Lo]*m.pow[b.Hi-b.Lo]
	return strconv.FormatUint(d, 36)
}

func (m *Memory) PutSummary(b block, digest, text string) {
	m.sums[b] = append(m.sums[b], summary{Digest: digest, Text: text})
}

// Forget drops a summary and every summary built on top of it.
func (m *Memory) Forget(b block) int {
	n := 0
	for size := b.Hi - b.Lo; size <= max(m.T(), b.Hi-b.Lo); size *= 2 {
		lo := (b.Lo / size) * size
		k := block{lo, lo + size}
		n += len(m.sums[k])
		delete(m.sums, k)
	}
	return n
}

// Summary returns the block's summary if one still matches its memories.
func (m *Memory) Summary(b block) (string, bool) {
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

// Pending lists blocks that can be compressed and have not been, smallest
// first. A block above RawMax is compressed from its two halves, so it only
// appears once both halves are settled.
func (m *Memory) Pending(limit int) []block {
	T := m.T()
	var todo []block
	for size := 2; size <= T; size *= 2 {
		for k := 0; k < T/size; k++ {
			b := block{k * size, (k + 1) * size}
			if _, ok := m.Summary(b); ok {
				continue
			}
			if size > RawMax {
				mid := (b.Lo + b.Hi) / 2
				if _, ok := m.Summary(block{b.Lo, mid}); !ok {
					continue
				}
				if _, ok := m.Summary(block{mid, b.Hi}); !ok {
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

func (m *Memory) line(i int) string {
	e := m.mems[i]
	return fmt.Sprintf("#%d %s %s", i, e.Date, e.Text)
}

const napCall = `memory_nap with block "%s" and text "<your line>" (CLI: one memory nap %s "<your line>")`

func (m *Memory) NapPrompt(b block, left int) string {
	var body []string
	if b.Hi-b.Lo <= RawMax {
		for i := b.Lo; i < b.Hi; i++ {
			body = append(body, "  "+m.line(i))
		}
	} else {
		mid := (b.Lo + b.Hi) / 2
		for _, h := range []block{{b.Lo, mid}, {mid, b.Hi}} {
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
	return fmt.Sprintf("Compress memories #%s into one line of at most %d bytes.\n"+
		"Keep what has lasting effect, drop what does not. Invent nothing.\n\n%s\n%s\nThen call "+napCall+".",
		b, EntryChars, strings.Join(body, "\n"), tail, b, b)
}

// NextNap is the prompt for the next compression, or "" if none is due.
func (m *Memory) NextNap() string {
	todo := m.Pending(1)
	if len(todo) == 0 {
		return ""
	}
	return m.NapPrompt(todo[0], len(m.Pending(0))-1)
}

// cover tiles [0,T) with aligned power-of-two blocks; a block stays whole iff
// its size is at most alpha times its age. Bigger alpha = coarser.
func cover(T int, alpha float64) []block {
	root := 1
	for root < T {
		root *= 2
	}
	var out []block
	stack := []block{{0, root}}
	for len(stack) > 0 {
		b := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		if b.Lo >= T {
			continue
		}
		size := b.Hi - b.Lo
		if size > 1 && (b.Hi > T || float64(size) > alpha*float64(T-b.Lo)) {
			mid := (b.Lo + b.Hi) / 2
			stack = append(stack, block{mid, b.Hi}, block{b.Lo, mid})
		} else {
			out = append(out, b)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Lo < out[j].Lo })
	return out
}

// Cover is what wake prints: at most budget blocks, finest near T. Detail
// decays with age; if everything fits, nothing is compressed at all.
func Cover(T, budget int) []block {
	if T <= 0 {
		return nil
	}
	if T <= budget {
		out := make([]block, T)
		for i := range out {
			out[i] = block{i, i + 1}
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
		out = append(out[:i], append([]block{{b.Lo, mid}, {mid, b.Hi}}, out[i+1:]...)...)
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

// Wake renders the memory context. part is 1-based; at is the snapshot T
// from part 1, so later parts line up even if notes arrive in between.
func (m *Memory) Wake(part int, at int) (string, error) {
	T := m.T()
	if part < 1 {
		part = 1
	}
	if at > 0 {
		if at > T {
			return "", fmt.Errorf("at=%d, but the memory holds %d", at, T)
		}
		T = at
	}
	if T == 0 {
		return "No memories yet. Record the first with memory_note.\nYou are awake.", nil
	}
	view := m.view(T)
	var lines []string
	for _, b := range Cover(T, WakeLines) {
		if b.Hi-b.Lo == 1 {
			lines = append(lines, m.line(b.Lo))
			continue
		}
		s, ok := view.Summary(b)
		if !ok {
			nap := view.NextNap()
			return fmt.Sprintf("Cannot wake: the memory context needs #%s, which is not compressed yet.\n"+
				"Do the %d compressions below, then call memory_wake again.\n\n%s", b, len(view.Pending(0)), nap), nil
		}
		lines = append(lines, fmt.Sprintf("#%s %s", b, s))
	}
	parts := paginate(lines)
	if part > len(parts) {
		return "", fmt.Errorf("no part %d: the memory has %d parts", part, len(parts))
	}
	var b strings.Builder
	if len(parts) > 1 {
		fmt.Fprintf(&b, "Your memory, part %d of %d, oldest first (%d memories).\n", part, len(parts), T)
	}
	b.WriteString(strings.Join(parts[part-1], "\n"))
	if part < len(parts) {
		fmt.Fprintf(&b, "\nNot awake yet. Call memory_wake with part %d and at %d (CLI: one memory wake %d %d).", part+1, T, part+1, T)
	} else {
		b.WriteString("\nYou are awake.")
		if nap := m.NextNap(); nap != "" {
			b.WriteString("\n\n" + nap)
		}
	}
	return b.String(), nil
}

// view is the memory as of the first T memories (for paged reads).
func (m *Memory) view(T int) *Memory {
	if T == m.T() {
		return m
	}
	return &Memory{mems: m.mems[:T], prefix: m.prefix[:T+1], pow: m.pow[:T+1], sums: m.sums}
}

// Zoom opens a block into its two halves.
func (m *Memory) Zoom(b block) (string, error) {
	if b.Lo >= m.T() {
		return "", fmt.Errorf("#%s is beyond the memory: it holds %d", b, m.T())
	}
	mid := (b.Lo + b.Hi) / 2
	var out []string
	for _, h := range []block{{b.Lo, mid}, {mid, b.Hi}} {
		if h.Lo >= m.T() {
			continue
		}
		if h.Hi-h.Lo == 1 {
			out = append(out, m.line(h.Lo))
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

// Recall searches every memory, keeping the newest matches that fit a part.
func (m *Memory) Recall(pattern string) (string, error) {
	re, err := regexp.Compile("(?i)" + pattern)
	if err != nil {
		return "", fmt.Errorf("bad regex: %w", err)
	}
	var out []string
	hits, size := 0, 0
	for i := range m.mems {
		l := m.line(i)
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
func ParseBlock(s string) (block, error) {
	m := blockID.FindStringSubmatch(strings.TrimPrefix(strings.TrimSpace(s), "#"))
	if m == nil {
		return block{}, fmt.Errorf("%q is not a block id; copy it as printed, like 16-31", s)
	}
	lo, _ := strconv.Atoi(m[1])
	hi, _ := strconv.Atoi(m[2])
	hi++
	n := hi - lo
	if n < 2 || n&(n-1) != 0 || lo%n != 0 {
		return block{}, fmt.Errorf("%s is not a block; copy the id printed by wake, like 16-31", s)
	}
	return block{lo, hi}, nil
}

// CheckText validates one memory or summary line.
func CheckText(text string) (string, error) {
	text = strings.TrimSpace(text)
	switch {
	case text == "":
		return "", fmt.Errorf("empty: a memory is one line of text")
	case strings.ContainsAny(text, "\r\n"):
		return "", fmt.Errorf("%d lines: a memory is one line; merge them, or note them separately", strings.Count(text, "\n")+1)
	case len(text) > EntryChars:
		return "", fmt.Errorf("too long: %d bytes, limit %d (accented characters cost 2 bytes); compress it further", len(text), EntryChars)
	}
	return text, nil
}
