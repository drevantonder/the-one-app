//! The inspector's protocol for this device's native windows, offered to the network as the Inspector app's tools, and whether a window holds what its person typed.

use super::*;

/// The Inspector app's tools of this device: its native windows, each by its id (`window`), as the network asks them
/// of the app's client (`client::DeviceTools`), answered on the native thread.
pub struct Inspector;

impl ::client::DeviceTools for Inspector {
    fn app(&self) -> &'static str {
        "inspector"
    }

    fn call<'a>(&'a self, tool: &'a str, arguments: Value) -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let native = match Native::get() {
                Ok(native) => native,
                Err(error) => return Ok(refused(&format!("no native window can open here: {error:#}"))),
            };
            if tool == "windows" {
                let (listed_to, listed) = tokio::sync::oneshot::channel();
                native.run(move |windows| drop(listed_to.send(windows.list())));
                let windows = json!({"device": ::client::name(), "windows": listed.await.unwrap_or_default()});
                return Ok(json!({"content": [{"type": "text", "text": windows.to_string()}], "structuredContent": windows}));
            }
            let mut request = match arguments {
                Value::Object(inputs) => inputs,
                _ => Default::default(),
            };
            let window = request.remove("window").and_then(|window| window.as_u64());
            request.remove("device");
            request.insert("v".into(), json!(renderer::inspect::VERSION));
            request.insert("id".into(), json!(1));
            request.insert("op".into(), json!(tool));
            let request = match renderer::inspect::Request::parse(&Value::Object(request).to_string()) {
                Ok(request) => request,
                Err(refused_with) => return Ok(refused(refused_with["error"].as_str().unwrap_or("that request isn't one"))),
            };
            let (answer_to, answer) = tokio::sync::oneshot::channel::<Value>();
            native.run(move |windows| {
                let open: Vec<WindowId> = windows.list().iter().filter_map(|listed| listed["window"].as_u64()).collect();
                let reply: renderer::Reply = Box::new(move |answered| drop(answer_to.send(answered)));
                match window.or_else(|| (open.len() == 1).then(|| open[0])) {
                    Some(id) => windows.inspect(id, request, reply),
                    None => reply(renderer::inspect::answer(&request.id, Err(anyhow::anyhow!("Say which native window, by the `window` that `windows` lists: none or more than one is open here.")))),
                }
            });
            let mut answer = answer.await.unwrap_or_else(|_| json!({"error": "the window closed"}));
            Ok(match answer.get("error") {
                Some(error) => refused(error.as_str().unwrap_or_default()),
                None if tool == "snap" => json!({
                    "content": [{"type": "image", "mimeType": "image/png", "data": answer["result"]["png"]}],
                    "structuredContent": {"width": answer["result"]["width"], "height": answer["result"]["height"]},
                }),
                // Each frame's picture is an image after what it cost, as a tool's answer with pictures has them.
                None if tool == "profile" || tool == "replay" => {
                    let mut result = answer["result"].take();
                    let pictures: Vec<Value> = result["frames"]
                        .as_array_mut()
                        .into_iter()
                        .flatten()
                        .filter_map(|frame| frame.as_object_mut()?.remove("png"))
                        .map(|png| json!({"type": "image", "mimeType": "image/png", "data": png}))
                        .collect();
                    let mut content = vec![json!({"type": "text", "text": result.to_string()})];
                    content.extend(pictures);
                    json!({"content": content, "structuredContent": result})
                }
                None => {
                    let result = answer["result"].take();
                    json!({"content": [{"type": "text", "text": result.to_string()}], "structuredContent": result})
                }
            })
        })
    }
}

/// A tool's result that says why it didn't.
pub(super) fn refused(why: &str) -> Value {
    json!({"isError": true, "content": [{"type": "text", "text": why}]})
}

/// Whether a native window holds something its person typed that its app hasn't taken (`Windows::typed`).
pub async fn typed() -> bool {
    let Some(native) = Native::started() else { return false };
    let (told, answer) = tokio::sync::oneshot::channel();
    native.run(move |windows| {
        let _ = told.send(windows.typed());
    });
    answer.await.unwrap_or(false)
}
