//! The browser tools for a chrome device, answered here over its DevTools pipe as Cua's driver answers them for a Linux
//! device: the same input, and answers of the same shape (a line saying what happened, then its details as JSON). A
//! page is read from Chrome's accessibility tree: its text as an outline, with a ref (p3:12) on each thing to click,
//! type in or select. target_id is the device's Chrome, and tab_id one of its tabs; there are no windows, so neither pid
//! nor window_id is needed, and a session is taken but not needed either: the device is one agent's.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use super::chrome::{Browser, DialogOpened, live_browser, tab_id, target_of};
use super::kinds::Vm;
use super::{Devices, Input, or};

/// How many lines of a page one read gives; continuation reads on.
const READ_LINES: usize = 400;

type Details = Map<String, Value>;

/// A tool's answer: what happened, then its details.
fn answer(said: &str, details: &Details, more: Vec<Value>) -> Value {
    let mut items = vec![json!({ "type": "text", "text": said })];
    items.extend(more);
    items.push(json!({ "type": "text", "text": Value::Object(details.clone()).to_string() }));
    json!({ "content": items })
}

/// An answer that did nothing, and why, as Cua's driver refuses.
fn refused(code: &str, why: &str, details: Option<Details>) -> Value {
    let mut details = details.unwrap_or_default();
    details.insert("status".into(), json!("refused"));
    details.insert("refusal".into(), json!({ "code": code, "message": why }));
    answer(&format!("refused ({code}): {why}"), &details, Vec::new())
}

fn obj(v: Value) -> Details {
    match v {
        Value::Object(o) => o,
        _ => Map::new(),
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
}

/// A string quoted as Go's %q does, near enough: in double quotes, with what needs it escaped.
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Focuses a field, its text selected to be replaced, or the caret at its end.
const FOCUS_FIELD: &str = r#"function(replace) {
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
}"#;

/// Picks the one option of a select whose label (or value) is the one given.
const SELECT_OPTION: &str = r#"function(label, value) {
	if (this.tagName !== "SELECT") return { error: "not_select" };
	const all = [...this.options];
	const hit = all.filter((o) => label != null ? o.label.trim() === label.trim() || o.text.trim() === label.trim() : o.value === value);
	if (hit.length !== 1) return { error: hit.length ? "ambiguous" : "no_match", options: all.map((o) => o.text.trim()).slice(0, 60) };
	const was = this.value;
	hit[0].selected = true;
	this.dispatchEvent(new Event("input", { bubbles: true }));
	this.dispatchEvent(new Event("change", { bubbles: true }));
	return { changed: was !== this.value, label: hit[0].text.trim(), value: this.value };
}"#;

/// Roles a ref goes on: what's clicked, typed in or picked.
const ACTABLE: [&str; 27] = [
    "link",
    "button",
    "textbox",
    "searchbox",
    "combobox",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "option",
    "treeitem",
    "listbox",
    "menulistoption",
    "inputtime",
    "date",
    "datetime",
    "colorwell",
    "popupbutton",
    "comboboxselect",
    "comboboxmenubutton",
    "disclosuretriangle",
    "togglebutton",
];

/// Roles that show as a line of their own, holding what's in them, when named.
const HOLDERS: [&str; 24] = [
    "dialog",
    "alertdialog",
    "alert",
    "navigation",
    "main",
    "banner",
    "contentinfo",
    "form",
    "region",
    "group",
    "table",
    "list",
    "tablist",
    "menu",
    "menubar",
    "tree",
    "row",
    "cell",
    "columnheader",
    "rowheader",
    "listitem",
    "article",
    "complementary",
    "status",
];

