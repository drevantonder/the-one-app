import Foundation

enum MCPError: Error, LocalizedError {
    case rpc(String)
    case sessionExpired
    case badResponse

    var errorDescription: String? {
        switch self {
        case .rpc(let m): return m
        case .sessionExpired: return "the node forgot this session"
        case .badResponse: return "the node sent something unexpected"
        }
    }
}

// MCPClient speaks MCP's Streamable HTTP transport to a node.
actor MCPClient {
    let node: URL
    private let name: String
    private var endpoint: URL { node.appendingPathComponent("mcp") }
    private var session: String?
    private var connecting: Task<Void, Error>?
    private var nextID = 1

    init(node: URL, name: String) {
        self.node = node
        self.name = name
    }

    func request(_ method: String, _ params: [String: Any] = [:]) async throws -> [String: Any] {
        try await connect()
        do {
            return try await send(method, params)
        } catch MCPError.sessionExpired {
            session = nil
            try await connect()
            return try await send(method, params)
        }
    }

    private func connect() async throws {
        if session != nil { return }
        if let connecting { return try await connecting.value }
        let task = Task {
            _ = try await self.send("initialize", [
                "protocolVersion": "2025-06-18", "capabilities": [:] as [String: Any],
                "clientInfo": ["name": self.name, "version": "1"],
            ])
            _ = try await self.post(["jsonrpc": "2.0", "method": "notifications/initialized"])
        }
        connecting = task
        defer { connecting = nil }
        try await task.value
    }

    private func send(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        let id = nextID
        nextID += 1
        let (data, resp) = try await post(["jsonrpc": "2.0", "id": id, "method": method, "params": params])
        if let s = resp.value(forHTTPHeaderField: "Mcp-Session-Id") { session = s }
        if resp.statusCode == 404 && method != "initialize" { throw MCPError.sessionExpired }
        let messages: [[String: Any]]
        if resp.value(forHTTPHeaderField: "Content-Type")?.contains("text/event-stream") == true {
            messages = events(data)
        } else {
            messages = [(try? JSONSerialization.jsonObject(with: data)) as? [String: Any]].compactMap { $0 }
        }
        guard let reply = messages.first(where: { ($0["id"] as? Int) == id }) else { throw MCPError.badResponse }
        if let err = reply["error"] as? [String: Any] { throw MCPError.rpc(err["message"] as? String ?? "error") }
        return reply["result"] as? [String: Any] ?? [:]
    }

    private func post(_ body: [String: Any]) async throws -> (Data, HTTPURLResponse) {
        var req = URLRequest(url: endpoint, timeoutInterval: 60)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        req.setValue("2025-06-18", forHTTPHeaderField: "MCP-Protocol-Version")
        if let session { req.setValue(session, forHTTPHeaderField: "Mcp-Session-Id") }
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw MCPError.badResponse }
        return (data, http)
    }

    private func events(_ data: Data) -> [[String: Any]] {
        let text = String(decoding: data, as: UTF8.self).replacingOccurrences(of: "\r\n", with: "\n")
        return text.components(separatedBy: "\n\n").compactMap { event in
            let payload = event.split(separator: "\n")
                .filter { $0.hasPrefix("data:") }
                .map { $0.dropFirst(5).drop(while: { $0 == " " }) }
                .joined(separator: "\n")
            return (try? JSONSerialization.jsonObject(with: Data(payload.utf8))) as? [String: Any]
        }
    }
}
