//! seal <device's seal key, hex> <title> <body>: an APNs payload for `xcrun simctl push`, its note sealed to that device
//! and signed by a key made here (so `by` names no device of the install).
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let to: [u8; 32] = data_encoding::HEXLOWER.decode(a[1].as_bytes()).unwrap().try_into().unwrap();
    let note: one::push::Note = serde_json::from_value(serde_json::json!({ "id": "0123456789abcdef", "title": a[2], "body": a[3], "app": "board", "screen": "main" })).unwrap();
    let by = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let sealed = one::push::seal_note(&by, &to, &note, at).unwrap();
    let n = data_encoding::BASE64URL_NOPAD.encode(&sealed);
    println!("{}", serde_json::json!({ "aps": { "alert": { "title": "New notification", "body": "" }, "mutable-content": 1 }, "n": n }));
}
