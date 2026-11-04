package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"time"
)

// The browser tools for a chrome device, answered here over its DevTools pipe as Cua's driver answers them for a
// Linux device: the same input, and answers of the same shape (a line saying what happened, then its details as
// JSON). A page is read from Chrome's accessibility tree: its text as an outline, with a ref (p3:12) on each thing
// to click, type in or select. target_id is the device's Chrome, and tab_id one of its tabs; there are no windows,
// so neither pid nor window_id is needed, and a session is taken but not needed either: the device is one agent's.

// readLines is how many lines of a page one read gives; continuation reads on.
const readLines = 400

func targetOf(v *vm) string { return "bt-" + v.Name }

// answer is a tool's answer: what happened, then its details.
func answer(said string, details map[string]any, more ...map[string]any) map[string]any {
	b, _ := json.Marshal(details)
	items := append([]map[string]any{{"type": "text", "text": said}}, more...)
	return map[string]any{"content": append(items, map[string]any{"type": "text", "text": string(b)})}
}

// refused is an answer that did nothing, and why, as Cua's driver refuses.
func refused(code, why string, details map[string]any) map[string]any {
	if details == nil {
		details = map[string]any{}
	}
	details["status"], details["refusal"] = "refused", map[string]any{"code": code, "message": why}
	return answer(fmt.Sprintf("refused (%s): %s", code, why), details)
}

