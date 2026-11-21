//! What the harness's window says it has and where: an inspector request answered, and where a press goes.

use renderer::platform::Windows;
use serde_json::{Value, json};

/// What window `id` answers `request`, an inspector's, once what it set off has run.
pub(crate) fn ask(windows: &mut Windows, id: renderer::platform::WindowId, mut request: Value) -> Option<Value> {
    request["v"] = json!(renderer::inspect::VERSION);
    request["id"] = json!(0);
    let request = renderer::inspect::Request::parse(&request.to_string()).ok()?;
    let answered = std::rc::Rc::new(std::cell::RefCell::new(None));
    let to = answered.clone();
    windows.inspect(id, request, Box::new(move |answer| *to.borrow_mut() = Some(answer)));
    windows.wait();
    answered.borrow_mut().take()
}

/// Where a request's press goes: the middle of the node it selects, as the window last drew it, or its point.
pub(crate) fn point_of(windows: &mut Windows, id: renderer::platform::WindowId, request: &renderer::inspect::Request) -> anyhow::Result<(f32, f32)> {
    if !request.selects() {
        return match (request.f32("x"), request.f32("y")) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => anyhow::bail!("say where: a node, by a selector, or a point, x and y"),
        };
    }
    let mut query = json!({"op": "query"});
    query.as_object_mut().into_iter().for_each(|query| query.extend(request.selector_inputs()));
    let nth = request.inputs.get("nth").and_then(Value::as_u64).unwrap_or(0) as usize;
    // What isn't there yet, its page busy, is waited for, a second at most, as Playwright's click waits for its locator.
    let started = renderer::platform::fake_time();
    let node = loop {
        let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
        // Several that match, with no `nth` to say which, is a selector too loose, not the first of them.
        if let Some(Err(error)) = windows.with(id, |screen| screen.matched(request)).flatten() {
            return Err(error);
        }
        if let Some(node) = found["result"]["nodes"].get(nth).and_then(|node| node["node"].as_u64()) {
            break node;
        }
        if renderer::platform::fake_time() - started >= 1000.0 {
            anyhow::bail!("{}", windows.with(id, |screen| screen.unmatched(request)).unwrap_or_default());
        }
        windows.run(1000.0 / 60.0);
    };
    // Scrolled into sight first where it isn't, as a person scrolls to it, and Playwright's click does, then found
    // again as it's drawn now: what's pressed is what's there, though the page drew it afresh meanwhile.
    if windows.with(id, |screen| screen.reveal(node)).unwrap_or(false) {
        windows.run(0.0);
    }
    // Where it's drawn: what was drawn afresh and isn't yet is waited for, a second at most, as Playwright waits for
    // what it clicks to be there and still, drawn in the same place two frames running, and seen, as a person presses
    // what they see.
    let started = renderer::platform::fake_time();
    let mut was = Value::Null;
    let rect = loop {
        let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
        let seen = |node: &&Value| node["opacity"].as_f64().is_some_and(|opacity| opacity > 0.0);
        if let Some(rect) = found["result"]["nodes"].get(nth).filter(seen).map(|node| node["rect"].clone()).filter(Value::is_object) {
            if rect == was {
                break rect;
            }
            was = rect;
        }
        if renderer::platform::fake_time() - started >= 1000.0 {
            anyhow::bail!("what matches that isn't drawn");
        }
        windows.run(1000.0 / 60.0);
    };
    let at = |edge: &str| rect[edge].as_f64().unwrap_or_default() as f32;
    Ok((at("x") + at("width") / 2.0, at("y") + at("height") / 2.0))
}
