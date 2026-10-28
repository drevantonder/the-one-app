// one-messages: the Mac side of the imessage plugin. Reads the Messages
// database (needs Full Disk Access) and sends through the Messages app.
// Usage: one-messages <command> < input.json   →   {"result": …} or {"error": "…"}
import Foundation
import SQLite3

func fail(_ msg: String) -> Never {
    FileHandle.standardOutput.write(try! JSONSerialization.data(withJSONObject: ["error": msg]))
    exit(1)
}

func done(_ result: Any) -> Never {
    FileHandle.standardOutput.write(try! JSONSerialization.data(withJSONObject: ["result": result], options: [.sortedKeys]))
    exit(0)
}

let iso = ISO8601DateFormatter()
iso.formatOptions = [.withInternetDateTime]
iso.timeZone = .current
let appleEpoch = 978_307_200.0

let fdaHelp = "Reading iMessages needs Full Disk Access for \(CommandLine.arguments[0]). Add it in System Settings → Privacy & Security → Full Disk Access (use + and press ⌘⇧G to paste the path)."

var db: OpaquePointer?
func open() {
    let path = NSHomeDirectory() + "/Library/Messages/chat.db"
    if sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) != SQLITE_OK { fail(fdaHelp) }
    if sqlite3_exec(db, "SELECT 1 FROM message LIMIT 1", nil, nil, nil) != SQLITE_OK { fail(fdaHelp) }
}

