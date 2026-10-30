import Foundation
import SQLite3

/// iMessage: reads the Messages database (which needs Full Disk Access) and sends through the Messages app.
struct MessagesApp: Provider {
    let app = "imessage"

    var description: [String: Any] {
        let chat = str("the chat's name, a part of it only one chat has, a phone number or email address, or an id chats gives")
        let since = str("only messages after this: an ISO 8601 date or date-time")
        return describe(title: "iMessage", icon: "message-square", instructions:
            "iMessage is the person's Messages on their Mac, iMessage and SMS. imessage_chats lists recent chats; imessage_read reads one, newest last; " +
            "imessage_search finds messages. imessage_send sends a real message to a real person: send only what the user asked to send, to whom they asked. " +
            "Reading never marks a message read.", ops: [
            op("chats", "Recent chats, most recent first: who, when, the last message, and how many are unread.", [
                "unread": flag("only chats with unread messages"),
                "limit": num("at most this many; default 30"),
                "skip": num("skip this many of the most recent, to page back"),
            ]),
            op("read", "A chat's messages, newest last. Page back with skip.", [
                "chat": chat, "since": since,
                "limit": num("this many; default 30"),
                "skip": num("skip this many of the newest, to read further back"),
            ], required: ["chat"]),
            op("search", "Messages containing some text, across chats or in one, newest last.", [
                "query": str("text to look for, case-insensitive"), "chat": str("only in this chat"), "since": since,
                "limit": num("at most this many; default 30"),
                "skip": num("skip this many of the newest matches, to page back"),
            ], required: ["query"]),
            op("send", "Send an iMessage from the person's account, to a phone number, an email address or a chat. This sends a real message to a real person.", [
                "to": str("a phone number with its country code, an email address, or a chat as chats names it"),
                "text": str("the message"),
            ], required: ["to", "text"]),
        ])
    }

    func run(_ op: String, _ input: Input) async throws -> Any {
        if op == "send", let to = input.string("to"), to.contains("@") || isPhone(to) {
            try await tell("participant", to, input.text("text"))
            return "Sent to \(to)."
        }
        let db = try ChatDB()
        let names = ContactsApp.names()
        let chats = try db.chats(names)
        switch op {
        case "chats":
            let shown = input.bool("unread") == true ? chats.filter { $0.unread > 0 } : chats
            return shown.dropFirst(input.int("skip", or: 0)).prefix(input.int("limit", or: 30)).map { c in
                object([
                    "chat": c.name, "id": c.id, "with": c.people.count > 1 ? c.people : nil, "last": iso(c.last),
                    "from": c.from, "text": c.text, "unread": c.unread > 0 ? c.unread : nil,
                ])
            }
        case "read":
            let chat = try find(input.text("chat"), in: chats)
            return try db.messages(in: chat.rows, since: input.date("since")?.date, limit: input.int("limit", or: 30), skip: input.int("skip", or: 0))
                .map { $0.json(names, chat: nil) }
        case "search":
            let only = try input.string("chat").map { try find($0, in: chats).rows }
            let found = try db.messages(in: only, matching: input.text("query"), since: input.date("since")?.date,
                                        limit: input.int("limit", or: 30), skip: input.int("skip", or: 0))
            let byRow = Dictionary(chats.flatMap { c in c.rows.map { ($0, c.name) } }) { a, _ in a }
            return found.map { $0.json(names, chat: byRow[$0.chat]) }
        case "send":
            let chat = try find(input.text("to"), in: chats)
            try await tell("chat", chat.guid, input.text("text"))
            return "Sent to \(chat.name)."
        default:
            throw Refusal("imessage has no operation \(op)")
        }
    }

    private func isPhone(_ s: String) -> Bool {
        s.filter(\.isNumber).count >= 7 && s.allSatisfy { $0.isNumber || " +-().".contains($0) }
    }

    /// A chat by its id, a phone number or email address, its name, or a part of a name only one chat has.
    private func find(_ want: String, in chats: [Chat]) throws -> Chat {
        if let c = chats.first(where: { $0.id == want || $0.guid == want }) { return c }
        if want.contains("@") || isPhone(want) {
            let handle = ContactsApp.handle(want)
            if let c = chats.first(where: { $0.people.count <= 1 && ContactsApp.handle($0.id) == handle }) { return c }
            throw Refusal("no chat with \(want)")
        }
        if let c = chats.first(where: { $0.name.localizedCaseInsensitiveCompare(want) == .orderedSame }) { return c }
        let partial = chats.filter { $0.name.localizedCaseInsensitiveContains(want) }
        switch partial.count {
        case 1: return partial[0]
        case 0: throw Refusal("no chat called \(want)")
        default: throw Refusal("\(want) matches several chats: \(partial.prefix(10).map(\.name).joined(separator: "; "))")
        }
    }