/// An accessibility value as text.
fn ax_text(v: &Value) -> String {
    match &v["value"] {
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl Devices {
    pub(super) fn browse(&self, v: &mut Vm, op: &str, input: &Input) -> Result<Value> {
        self.awake(v)?;
        let b = live_browser(v)?;
        let t = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let number = |k: &str| input.get(k).and_then(Value::as_f64);
        let target = target_of(v);
        if !t("target_id").is_empty() && t("target_id") != target {
            return Ok(refused("browser_target_unknown", &format!("target_id {} isn't {}'s Chrome: it's {target}", t("target_id"), v.name), None));
        }
        if op == "browser_prepare" {
            let session = b.current_page()?;
            let tab = tab_id(&b.page(&session).map(|p| p.target).unwrap_or_default());
            let tabs = b.tabs().unwrap_or_default();
            let details = obj(json!({ "action": "attached", "prepared": true, "status": "ok", "target_id": target, "tab_id": tab, "tabs": tabs,
                "message": "Chrome is running: pass target_id and tab_id to the browser tools; get_browser_state reads the page." }));
            return Ok(answer(&format!("browser_prepare: ready — Chrome on {}, target_id {target}, tab_id {tab}", v.name), &details, Vec::new()));
        }
        let session = match b.page_of(&t("tab_id")) {
            Ok(s) => s,
            Err(err) => return Ok(refused("browser_tab_unknown", &format!("{err:#}"), Some(obj(json!({ "target_id": target }))))),
        };
        let page = b.page(&session).unwrap_or_default();
        let tab = tab_id(&page.target);
        let ids = obj(json!({ "target_id": target, "tab_id": tab }));
        let with = |m: Value| {
            let mut m = obj(m);
            m.extend(ids.clone());
            m
        };
        let open = page.dialog.clone();
        if let Some(d) = open.as_ref().filter(|_| op != "browser_dialog") {
            return Ok(refused("browser_dialog_open", &format!("a {} dialog is open in {tab} ({}): answer it with browser_dialog first", d.kind, q(&d.message)), Some(with(json!({ "dialog_id": d.id })))));
        }
        // The page's thing a ref names, or a refusal.
        let node = |r: &str| -> std::result::Result<i64, Value> {
            b.page(&session).and_then(|p| p.refs.get(r).copied()).ok_or_else(|| {
                refused("browser_ref_stale", &format!("ref {r} is stale — the page navigated or it isn't one of this tab's; re-run get_browser_state for fresh refs"), Some(with(json!({}))))
            })
        };
        let gone = |r: &str| refused("browser_ref_stale", &format!("ref {r} is gone from the page: re-run get_browser_state"), Some(with(json!({}))));
        let mode = or(&t("delivery_mode"), "background");
        match op {
            "get_browser_state" => self.state(&b, &session, &ids, input),
            "browser_navigate" => {
                let url = t("url").trim().to_string();
                if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("about:") {
                    bail!("browser_navigate only accepts http/https/about URLs, got: {url}");
                }
                b.navigate(&session, &url)?;
                Ok(answer(&format!("navigated {tab} to {url}"), &with(json!({ "refs_invalidated": true, "status": "ok", "url": url })), Vec::new()))
            }
            "browser_click" => {
                let r = t("ref");
                if or(&t("input_route"), "trusted") == "dom_event" {
                    if r.is_empty() {
                        return Ok(refused("browser_ref_required", "input_route dom_event needs ref", Some(with(json!({})))));
                    }
                    let id = match node(&r) {
                        Ok(id) => id,
                        Err(no) => return Ok(no),
                    };
                    // Clicked after this returns, so a dialog it opens doesn't hold the call.
                    if on(&b, &session, id, r#"function() { this.scrollIntoView({ block: "center", inline: "center" }); setTimeout(() => this.click(), 0); return true }"#, &[]).is_err() {
                        return Ok(gone(&r));
                    }
                    std::thread::sleep(Duration::from_millis(150));
                    return Ok(acted(&b, &session, &format!("dispatched DOM click on {r} in {tab}"), "dom", &mode, Ok(()), &ids));
                }
                let (x, y) = match point(&b, &session, &r, Some(input), &node) {
                    Ok(p) => p,
                    Err(no) => return Ok(no),
                };
                let done = b.click(&session, x, y, "left", 1);
                Ok(acted(&b, &session, &format!("clicked ({x:.0}, {y:.0}) in {tab}"), "trusted_input", &mode, done, &ids))
            }
            "browser_pointer" => {
                let action = t("action");
                let (mut x, mut y) = (0.0, 0.0);
                if action == "scroll" && t("ref").is_empty() && number("x").is_none() {
                    (x, y) = (640.0, 400.0);
                }
                if x == 0.0 && y == 0.0 {
                    (x, y) = match point(&b, &session, &t("ref"), Some(input), &node) {
                        Ok(p) => p,
                        Err(no) => return Ok(no),
                    };
                }
                let done = match action.as_str() {
                    "hover" => b.input(&session, "Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y })).map(|_| ()),
                    "right_click" => b.click(&session, x, y, "right", 1),
                    "double_click" => b.click(&session, x, y, "left", 2),
                    "scroll" => {
                        let (dx, dy) = (number("delta_x").unwrap_or(0.0), number("delta_y").unwrap_or(0.0));
                        b.input(&session, "Input.dispatchMouseEvent", json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy })).map(|_| ())
                    }
                    "drag" => {
                        let dest = t("destination_ref");
                        let (to_x, to_y) = if !dest.is_empty() {
                            match point(&b, &session, &dest, None, &node) {
                                Ok(p) => p,
                                Err(no) => return Ok(no),
                            }
                        } else {
                            match (number("to_x"), number("to_y")) {
                                (Some(x), Some(y)) => (x, y),
                                _ => bail!("drag to where? destination_ref, or to_x and to_y"),
                            }
                        };
                        b.drag(&session, x, y, to_x, to_y)
                    }
                    _ => bail!("action is hover, right_click, double_click, scroll or drag"),
                };
                Ok(acted(&b, &session, &format!("dispatched {action} at ({x:.0}, {y:.0}) in {tab}"), "trusted_input", &mode, done, &ids))
            }
            "browser_type" => {
                let r = t("ref");
                let id = match node(&r) {
                    Ok(id) => id,
                    Err(no) => return Ok(no),
                };
                let replace = input.get("replace").and_then(Value::as_bool).unwrap_or(false);
                if on(&b, &session, id, FOCUS_FIELD, &[json!(replace)]).is_err() {
                    return Ok(gone(&r));
                }
                let typed = t("text");
                let mut n = 0;
                let mut done: Result<()> = Ok(());
                if typed.is_empty() && replace {
                    done = b.key(&session, "backspace").map(|_| ());
                } else if t("mode") == "keystrokes" {
                    for ch in typed.chars() {
                        done = if ch == '\n' {
                            b.key(&session, "enter").map(|_| ())
                        } else {
                            let ch = ch.to_string();
                            b.input(&session, "Input.dispatchKeyEvent", json!({ "type": "keyDown", "key": ch, "text": ch, "unmodifiedText": ch }))
                                .and_then(|_| b.input(&session, "Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": ch })))
                                .map(|_| ())
                        };
                        if done.is_err() {
                            break;
                        }
                        n += 1;
                    }
                } else {
                    done = b.input(&session, "Input.insertText", json!({ "text": typed })).map(|_| ());
                    n = typed.chars().count();
                }
                let dialog = done.as_ref().err().is_some_and(|e| e.downcast_ref::<DialogOpened>().is_some());
                if !dialog {
                    done?;
                }
                let mut details = with(json!({ "delivery": { "delivered_count": n, "mode": "background" }, "effect": "unverifiable", "route": "trusted_input" }));
                let mut said = format!("typed {n} char(s) into {tab}");
                if dialog {
                    details.insert("effect".into(), json!("dialog_opened"));
                    details.insert("hint".into(), json!("A dialog opened: answer it with browser_dialog."));
                    said += "; a dialog opened: answer it with browser_dialog";
                } else if let Ok(now) = on(&b, &session, id, r#"function() { return this.type === "password" ? null : "value" in this ? String(this.value) : this.innerText }"#, &[])
                    && let Some(now) = now.as_str()
                {
                    details.insert("value".into(), json!(clip(now, 300)));
                    if now.contains(typed.trim_end_matches('\n')) {
                        details.insert("effect".into(), json!("confirmed"));
                    }
                }
                if details["effect"] == "unverifiable" {
                    details.insert("hint".into(), json!("The text was typed but not read back. Read the field with get_browser_state before typing again."));
                }
                details.insert("summary".into(), json!(said));
                Ok(answer(&said, &details, Vec::new()))
            }
            "browser_select" => {
                let r = t("ref");
                let id = match node(&r) {
                    Ok(id) => id,
                    Err(no) => return Ok(no),
                };
                let (label, value) = match (input.get("label").and_then(Value::as_str), input.get("value").and_then(Value::as_str)) {
                    (Some(l), _) => (json!(l), Value::Null),
                    (None, Some(v)) => (Value::Null, json!(v)),
                    _ => bail!("select which option? give label or value"),
                };
                let Ok(got) = on(&b, &session, id, SELECT_OPTION, &[label, value]) else { return Ok(gone(&r)) };
                let options: Vec<String> = got["options"].as_array().into_iter().flatten().filter_map(|o| o.as_str().map(str::to_string)).collect();
                match got["error"].as_str().unwrap_or_default() {
                    "not_select" => return Ok(refused("browser_select_not_select", &format!("ref {r} isn't a drop-down (a select): click it instead"), Some(with(json!({}))))),
                    e @ ("no_match" | "ambiguous") => {
                        return Ok(refused(&format!("browser_select_option_{e}"), &format!("not exactly one option matches; its options are: {}", options.join(", ")), Some(with(json!({ "options": options })))));
                    }
                    _ => {}
                }
                let label = got["label"].as_str().unwrap_or_default();
                Ok(answer(
                    &format!("selected {} in {tab} and dispatched input and change events", q(label)),
                    &with(json!({ "changed": got["changed"].as_bool().unwrap_or(false), "frame": "main", "label": label, "ref": r, "status": "ok", "value": got["value"].as_str().unwrap_or_default() })),
                    Vec::new(),
                ))
            }
            "browser_dialog" => match t("action").as_str() {
                "inspect" => Ok(match &open {
                    None => answer(&format!("no page-owned JavaScript dialog is open in {tab}"), &with(json!({ "present": false, "status": "ok" })), Vec::new()),
                    Some(d) => answer(
                        &format!("{} dialog is open in {tab}", d.kind),
                        &with(json!({ "present": true, "status": "ok", "dialog_id": d.id, "kind": d.kind, "message": d.message, "default_prompt": d.default })),
                        Vec::new(),
                    ),
                }),
                action @ ("accept" | "dismiss") => {
                    let Some(d) = &open else { return Ok(refused("browser_dialog_absent", &format!("no dialog is open in {tab}"), Some(with(json!({}))))) };
                    let asked = t("dialog_id");
                    if !asked.is_empty() && asked != d.id {
                        return Ok(refused("browser_dialog_stale", &format!("dialog {asked} is gone: the open one is {}", d.id), Some(with(json!({})))));
                    }
                    let accept = action == "accept";
                    let mut params = json!({ "accept": accept });
                    if let Some(s) = input.get("prompt_text").and_then(Value::as_str).filter(|_| accept) {
                        params["promptText"] = json!(s);
                    }
                    b.call(&session, "Page.handleJavaScriptDialog", Some(params))?;
                    if let Some(p) = b.now.lock().unwrap().pages.get_mut(&session) {
                        p.dialog = None;
                    }
                    Ok(answer(
                        &format!("{} {} dialog in {tab}", if accept { "accepted" } else { "dismissed" }, d.kind),
                        &with(json!({ "action": action, "dialog_id": d.id, "kind": d.kind, "status": "ok" })),
                        Vec::new(),
                    ))
                }
                _ => bail!("action is inspect, accept or dismiss"),
            },
            op => bail!("no browser tool {op}"),
        }
    }

    /// get_browser_state: the page's text as an outline, a ref on each thing to act on.
    fn state(&self, b: &Browser, session: &str, ids: &Details, input: &Input) -> Result<Value> {
        let t = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let format = or(&t("snapshot_format"), "dom_refs_v1");
        let page = b.page(session).unwrap_or_default();
        let info = b.call("", "Target.getTargetInfo", Some(json!({ "targetId": page.target }))).unwrap_or_default();
        let (url, title) = (info["targetInfo"]["url"].as_str().unwrap_or_default().to_string(), info["targetInfo"]["title"].as_str().unwrap_or_default().to_string());
        let tabs = b.tabs().unwrap_or_default();
        let mut details = obj(json!({ "mode": "snapshot", "status": "ok", "url": url, "title": title, "tabs": tabs }));
        details.extend(ids.clone());
        let (lines, refs, from, id) = match t("continuation") {
            c if !c.is_empty() => {
                let (id, at) = c.split_once(':').unwrap_or((&c, ""));
                if id != page.snapshot {
                    return Ok(refused("browser_continuation_stale", "that read was superseded or the page navigated: read it again", Some(details)));
                }
                (page.outline.clone(), None, at.parse().unwrap_or(0), id.to_string())
            }
            _ => {
                let id = {
                    let mut s = b.now.lock().unwrap();
                    s.snaps += 1;
                    format!("p{}", s.snaps)
                };
                let (lines, refs) = outline(b, session, &id, &t("scope_ref"), &t("query"))?;
                if let Some(p) = b.now.lock().unwrap().pages.get_mut(session) {
                    (p.snapshot, p.outline) = (id.clone(), lines.clone());
                }
                (lines, Some(refs), 0, id)
            }
        };
        details.insert("snapshot_id".into(), json!(id));
        let to = (from + READ_LINES).min(lines.len());
        let from = from.min(to);
        let shown = lines[from..to].join("\n");
        details.insert("truncated".into(), json!(to < lines.len()));
        let continuation = (to < lines.len()).then(|| format!("{id}:{to}"));
        if let Some(c) = &continuation {
            details.insert("continuation".into(), json!(c));
        }
        if format == "semantic_v2" {
            details.insert("snapshot".into(), json!({ "format": format, "id": id, "complete": to == lines.len(), "continuation": continuation }));
        }
        if let Some(refs) = refs {
            // Only the refs in what's shown.
            let shown_refs: Vec<Value> = refs.into_iter().filter(|r| shown.contains(&format!("[ref={}]", r["ref"].as_str().unwrap_or_default()))).collect();
            details.insert("refs".into(), json!(shown_refs));
        }
        let mut said = format!("snapshot {id} of {url} ({title}): {} ref(s)", shown.matches("[ref=").count());
        if from > 0 || to < lines.len() {
            said += &format!(", lines {}-{to} of {}", from + 1, lines.len());
        }
        if let Some(c) = &continuation {
            said += &format!("; continuation {} reads on", q(c));
        }
        said += &format!("\n{shown}");
        let mut shot = Vec::new();
        if input.get("include_screenshot") == Some(&json!(true))
            && let Ok(s) = b.call(session, "Page.captureScreenshot", Some(json!({ "format": "png" })))
            && let Some(data) = s["data"].as_str().filter(|d| !d.is_empty())
        {
            shot.push(json!({ "type": "image", "mimeType": "image/png", "data": data }));
            details.insert("screenshot".into(), json!({ "coordinate_space": "viewport_css_px", "mime_type": "image/png", "source": "cdp_tab" }));
        }
        Ok(answer(&said, &details, shot))
    }
}