func query(_ sql: String, _ binds: [Any] = [], _ row: (OpaquePointer) -> Void) {
    var stmt: OpaquePointer?
    guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { fail(String(cString: sqlite3_errmsg(db))) }
    defer { sqlite3_finalize(stmt) }
    for (i, b) in binds.enumerated() {
        switch b {
        case let v as Int: sqlite3_bind_int64(stmt, Int32(i + 1), Int64(v))
        case let v as Int64: sqlite3_bind_int64(stmt, Int32(i + 1), v)
        case let v as Double: sqlite3_bind_double(stmt, Int32(i + 1), v)
        case let v as String: sqlite3_bind_text(stmt, Int32(i + 1), v, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
        default: sqlite3_bind_null(stmt, Int32(i + 1))
        }
    }
    while sqlite3_step(stmt) == SQLITE_ROW { row(stmt!) }
}

func text(_ s: OpaquePointer, _ col: Int32) -> String? {
    guard let c = sqlite3_column_text(s, col) else { return nil }
    return String(cString: c)
}

// Recent macOS versions keep a message's text in attributedBody, an
// archived NSAttributedString, rather than in the text column.
func body(_ s: OpaquePointer, textCol: Int32, blobCol: Int32) -> String {
    if let t = text(s, textCol), !t.isEmpty { return t }
    guard let bytes = sqlite3_column_blob(s, blobCol) else { return "" }
    let data = Data(bytes: bytes, count: Int(sqlite3_column_bytes(s, blobCol)))
    if let a = NSUnarchiver.unarchiveObject(with: data) as? NSAttributedString { return a.string }
    return ""
}

func date(_ raw: Int64) -> String {
    let seconds = raw > 1_000_000_000_000 ? Double(raw) / 1e9 : Double(raw)
    return iso.string(from: Date(timeIntervalSince1970: seconds + appleEpoch))
}

func since(_ v: Any?) -> Int64 {
    guard let s = v as? String, let d = iso.date(from: s) ?? ISO8601DateFormatter().date(from: s + "T00:00:00Z") else { return 0 }
    return Int64((d.timeIntervalSince1970 - appleEpoch) * 1e9)
}

let messageColumns = "m.ROWID, m.date, m.is_from_me, IFNULL(h.id, ''), m.text, m.attributedBody, IFNULL(c.chat_identifier, ''), IFNULL(c.display_name, '')"
let messageJoins = """
    FROM message m
    LEFT JOIN handle h ON h.ROWID = m.handle_id
    LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
    LEFT JOIN chat c ON c.ROWID = cmj.chat_id
    """

func messageRow(_ s: OpaquePointer) -> [String: Any] {
    let fromMe = sqlite3_column_int(s, 2) != 0
    var o: [String: Any] = ["date": date(sqlite3_column_int64(s, 1)), "from": fromMe ? "me" : (text(s, 3) ?? ""), "text": body(s, textCol: 4, blobCol: 5)]
    let chat = text(s, 6) ?? ""
    if !chat.isEmpty { o["chat"] = chat }
    if let name = text(s, 7), !name.isEmpty { o["chatName"] = name }
    return o
}

func send(to: String, message: String) {
    let esc = { (s: String) in s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") }
    let script = """
        tell application "Messages"
            set svc to 1st account whose service type = iMessage
            send "\(esc(message))" to participant "\(esc(to))" of svc
        end tell
        """
    var err: NSDictionary?
    NSAppleScript(source: script)?.executeAndReturnError(&err)
    if let err { fail("Messages couldn't send it: \(err[NSAppleScript.errorMessage] ?? err). macOS may need you to allow controlling Messages in System Settings → Privacy & Security → Automation.") }
}

let args = CommandLine.arguments
guard args.count == 2 else { fail("usage: one-messages <command> < input.json") }
let raw = FileHandle.standardInput.readDataToEndOfFile()
let input = (raw.isEmpty ? [:] : (try? JSONSerialization.jsonObject(with: raw)) as? [String: Any]) ?? [:]

switch args[1] {
case "messages.chats":
    open()
    var out: [[String: Any]] = []
    query("""
        SELECT c.ROWID, c.chat_identifier, IFNULL(c.display_name, ''), MAX(m.date),
            (SELECT group_concat(h.id, ', ') FROM chat_handle_join chj JOIN handle h ON h.ROWID = chj.handle_id WHERE chj.chat_id = c.ROWID)
        FROM chat c JOIN chat_message_join cmj ON cmj.chat_id = c.ROWID JOIN message m ON m.ROWID = cmj.message_id
        GROUP BY c.ROWID ORDER BY MAX(m.date) DESC LIMIT ?
        """, [input["limit"] as? Int ?? 30]) { s in
        var o: [String: Any] = ["chat": text(s, 1) ?? "", "last": date(sqlite3_column_int64(s, 3)), "participants": text(s, 4) ?? ""]
        if let n = text(s, 2), !n.isEmpty { o["name"] = n }
        out.append(o)
    }
    done(out)
case "messages.read":
    open()
    guard let chat = input["chat"] as? String, !chat.isEmpty else { fail("chat is required: a chat id, group name, or the other person's phone or email, as listed by chats") }
    var out: [[String: Any]] = []
    query("""
        SELECT \(messageColumns) \(messageJoins)
        WHERE (c.chat_identifier = ?1 OR c.display_name = ?1 OR h.id = ?1) AND m.date >= ?2
        ORDER BY m.date DESC LIMIT ?3
        """, [chat, since(input["since"]), input["limit"] as? Int ?? 50]) { s in out.append(messageRow(s)) }
    done(Array(out.reversed()))
case "messages.search":
    open()
    guard let q = (input["query"] as? String)?.lowercased(), !q.isEmpty else { fail("query is required") }
    let limit = input["limit"] as? Int ?? 50
    var out: [[String: Any]] = []
    query("""
        SELECT \(messageColumns) \(messageJoins)
        WHERE m.date >= ? AND (m.text LIKE ? OR m.attributedBody IS NOT NULL)
        ORDER BY m.date DESC LIMIT 20000
        """, [since(input["since"]), "%" + q + "%"]) { s in
        if out.count < limit, let row = Optional(messageRow(s)), (row["text"] as! String).lowercased().contains(q) { out.append(row) }
    }
    done(out)
case "messages.send":
    guard let to = input["to"] as? String, !to.isEmpty, let message = input["text"] as? String, !message.isEmpty else { fail("to and text are required") }
    send(to: to, message: message)
    done("Sent.")
default:
    fail("unknown command \(args[1])")
}