func (ds *devices) browse(ctx context.Context, v *vm, op string, in map[string]any) (any, error) {
	if err := ds.awake(ctx, v); err != nil {
		return nil, err
	}
	b, err := liveBrowser(v)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	text := func(k string) string { s, _ := in[k].(string); return s }
	number := func(k string) (float64, bool) { n, ok := in[k].(float64); return n, ok }
	target := targetOf(v)
	if t := text("target_id"); t != "" && t != target {
		return refused("browser_target_unknown", fmt.Sprintf("target_id %s isn't %s's Chrome: it's %s", t, v.Name, target), nil), nil
	}
	if op == "browser_prepare" {
		p, err := b.currentPage(ctx)
		if err != nil {
			return nil, err
		}
		tabs, _ := b.tabs(ctx)
		return answer(fmt.Sprintf("browser_prepare: ready — Chrome on %s, target_id %s, tab_id %s", v.Name, target, tabID(p.target)), map[string]any{
			"action": "attached", "prepared": true, "status": "ok", "target_id": target, "tab_id": tabID(p.target), "tabs": tabs,
			"message": "Chrome is running: pass target_id and tab_id to the browser tools; get_browser_state reads the page.",
		}), nil
	}
	p, err := b.pageOf(ctx, text("tab_id"))
	if err != nil {
		return refused("browser_tab_unknown", err.Error(), map[string]any{"target_id": target}), nil
	}
	tab := tabID(p.target)
	ids := map[string]any{"target_id": target, "tab_id": tab}
	with := func(m map[string]any) map[string]any {
		for k, v := range ids {
			m[k] = v
		}
		return m
	}
	b.mu.Lock()
	open := p.dialog
	b.mu.Unlock()
	if open != nil && op != "browser_dialog" {
		return refused("browser_dialog_open", fmt.Sprintf("a %s dialog is open in %s (%q): answer it with browser_dialog first", open.Kind, tab, open.Message),
			with(map[string]any{"dialog_id": open.ID})), nil
	}
	// node is the page's thing a ref names, or a refusal.
	node := func(ref string) (int64, map[string]any) {
		b.mu.Lock()
		id, ok := p.refs[ref]
		b.mu.Unlock()
		if !ok {
			return 0, refused("browser_ref_stale", fmt.Sprintf("ref %s is stale — the page navigated or it isn't one of this tab's; re-run get_browser_state for fresh refs", ref), with(map[string]any{}))
		}
		return id, nil
	}
	mode := cmpOr(text("delivery_mode"), "background")
	switch op {
	case "get_browser_state":
		return b.state(ctx, p, ids, in)
	case "browser_navigate":
		url := strings.TrimSpace(text("url"))
		if !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") && !strings.HasPrefix(url, "about:") {
			return nil, fmt.Errorf("browser_navigate only accepts http/https/about URLs, got: %s", url)
		}
		if err := b.navigate(ctx, p, url); err != nil {
			return nil, err
		}
		return answer(fmt.Sprintf("navigated %s to %s", tab, url), with(map[string]any{"refs_invalidated": true, "status": "ok", "url": url})), nil
	case "browser_click":
		route := cmpOr(text("input_route"), "trusted")
		ref := text("ref")
		if route == "dom_event" {
			if ref == "" {
				return refused("browser_ref_required", "input_route dom_event needs ref", with(map[string]any{})), nil
			}
			id, no := node(ref)
			if no != nil {
				return no, nil
			}
			// Clicked after this returns, so a dialog it opens doesn't hold the call.
			if _, err := b.on(ctx, p, id, `function() { this.scrollIntoView({ block: "center", inline: "center" }); setTimeout(() => this.click(), 0); return true }`); err != nil {
				return refused("browser_ref_stale", fmt.Sprintf("ref %s is gone from the page: re-run get_browser_state", ref), with(map[string]any{})), nil
			}
			time.Sleep(150 * time.Millisecond)
			return acted(b, p, fmt.Sprintf("dispatched DOM click on %s in %s", ref, tab), "dom", mode, nil, ids), nil
		}
		x, y, no := b.point(ctx, p, ref, in, node)
		if no != nil {
			return no, nil
		}
		_, err := b.click(ctx, p, x, y, "left", 1)
		return acted(b, p, fmt.Sprintf("clicked (%.0f, %.0f) in %s", x, y, tab), "trusted_input", mode, err, ids), nil
	case "browser_pointer":
		action := text("action")
		var x, y float64
		var no map[string]any
		if action == "scroll" && text("ref") == "" {
			if _, ok := number("x"); !ok {
				x, y = 640, 400
			}
		}
		if x == 0 && y == 0 {
			if x, y, no = b.point(ctx, p, text("ref"), in, node); no != nil {
				return no, nil
			}
		}
		var err error
		switch action {
		case "hover":
			_, err = b.input(ctx, p, "Input.dispatchMouseEvent", map[string]any{"type": "mouseMoved", "x": x, "y": y})
		case "right_click":
			_, err = b.click(ctx, p, x, y, "right", 1)
		case "double_click":
			_, err = b.click(ctx, p, x, y, "left", 2)
		case "scroll":
			dx, _ := number("delta_x")
			dy, _ := number("delta_y")
			_, err = b.input(ctx, p, "Input.dispatchMouseEvent", map[string]any{"type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy})
		case "drag":
			toX, okX := number("to_x")
			toY, okY := number("to_y")
			if dest := text("destination_ref"); dest != "" {
				if toX, toY, no = b.point(ctx, p, dest, nil, node); no != nil {
					return no, nil
				}
			} else if !okX || !okY {
				return nil, errors.New("drag to where? destination_ref, or to_x and to_y")
			}
			_, err = b.drag(ctx, p, x, y, toX, toY)
		default:
			return nil, errors.New("action is hover, right_click, double_click, scroll or drag")
		}
		return acted(b, p, fmt.Sprintf("dispatched %s at (%.0f, %.0f) in %s", action, x, y, tab), "trusted_input", mode, err, ids), nil
	case "browser_type":
		id, no := node(text("ref"))
		if no != nil {
			return no, nil
		}
		replace, _ := in["replace"].(bool)
		if _, err := b.on(ctx, p, id, focusField, replace); err != nil {
			return refused("browser_ref_stale", fmt.Sprintf("ref %s is gone from the page: re-run get_browser_state", text("ref")), with(map[string]any{})), nil
		}
		typed := text("text")
		n := 0
		var err error
		switch {
		case typed == "" && replace:
			_, err = b.key(ctx, p, "backspace")
		case text("mode") == "keystrokes":
			for _, r := range typed {
				if r == '\n' {
					_, err = b.key(ctx, p, "enter")
				} else {
					ch := string(r)
					if _, err = b.input(ctx, p, "Input.dispatchKeyEvent", map[string]any{"type": "keyDown", "key": ch, "text": ch, "unmodifiedText": ch}); err == nil {
						_, err = b.input(ctx, p, "Input.dispatchKeyEvent", map[string]any{"type": "keyUp", "key": ch})
					}
				}
				if err != nil {
					break
				}
				n++
			}
		default:
			_, err = b.input(ctx, p, "Input.insertText", map[string]any{"text": typed})
			n = len([]rune(typed))
		}
		if err != nil && !errors.Is(err, errDialog) {
			return nil, err
		}
		details := with(map[string]any{"delivery": map[string]any{"delivered_count": n, "mode": "background"}, "effect": "unverifiable", "route": "trusted_input"})
		said := fmt.Sprintf("typed %d char(s) into %s", n, tab)
		if errors.Is(err, errDialog) {
			details["effect"], details["hint"] = "dialog_opened", "A dialog opened: answer it with browser_dialog."
			said += "; a dialog opened: answer it with browser_dialog"
		} else if raw, err := b.on(ctx, p, id, `function() { return this.type === "password" ? null : "value" in this ? String(this.value) : this.innerText }`); err == nil {
			var now *string
			json.Unmarshal(raw, &now)
			if now != nil {
				details["value"] = clip(*now, 300)
				if strings.Contains(*now, strings.TrimSuffix(typed, "\n")) {
					details["effect"] = "confirmed"
				}
			}
		}
		if details["effect"] == "unverifiable" {
			details["hint"] = "The text was typed but not read back. Read the field with get_browser_state before typing again."
		}
		details["summary"] = said
		return answer(said, details), nil
	case "browser_select":
		id, no := node(text("ref"))
		if no != nil {
			return no, nil
		}
		var label, value any
		if l, ok := in["label"].(string); ok {
			label = l
		} else if v, ok := in["value"].(string); ok {
			value = v
		} else {
			return nil, errors.New("select which option? give label or value")
		}
		raw, err := b.on(ctx, p, id, selectOption, label, value)
		if err != nil {
			return refused("browser_ref_stale", fmt.Sprintf("ref %s is gone from the page: re-run get_browser_state", text("ref")), with(map[string]any{})), nil
		}
		var got struct {
			Error   string   `json:"error"`
			Options []string `json:"options"`
			Changed bool     `json:"changed"`
			Label   string   `json:"label"`
			Value   string   `json:"value"`
		}
		json.Unmarshal(raw, &got)
		switch got.Error {
		case "not_select":
			return refused("browser_select_not_select", fmt.Sprintf("ref %s isn't a drop-down (a select): click it instead", text("ref")), with(map[string]any{})), nil
		case "no_match", "ambiguous":
			return refused("browser_select_option_"+got.Error, fmt.Sprintf("not exactly one option matches; its options are: %s", strings.Join(got.Options, ", ")), with(map[string]any{"options": got.Options})), nil
		}
		return answer(fmt.Sprintf("selected %q in %s and dispatched input and change events", got.Label, tab),
			with(map[string]any{"changed": got.Changed, "frame": "main", "label": got.Label, "ref": text("ref"), "status": "ok", "value": got.Value})), nil
	case "browser_dialog":
		switch text("action") {
		case "inspect":
			if open == nil {
				return answer("no page-owned JavaScript dialog is open in "+tab, with(map[string]any{"present": false, "status": "ok"})), nil
			}
			return answer(fmt.Sprintf("%s dialog is open in %s", open.Kind, tab), with(map[string]any{"present": true, "status": "ok", "dialog_id": open.ID,
				"kind": open.Kind, "message": open.Message, "default_prompt": open.Default})), nil
		case "accept", "dismiss":
			if open == nil {
				return refused("browser_dialog_absent", "no dialog is open in "+tab, with(map[string]any{})), nil
			}
			if id := text("dialog_id"); id != "" && id != open.ID {
				return refused("browser_dialog_stale", fmt.Sprintf("dialog %s is gone: the open one is %s", id, open.ID), with(map[string]any{})), nil
			}
			accept := text("action") == "accept"
			params := map[string]any{"accept": accept}
			if s, ok := in["prompt_text"].(string); ok && accept {
				params["promptText"] = s
			}
			if _, err := b.call(ctx, p.session, "Page.handleJavaScriptDialog", params); err != nil {
				return nil, err
			}
			b.mu.Lock()
			p.dialog = nil
			b.mu.Unlock()
			return answer(fmt.Sprintf("%s %s dialog in %s", map[bool]string{true: "accepted", false: "dismissed"}[accept], open.Kind, tab),
				with(map[string]any{"action": text("action"), "dialog_id": open.ID, "kind": open.Kind, "status": "ok"})), nil
		}
		return nil, errors.New("action is inspect, accept or dismiss")
	}
	return nil, fmt.Errorf("no browser tool %s", op)
}

// acted is the answer to input sent to a page: unverifiable, as a click is until the page is read again, or a dialog
// it opened.
func acted(b *browser, p *page, said, route, mode string, err error, ids map[string]any) map[string]any {
	details := map[string]any{"delivery": map[string]any{"mode": mode}, "effect": "unverifiable", "route": route,
		"hint": "The action was sent but nothing confirmed it. Read the page again with get_browser_state and check the change before repeating."}
	b.mu.Lock()
	open := p.dialog
	b.mu.Unlock()
	if errors.Is(err, errDialog) || open != nil {
		details["effect"], details["hint"] = "dialog_opened", "A dialog opened: answer it with browser_dialog."
		said += "; a dialog opened: answer it with browser_dialog"
	} else if err != nil {
		return refused("browser_input_failed", err.Error(), details)
	}
	for k, v := range ids {
		details[k] = v
	}
	details["summary"] = said
	return answer(said, details)
}

// focusField focuses a field, its text selected to be replaced, or the caret at its end.
const focusField = `function(replace) {
	this.scrollIntoView({ block: "center", inline: "center" });
	this.focus();
	const field = "value" in this && typeof this.setSelectionRange === "function";
	if (field) {
		try { replace ? this.select() : this.setSelectionRange(this.value.length, this.value.length); } catch (e) { if (replace) this.select(); }
	} else if (this.isContentEditable) {
		const r = document.createRange(); r.selectNodeContents(this); if (!replace) r.collapse(false);
		const s = getSelection(); s.removeAllRanges(); s.addRange(r);
	}
	return true;
}`

// selectOption picks the one option of a select whose label (or value) is the one given.
const selectOption = `function(label, value) {
	if (this.tagName !== "SELECT") return { error: "not_select" };
	const all = [...this.options];
	const hit = all.filter((o) => label != null ? o.label.trim() === label.trim() || o.text.trim() === label.trim() : o.value === value);
	if (hit.length !== 1) return { error: hit.length ? "ambiguous" : "no_match", options: all.map((o) => o.text.trim()).slice(0, 60) };
	const was = this.value;
	hit[0].selected = true;
	this.dispatchEvent(new Event("input", { bubbles: true }));
	this.dispatchEvent(new Event("change", { bubbles: true }));
	return { changed: was !== this.value, label: hit[0].text.trim(), value: this.value };
}`

// on runs a function on a node of the page, with this as the node, and gives what it returns.
func (b *browser) on(ctx context.Context, p *page, backend int64, fn string, args ...any) (json.RawMessage, error) {
	raw, err := b.call(ctx, p.session, "DOM.resolveNode", map[string]any{"backendNodeId": backend})
	if err != nil {
		return nil, err
	}
	var obj struct {
		Object struct{ ObjectID string } `json:"object"`
	}
	json.Unmarshal(raw, &obj)
	as := []map[string]any{}
	for _, a := range args {
		as = append(as, map[string]any{"value": a})
	}
	raw, err = b.call(ctx, p.session, "Runtime.callFunctionOn", map[string]any{"objectId": obj.Object.ObjectID, "functionDeclaration": fn, "arguments": as, "returnByValue": true})
	if err != nil {
		return nil, err
	}
	var r struct {
		Result struct {
			Value json.RawMessage `json:"value"`
		} `json:"result"`
		ExceptionDetails *struct {
			Text string `json:"text"`
		} `json:"exceptionDetails"`
	}
	json.Unmarshal(raw, &r)
	if r.ExceptionDetails != nil {
		return nil, errors.New(r.ExceptionDetails.Text)
	}
	return r.Result.Value, nil
}

// point is where to act: the middle of the thing ref names, scrolled into view, or x and y.
func (b *browser) point(ctx context.Context, p *page, ref string, in map[string]any, node func(string) (int64, map[string]any)) (float64, float64, map[string]any) {
	if ref == "" {
		x, okX := in["x"].(float64)
		y, okY := in["y"].(float64)
		if !okX || !okY {
			return 0, 0, refused("browser_target_required", "give ref, or x and y", nil)
		}
		return x, y, nil
	}
	id, no := node(ref)
	if no != nil {
		return 0, 0, no
	}
	b.call(ctx, p.session, "DOM.scrollIntoViewIfNeeded", map[string]any{"backendNodeId": id})
	raw, err := b.call(ctx, p.session, "DOM.getContentQuads", map[string]any{"backendNodeId": id})
	var q struct{ Quads [][]float64 }
	json.Unmarshal(raw, &q)
	if err != nil || len(q.Quads) == 0 || len(q.Quads[0]) < 8 {
		return 0, 0, refused("browser_ref_not_visible", fmt.Sprintf("ref %s isn't on the page's screen (hidden, or gone): re-run get_browser_state", ref), nil)
	}
	c := q.Quads[0]
	return (c[0] + c[2] + c[4] + c[6]) / 4, (c[1] + c[3] + c[5] + c[7]) / 4, nil
}

func clip(s string, n int) string {
	if r := []rune(s); len(r) > n {
		return string(r[:n]) + "…"
	}
	return s
}

// An axNode is a node of Chrome's accessibility tree.
type axNode struct {
	NodeID     string   `json:"nodeId"`
	Ignored    bool     `json:"ignored"`
	Role       *axValue `json:"role"`
	Name       *axValue `json:"name"`
	Value      *axValue `json:"value"`
	Properties []struct {
		Name  string  `json:"name"`
		Value axValue `json:"value"`
	} `json:"properties"`
	ChildIDs []string `json:"childIds"`
	Backend  int64    `json:"backendDOMNodeId"`
}

type axValue struct {
	Value any `json:"value"`
}

func (v *axValue) text() string {
	if v == nil || v.Value == nil {
		return ""
	}
	switch x := v.Value.(type) {
	case string:
		return strings.Join(strings.Fields(x), " ")
	case float64:
		return strconv.FormatFloat(x, 'f', -1, 64)
	case bool:
		return strconv.FormatBool(x)
	}
	return fmt.Sprint(v.Value)
}

// Roles a ref goes on: what's clicked, typed in or picked.
var actable = []string{"link", "button", "textbox", "searchbox", "combobox", "checkbox", "radio", "switch", "slider", "spinbutton",
	"menuitem", "menuitemcheckbox", "menuitemradio", "tab", "option", "treeitem", "listbox", "menulistoption", "inputtime", "date",
	"datetime", "colorwell", "popupbutton", "comboboxselect", "comboboxmenubutton", "disclosuretriangle", "togglebutton"}

// Roles that show as a line of their own, holding what's in them, when named.
var holders = []string{"dialog", "alertdialog", "alert", "navigation", "main", "banner", "contentinfo", "form", "region", "group",
	"table", "list", "tablist", "menu", "menubar", "tree", "row", "cell", "columnheader", "rowheader", "listitem", "article", "complementary", "status"}

// state is get_browser_state: the page's text as an outline, a ref on each thing to act on.
func (b *browser) state(ctx context.Context, p *page, ids map[string]any, in map[string]any) (any, error) {
	text := func(k string) string { s, _ := in[k].(string); return s }
	format := cmpOr(text("snapshot_format"), "dom_refs_v1")
	raw, _ := b.call(ctx, "", "Target.getTargetInfo", map[string]any{"targetId": p.target})
	var info struct {
		TargetInfo struct{ Title, URL string } `json:"targetInfo"`
	}
	json.Unmarshal(raw, &info)
	tabs, _ := b.tabs(ctx)
	details := map[string]any{"mode": "snapshot", "status": "ok", "url": info.TargetInfo.URL, "title": info.TargetInfo.Title, "tabs": tabs}
	for k, v := range ids {
		details[k] = v
	}
	var lines []string
	var refs []map[string]any
	from := 0
	if c := text("continuation"); c != "" {
		id, at, _ := strings.Cut(c, ":")
		b.mu.Lock()
		same := id == p.snapshot
		lines = p.outline
		b.mu.Unlock()
		if !same {
			return refused("browser_continuation_stale", "that read was superseded or the page navigated: read it again", details), nil
		}
		from, _ = strconv.Atoi(at)
		details["snapshot_id"] = id
	} else {
		b.mu.Lock()
		b.snaps++
		id := fmt.Sprintf("p%d", b.snaps)
		b.mu.Unlock()
		var err error
		lines, refs, err = b.outline(ctx, p, id, text("scope_ref"), text("query"))
		if err != nil {
			return nil, err
		}
		b.mu.Lock()
		p.snapshot, p.outline = id, lines
		b.mu.Unlock()
		details["snapshot_id"] = id
		details["refs"] = refs
	}
	to := min(from+readLines, len(lines))
	from = min(from, to)
	shown := lines[from:to]
	details["truncated"] = to < len(lines)
	if to < len(lines) {
		details["continuation"] = fmt.Sprintf("%s:%d", details["snapshot_id"], to)
	}
	if format == "semantic_v2" {
		details["snapshot"] = map[string]any{"format": format, "id": details["snapshot_id"], "complete": to == len(lines), "continuation": details["continuation"]}
	}
	if refs != nil {
		// Only the refs in what's shown.
		in := strings.Join(shown, "\n")
		details["refs"] = slices.DeleteFunc(refs, func(r map[string]any) bool { return !strings.Contains(in, "[ref="+r["ref"].(string)+"]") })
	}
	said := fmt.Sprintf("snapshot %s of %s (%s): %d ref(s)", details["snapshot_id"], info.TargetInfo.URL, info.TargetInfo.Title, strings.Count(strings.Join(shown, "\n"), "[ref="))
	if from > 0 || to < len(lines) {
		said += fmt.Sprintf(", lines %d-%d of %d", from+1, to, len(lines))
	}
	if to < len(lines) {
		said += fmt.Sprintf("; continuation %q reads on", details["continuation"])
	}
	said += "\n" + strings.Join(shown, "\n")
	var shot []map[string]any
	if in["include_screenshot"] == true {
		raw, err := b.call(ctx, p.session, "Page.captureScreenshot", map[string]any{"format": "png"})
		var s struct{ Data string }
		if json.Unmarshal(raw, &s); err == nil && s.Data != "" {
			shot = append(shot, map[string]any{"type": "image", "mimeType": "image/png", "data": s.Data})
			details["screenshot"] = map[string]any{"coordinate_space": "viewport_css_px", "mime_type": "image/png", "source": "cdp_tab"}
		}
	}
	return answer(said, details, shot...), nil
}

// outline is the page's accessibility tree, and its frames', as lines of text, with refs (named id:n) on what can be
// acted on, which the page keeps until it navigates; from the node ref names only, and only lines matching query's
// words, when given.
func (b *browser) outline(ctx context.Context, p *page, id, scope, query string) ([]string, []map[string]any, error) {
	raw, err := b.call(ctx, p.session, "Page.getFrameTree", nil)
	if err != nil {
		return nil, nil, err
	}
	type frameTree struct {
		Frame       struct{ ID, URL string } `json:"frame"`
		ChildFrames []frameTree              `json:"childFrames"`
	}
	var tree struct {
		FrameTree frameTree `json:"frameTree"`
	}
	json.Unmarshal(raw, &tree)
	var frames []frameTree
	var walkFrames func(f frameTree)
	walkFrames = func(f frameTree) {
		frames = append(frames, f)
		for _, c := range f.ChildFrames {
			walkFrames(c)
		}
	}
	walkFrames(tree.FrameTree)
	var scopeID int64
	if scope != "" {
		b.mu.Lock()
		scopeID = p.refs[scope]
		b.mu.Unlock()
		if scopeID == 0 {
			return nil, nil, fmt.Errorf("scope_ref %s is stale: read the page again", scope)
		}
	}
	words := strings.Fields(strings.ToLower(query))
	var lines []string
	var refs []map[string]any
	n := 0
	for i, f := range frames {
		raw, err := b.call(ctx, p.session, "Accessibility.getFullAXTree", map[string]any{"frameId": f.Frame.ID})
		frame := "main"
		if i > 0 {
			frame = fmt.Sprintf("frame%d", i)
			if scopeID == 0 && len(words) == 0 {
				lines = append(lines, fmt.Sprintf("- frame %q", clip(f.Frame.URL, 120)))
			}
		}
		if err != nil {
			if i == 0 {
				return nil, nil, err
			}
			continue // another site's frame, in a process of its own
		}
		var got struct{ Nodes []axNode }
		json.Unmarshal(raw, &got)
		byID := map[string]*axNode{}
		child := map[string]bool{}
		for i := range got.Nodes {
			byID[got.Nodes[i].NodeID] = &got.Nodes[i]
			for _, c := range got.Nodes[i].ChildIDs {
				child[c] = true
			}
		}
		// inside: within the node scope_ref names; within: within something acted on, whose inner editor isn't one.
		var walk func(nd *axNode, depth int, inside, within bool)
		walk = func(nd *axNode, depth int, inside, within bool) {
			role := strings.ToLower(nd.Role.text())
			name := clip(nd.Name.text(), 200)
			line := ""
			states := map[string]string{}
			for _, pr := range nd.Properties {
				states[pr.Name] = pr.Value.text()
			}
			acts := !nd.Ignored && nd.Backend != 0 && !(within && role == "generic") && (slices.Contains(actable, role) || states["editable"] != "" ||
				(states["focusable"] == "true" && name != "" && !slices.Contains([]string{"rootwebarea", "webarea", "iframe", "document", "scrollbar"}, role)))
			if scopeID != 0 && nd.Backend == scopeID {
				inside = true
			}
			switch {
			case nd.Ignored || !inside:
			case acts:
				n++
				ref := fmt.Sprintf("%s:%d", id, n)
				b.mu.Lock()
				p.refs[ref] = nd.Backend
				b.mu.Unlock()
				line = "- " + role
				if name != "" {
					line += fmt.Sprintf(" %q", name)
				}
				if v := nd.Value.text(); v != "" && v != name {
					line += fmt.Sprintf(" = %q", clip(v, 200))
				}
				for _, s := range []string{"checked", "pressed", "selected", "expanded", "disabled", "required", "readonly", "focused", "invalid"} {
					switch v := states[s]; {
					case v == "true" || (s == "checked" && v == "mixed"):
						line += " [" + map[bool]string{true: "mixed", false: s}[v == "mixed"] + "]"
					case v == "false" && (s == "checked" || s == "expanded" || s == "pressed"):
						line += " [not " + s + "]"
					}
				}
				line += " [ref=" + ref + "]"
				refs = append(refs, map[string]any{"ref": ref, "role": role, "name": name, "frame": frame})
			case role == "statictext":
				if name != "" {
					line = "- text " + strconv.Quote(name)
				}
			case role == "heading":
				line = fmt.Sprintf("- heading %q", name)
				if l := states["level"]; l != "" {
					line += " (level " + l + ")"
				}
			case role == "image" || role == "img":
				if name != "" {
					line = fmt.Sprintf("- image %q", name)
				}
			case slices.Contains(holders, role) && name != "":
				line = fmt.Sprintf("- %s %q", role, name)
			}
			if line != "" {
				if len(words) == 0 || slices.ContainsFunc(words, func(w string) bool { return strings.Contains(strings.ToLower(line), w) }) {
					lines = append(lines, strings.Repeat("  ", depth)+line)
				}
				depth++
			}
			for _, c := range nd.ChildIDs {
				cn := byID[c]
				if cn == nil {
					continue
				}
				if (acts || role == "heading") && strings.ToLower(cn.Role.text()) == "statictext" {
					continue // the name says it already
				}
				walk(cn, depth, inside, within || acts)
			}
		}
		for i := range got.Nodes {
			if !child[got.Nodes[i].NodeID] {
				walk(&got.Nodes[i], 0, scopeID == 0, false)
			}
		}
	}
	if len(lines) == 0 {
		lines = []string{"(nothing on the page" + map[bool]string{true: " matches", false: ""}[len(words) > 0 || scopeID != 0] + ")"}
	}
	return lines, refs, nil
}