/// The answer to input sent to a page: unverifiable, as a click is until the page is read again, or a dialog it opened.
fn acted(b: &Browser, session: &str, said: &str, route: &str, mode: &str, done: Result<()>, ids: &Details) -> Value {
    let mut details = obj(json!({ "delivery": { "mode": mode }, "effect": "unverifiable", "route": route,
        "hint": "The action was sent but nothing confirmed it. Read the page again with get_browser_state and check the change before repeating." }));
    let mut said = said.to_string();
    let dialog = b.page(session).is_some_and(|p| p.dialog.is_some()) || done.as_ref().err().is_some_and(|e| e.downcast_ref::<DialogOpened>().is_some());
    if dialog {
        details.insert("effect".into(), json!("dialog_opened"));
        details.insert("hint".into(), json!("A dialog opened: answer it with browser_dialog."));
        said += "; a dialog opened: answer it with browser_dialog";
    } else if let Err(err) = done {
        return refused("browser_input_failed", &format!("{err:#}"), Some(details));
    }
    details.extend(ids.clone());
    details.insert("summary".into(), json!(said));
    answer(&said, &details, Vec::new())
}

/// Runs a function on a node of the page, with this as the node: what it returns.
fn on(b: &Browser, session: &str, backend: i64, f: &str, args: &[Value]) -> Result<Value> {
    let obj = b.call(session, "DOM.resolveNode", Some(json!({ "backendNodeId": backend })))?;
    let args: Vec<Value> = args.iter().map(|a| json!({ "value": a })).collect();
    let r = b.call(session, "Runtime.callFunctionOn", Some(json!({ "objectId": obj["object"]["objectId"], "functionDeclaration": f, "arguments": args, "returnByValue": true })))?;
    if let Some(e) = r.get("exceptionDetails").filter(|e| !e.is_null()) {
        bail!("{}", e["text"].as_str().unwrap_or("it failed"));
    }
    Ok(r["result"]["value"].clone())
}