    /// Asks Messages to send, through AppleScript, which macOS lets this app do once it's allowed to control Messages.
    private func tell(_ kind: String, _ target: String, _ text: String) async throws {
        let script = """
            on run {kind, target, message}
                tell application "Messages"
                    if kind is "chat" then
                        send message to chat id target
                    else
                        send message to participant target of (1st account whose service type = iMessage)
                    end if
                end tell
            end run
            """
        let p = Process()
        p.executableURL = URL(filePath: "/usr/bin/osascript")
        p.arguments = ["-", kind, target, text]
        let input = Pipe(), errors = Pipe()
        (p.standardInput, p.standardError, p.standardOutput) = (input, errors, FileHandle.nullDevice)
        try p.run()
        input.fileHandleForWriting.write(Data(script.utf8))
        try input.fileHandleForWriting.close()
        let deadline = Date.now.addingTimeInterval(30)
        while p.isRunning, Date.now < deadline { try await Task.sleep(for: .milliseconds(100)) }
        if p.isRunning {
            p.terminate()
            throw Refusal("Messages didn't answer. macOS may be asking on the Mac's screen whether \(appName) may control Messages: click Allow there, then try again.")
        }
        guard p.terminationStatus != 0 else { return }
        let error = String(decoding: errors.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        if error.contains("-1743") {
            throw Refusal("macOS doesn't let \(appName) control Messages. Allow it in System Settings → Privacy & Security → Automation: under \(appName), turn on Messages.")
        }
        throw Refusal("Messages couldn't send it: \(error)")
    }
}

/// A conversation: every chat Messages keeps with the same person or group (an iMessage one and an SMS one, say).
struct Chat {
    var rows: [Int64]
    var id: String
    var guid: String
    var name: String
    var people: [String]
    var last: Date
    var from: String
    var text: String
    var unread: Int
}

struct Message {
    var chat: Int64
    var time: Date
    var me: Bool
    var handle: String
    var text: String
    var attachments: String?

    func json(_ names: [String: String], chat: String?) -> [String: Any] {
        object([
            "time": iso(time), "from": me ? "me" : names[ContactsApp.handle(handle)] ?? handle, "text": text,
            "attachments": attachments, "chat": chat,
        ])
    }
}

/// ~/Library/Messages/chat.db, read-only.
final class ChatDB {
    private var db: OpaquePointer?

