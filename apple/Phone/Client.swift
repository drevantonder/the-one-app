import Foundation
import OneClient

/// The client library (client/one_client.h): a phone keeps no log, so a member device that does acts for it.
enum Client {
    /// Starts the client with its key and the devices it knows in `dir`. Whether it has joined a space yet.
    static func start(in dir: URL) async throws -> Bool {
        let path = dir.path(percentEncoded: false)
        let reply = try await run { path.withCString { one_start($0, "") } }
        struct Started: Decodable { var joined: Bool }
        return try JSONDecoder().decode(Started.self, from: reply).joined
    }

    /// Joins the space a pairing code is for, calling this device `name` there.
    static func join(_ code: String, name: String) async throws {
        _ = try await run { code.withCString { c in name.withCString { one_join(c, $0) } } }
    }

    /// Runs one blocking call on a thread of its own, never the main one or Swift's pool, and unwraps its `ok`.
    fileprivate static func run(_ call: @escaping @Sendable () -> UnsafeMutablePointer<CChar>?) async throws -> Data {
        let text: String = await withCheckedContinuation { done in
            DispatchQueue.global(qos: .userInitiated).async {
                guard let raw = call() else { return done.resume(returning: #"{"error":"no answer"}"#) }
                let text = String(cString: raw)
                one_free(raw)
                done.resume(returning: text)
            }
        }
        let reply = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] ?? [:]
        if let error = reply["error"] as? String { throw NodeError(error) }
        return try JSONSerialization.data(withJSONObject: reply["ok"] ?? NSNull(), options: .fragmentsAllowed)
    }
}

/// The local API, asked of a member device over iroh.
struct ClientTransport: NodeTransport {
    var unreachable: String { String(localized: "None of the space’s devices answered.") }

    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data {
        let path = "/" + path
        let body = body.map { String(decoding: $0, as: UTF8.self) } ?? ""
        return try await Client.run {
            method.withCString { m in path.withCString { p in body.withCString { one_request(m, p, $0) } } }
        }
    }

    func page(app: String, screen: String, params: String) async throws -> (html: String, base: URL?) {
        let params = (try? JSONSerialization.jsonObject(with: Data(params.utf8))) ?? [String: Any]()
        let body = try JSONSerialization.data(withJSONObject: ["app": app, "screen": screen, "params": params])
        let reply = try JSONSerialization.jsonObject(with: try await send("GET", "v1/screen", body: body, timeout: 15)) as? [String: Any]
        return (reply?["html"] as? String ?? "", nil)
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                while !Task.isCancelled {
                    do {
                        let data = try await Client.run { one_next_event(20_000) }
                        if let event = try? JSONDecoder().decode(NodeEvent.self, from: data) { continuation.yield(event) }
                    } catch {
                        continuation.finish(throwing: error)
                        return
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