/// Where to act: the middle of the thing ref names, scrolled into view, or x and y.
fn point(b: &Browser, session: &str, r: &str, input: Option<&Input>, node: &dyn Fn(&str) -> std::result::Result<i64, Value>) -> std::result::Result<(f64, f64), Value> {
    if r.is_empty() {
        let n = |k: &str| input.and_then(|i| i.get(k)).and_then(Value::as_f64);
        return match (n("x"), n("y")) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => Err(refused("browser_target_required", "give ref, or x and y", None)),
        };
    }
    let id = node(r)?;
    let _ = b.call(session, "DOM.scrollIntoViewIfNeeded", Some(json!({ "backendNodeId": id })));
    let quads = b.call(session, "DOM.getContentQuads", Some(json!({ "backendNodeId": id }))).ok();
    let c: Vec<f64> = quads.as_ref().and_then(|q| q["quads"][0].as_array()).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    if c.len() < 8 {
        return Err(refused("browser_ref_not_visible", &format!("ref {r} isn't on the page's screen (hidden, or gone): re-run get_browser_state"), None));
    }
    Ok(((c[0] + c[2] + c[4] + c[6]) / 4.0, (c[1] + c[3] + c[5] + c[7]) / 4.0))
}

/// The page's accessibility tree, and its frames', as lines of text, with refs (named id:n) on what can be acted on,
/// which the page keeps until it navigates; from the node ref names only, and only lines matching query's words, when
/// given.
fn outline(b: &Browser, session: &str, id: &str, scope: &str, query: &str) -> Result<(Vec<String>, Vec<Value>)> {
    let tree = b.call(session, "Page.getFrameTree", None)?;
    let mut frames = Vec::new();
    fn walk_frames(f: &Value, out: &mut Vec<(String, String)>) {
        out.push((f["frame"]["id"].as_str().unwrap_or_default().into(), f["frame"]["url"].as_str().unwrap_or_default().into()));
        for c in f["childFrames"].as_array().into_iter().flatten() {
            walk_frames(c, out);
        }
    }
    walk_frames(&tree["frameTree"], &mut frames);
    let scope_id = if scope.is_empty() {
        0
    } else {
        match b.page(session).and_then(|p| p.refs.get(scope).copied()) {
            Some(id) => id,
            None => bail!("scope_ref {scope} is stale: read the page again"),
        }
    };
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut out = Outline { lines: Vec::new(), refs: Vec::new(), n: 0, new_refs: HashMap::new(), id: id.into(), words, scope_id };
    for (i, (frame_id, frame_url)) in frames.iter().enumerate() {
        let got = b.call(session, "Accessibility.getFullAXTree", Some(json!({ "frameId": frame_id })));
        let frame = if i == 0 { "main".to_string() } else { format!("frame{i}") };
        if i > 0 && scope_id == 0 && out.words.is_empty() {
            out.lines.push(format!("- frame {}", q(&clip(frame_url, 120))));
        }
        let got = match got {
            Ok(g) => g,
            Err(err) if i == 0 => return Err(err),
            Err(_) => continue, // another site's frame, in a process of its own
        };
        let nodes: Vec<Value> = got["nodes"].as_array().cloned().unwrap_or_default();
        let by_id: HashMap<&str, &Value> = nodes.iter().map(|n| (n["nodeId"].as_str().unwrap_or_default(), n)).collect();
        let children: std::collections::HashSet<&str> = nodes.iter().flat_map(|n| n["childIds"].as_array().into_iter().flatten().filter_map(Value::as_str)).collect();
        for n in nodes.iter().filter(|n| !children.contains(n["nodeId"].as_str().unwrap_or_default())) {
            out.walk(n, &by_id, 0, scope_id == 0, false, &frame);
        }
    }
    if let Some(p) = b.now.lock().unwrap().pages.get_mut(session) {
        p.refs.extend(out.new_refs);
    }
    let mut lines = out.lines;
    if lines.is_empty() {
        lines.push(format!("(nothing on the page{})", if !out.words.is_empty() || scope_id != 0 { " matches" } else { "" }));
    }
    Ok((lines, out.refs))
}

