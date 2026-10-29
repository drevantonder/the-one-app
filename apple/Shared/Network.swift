import Foundation

// Network finds a node that answers. Every node holds the whole network, so
// any one will do: this Mac's own, or one of the user's always-on devices.
enum Network {
    static var nodes: [URL] {
        #if os(macOS)
        ["http://127.0.0.1:4747", "http://mac:4747", "http://linux:4747"].compactMap(URL.init(string:))
        #else
        ["http://mac:4747", "http://linux:4747"].compactMap(URL.init(string:))
        #endif
    }

    static func node() async -> URL? {
        for node in nodes {
            let req = URLRequest(url: node.appendingPathComponent("healthz"), timeoutInterval: 3)
            if let (_, resp) = try? await URLSession.shared.data(for: req), (resp as? HTTPURLResponse)?.statusCode == 200 { return node }
        }
        return nil
    }

    static func client(_ name: String) async -> MCPClient? {
        await node().map { MCPClient(node: $0, name: name) }
    }
}