    init() throws {
        let path = NSHomeDirectory() + "/Library/Messages/chat.db"
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK,
              sqlite3_exec(db, "SELECT 1 FROM message LIMIT 1", nil, nil, nil) == SQLITE_OK else {
            sqlite3_close(db)
            db = nil
            let folder = Bundle.main.bundleURL.deletingLastPathComponent().path.replacingOccurrences(of: NSHomeDirectory(), with: "~")
            throw Refusal("Reading iMessages needs Full Disk Access. Allow it in System Settings → Privacy & Security → Full Disk Access: turn on \(appName), " +
                          "or if it isn't listed, add it with + from \(folder).")
        }
        // body(text, attributedBody): a message's text, wherever this macOS keeps it.
        sqlite3_create_function_v2(db, "body", 2, SQLITE_UTF8 | SQLITE_DETERMINISTIC, nil, { ctx, _, args in
            var text = args?[0].flatMap { sqlite3_value_text($0) }.map { String(cString: $0) } ?? ""
            if text.isEmpty, let blob = args?[1], let bytes = sqlite3_value_blob(blob) {
                text = archivedText([UInt8](UnsafeRawBufferPointer(start: bytes, count: Int(sqlite3_value_bytes(blob))))) ?? ""
            }
            text = text.replacingOccurrences(of: "\u{FFFC}", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
            sqlite3_result_text(ctx, text, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
        }, nil, nil, nil)
    }

    deinit { sqlite3_close(db) }

    /// Every conversation, most recent first.
    func chats(_ names: [String: String]) throws -> [Chat] {
        let rows = try query("""
            SELECT c.ROWID, c.guid, c.chat_identifier, IFNULL(c.display_name, ''),
                (SELECT group_concat(h.id, char(10)) FROM chat_handle_join chj JOIN handle h ON h.ROWID = chj.handle_id WHERE chj.chat_id = c.ROWID),
                m.date, m.is_from_me, IFNULL(h.id, ''), body(m.text, m.attributedBody),
                (SELECT COUNT(*) FROM chat_message_join j JOIN message u ON u.ROWID = j.message_id
                    WHERE j.chat_id = c.ROWID AND u.is_read = 0 AND u.is_from_me = 0 AND u.item_type = 0 AND u.associated_message_type = 0)
            FROM chat c
            JOIN message m ON m.ROWID = (SELECT MAX(message_id) FROM chat_message_join WHERE chat_id = c.ROWID)
            LEFT JOIN handle h ON h.ROWID = m.handle_id
            ORDER BY m.date DESC
            """)
        var chats: [Chat] = []
        var byID: [String: Int] = [:]
        for r in rows {
            let id = r[2] as? String ?? ""
            if let i = byID[id] {
                chats[i].rows.append(r[0] as! Int64)
                chats[i].unread += Int(r[9] as? Int64 ?? 0)
                continue
            }
            let people = (r[4] as? String ?? "").split(separator: "\n").map { names[ContactsApp.handle(String($0))] ?? String($0) }
            let title = (r[3] as? String)?.nonEmpty ?? people.joined(separator: ", ").nonEmpty ?? id
            let from = r[6] as? Int64 == 1 ? "me" : names[ContactsApp.handle(r[7] as? String ?? "")] ?? r[7] as? String ?? ""
            byID[id] = chats.count
            chats.append(Chat(rows: [r[0] as! Int64], id: id, guid: r[1] as? String ?? "", name: title, people: people,
                              last: time(r[5]), from: from, text: r[8] as? String ?? "", unread: Int(r[9] as? Int64 ?? 0)))
        }
        return chats
    }

    /// Messages in some chats (or all), containing some text (or any), newest last.
    func messages(in chats: [Int64]?, matching text: String? = nil, since: Date?, limit: Int, skip: Int) throws -> [Message] {
        var filters = ["m.item_type = 0", "m.associated_message_type = 0", "m.date >= ?"]
        var binds: [Any] = [Int64((since?.timeIntervalSinceReferenceDate ?? 0) * 1e9)]
        if let chats { filters.append("j.chat_id IN (\(chats.map(String.init).joined(separator: ",")))") }
        if let text {
            filters.append("body(m.text, m.attributedBody) LIKE ? ESCAPE '\\'")
            binds.append("%" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "%", with: "\\%").replacingOccurrences(of: "_", with: "\\_") + "%")
        }
        binds += [Int64(limit), Int64(skip)]
        let rows = try query("""
            SELECT j.chat_id, m.date, m.is_from_me, IFNULL(h.id, ''), body(m.text, m.attributedBody),
                (SELECT group_concat(a.transfer_name, ', ') FROM message_attachment_join maj JOIN attachment a ON a.ROWID = maj.attachment_id WHERE maj.message_id = m.ROWID)
            FROM chat_message_join j JOIN message m ON m.ROWID = j.message_id LEFT JOIN handle h ON h.ROWID = m.handle_id
            WHERE \(filters.joined(separator: " AND "))
            ORDER BY m.date DESC LIMIT ? OFFSET ?
            """, binds)
        return rows.reversed().compactMap { r in
            let m = Message(chat: r[0] as! Int64, time: time(r[1]), me: r[2] as? Int64 == 1, handle: r[3] as? String ?? "",
                            text: r[4] as? String ?? "", attachments: r[5] as? String)
            return m.text.isEmpty && m.attachments == nil ? nil : m
        }
    }

    /// Messages keeps time in nanoseconds since 2001 (seconds, on old Macs).
    private func time(_ value: Any?) -> Date {
        let raw = Double(value as? Int64 ?? 0)
        return Date(timeIntervalSinceReferenceDate: raw > 1e11 ? raw / 1e9 : raw)
    }

    private func query(_ sql: String, _ binds: [Any] = []) throws -> [[Any?]] {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { throw Refusal("reading Messages failed: \(String(cString: sqlite3_errmsg(db)))") }
        defer { sqlite3_finalize(stmt) }
        for (i, b) in binds.enumerated() {
            switch b {
            case let v as Int64: sqlite3_bind_int64(stmt, Int32(i + 1), v)
            case let v as String: sqlite3_bind_text(stmt, Int32(i + 1), v, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
            default: sqlite3_bind_null(stmt, Int32(i + 1))
            }
        }
        var rows: [[Any?]] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            rows.append((0..<sqlite3_column_count(stmt)).map { c -> Any? in
                switch sqlite3_column_type(stmt, c) {
                case SQLITE_INTEGER: sqlite3_column_int64(stmt, c)
                case SQLITE_TEXT: String(cString: sqlite3_column_text(stmt, c))
                default: nil
                }
            })
        }
        return rows
    }
}

/// The text of an attributedBody: an NSAttributedString archived as a typedstream, whose string is the
/// run of UTF-8 after the first NSString class name, marked "+" and prefixed with its length.
func archivedText(_ b: [UInt8]) -> String? {
    guard let name = b.firstRange(of: Array("NSString".utf8)),
          let plus = b[name.upperBound..<min(name.upperBound + 8, b.count)].firstIndex(of: 0x2B) else { return nil }
    var i = plus + 1
    func int(_ width: Int) -> Int? {
        guard i + width <= b.count else { return nil }
        defer { i += width }
        return (0..<width).reduce(0) { $0 | Int(b[i + $1]) << (8 * $1) }
    }
    let length = switch int(1) {
    case 0x81: int(2)
    case 0x82: int(4)
    case let n: n
    }
    guard let length, i + length <= b.count else { return nil }
    return String(decoding: b[i..<i + length], as: UTF8.self)
}