struct Outline {
    lines: Vec<String>,
    refs: Vec<Value>,
    n: u64,
    new_refs: HashMap<String, i64>,
    id: String,
    words: Vec<String>,
    scope_id: i64,
}

impl Outline {
    /// inside: within the node scope_ref names; within: within something acted on, whose inner editor isn't one.
    fn walk(&mut self, nd: &Value, by_id: &HashMap<&str, &Value>, mut depth: usize, mut inside: bool, within: bool, frame: &str) {
        let role = ax_text(&nd["role"]).to_lowercase();
        let name = clip(&ax_text(&nd["name"]), 200);
        let ignored = nd["ignored"] == true;
        let backend = nd["backendDOMNodeId"].as_i64().unwrap_or(0);
        let states: HashMap<String, String> = nd["properties"].as_array().into_iter().flatten().map(|p| (p["name"].as_str().unwrap_or_default().to_string(), ax_text(&p["value"]))).collect();
        let state = |k: &str| states.get(k).cloned().unwrap_or_default();
        let acts = !ignored
            && backend != 0
            && !(within && role == "generic")
            && (ACTABLE.contains(&role.as_str()) || !state("editable").is_empty() || (state("focusable") == "true" && !name.is_empty() && !["rootwebarea", "webarea", "iframe", "document", "scrollbar"].contains(&role.as_str())));
        if self.scope_id != 0 && backend == self.scope_id {
            inside = true;
        }
        let mut line = String::new();
        if ignored || !inside {
        } else if acts {
            self.n += 1;
            let r = format!("{}:{}", self.id, self.n);
            self.new_refs.insert(r.clone(), backend);
            line = format!("- {role}");
            if !name.is_empty() {
                line += &format!(" {}", q(&name));
            }
            let v = ax_text(&nd["value"]);
            if !v.is_empty() && v != name {
                line += &format!(" = {}", q(&clip(&v, 200)));
            }
            for s in ["checked", "pressed", "selected", "expanded", "disabled", "required", "readonly", "focused", "invalid"] {
                let v = state(s);
                if v == "true" || (s == "checked" && v == "mixed") {
                    line += &format!(" [{}]", if v == "mixed" { "mixed" } else { s });
                } else if v == "false" && ["checked", "expanded", "pressed"].contains(&s) {
                    line += &format!(" [not {s}]");
                }
            }
            line += &format!(" [ref={r}]");
            self.refs.push(json!({ "ref": r, "role": role, "name": name, "frame": frame }));
        } else if role == "statictext" {
            if !name.is_empty() {
                line = format!("- text {}", q(&name));
            }
        } else if role == "heading" {
            line = format!("- heading {}", q(&name));
            let l = state("level");
            if !l.is_empty() {
                line += &format!(" (level {l})");
            }
        } else if role == "image" || role == "img" {
            if !name.is_empty() {
                line = format!("- image {}", q(&name));
            }
        } else if HOLDERS.contains(&role.as_str()) && !name.is_empty() {
            line = format!("- {role} {}", q(&name));
        }
        if !line.is_empty() {
            if self.words.is_empty() || self.words.iter().any(|w| line.to_lowercase().contains(w)) {
                self.lines.push(format!("{}{line}", "  ".repeat(depth)));
            }
            depth += 1;
        }
        for c in nd["childIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let Some(cn) = by_id.get(c) else { continue };
            if (acts || role == "heading") && ax_text(&cn["role"]).to_lowercase() == "statictext" {
                continue; // the name says it already
            }
            self.walk(cn, by_id, depth, inside, within || acts, frame);
        }
    }
}
