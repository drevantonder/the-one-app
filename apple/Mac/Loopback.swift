import Darwin
import Foundation

/// Where the person's browser comes back to once they've signed in (RFC 8252's loopback redirect): a port on this
/// Mac, open only until it hears from the browser. Sign-in and consent happen in their own browser, where their
/// passkey is, and the auth service matches loopback redirects whatever the port. `windows/Auth.cs` does the same.
final class Loopback: @unchecked Sendable {
    struct Failed: LocalizedError {
        var errorDescription: String? { String(localized: "Couldn’t listen for the browser to come back. Try again.") }
    }

    private let fd: Int32
    /// Where the browser is sent back to.
    let redirect: String
    private let lock = NSLock()
    private var cancelled = false

    init() throws {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw Failed() }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = UInt32(0x7F00_0001).bigEndian
        address.sin_port = 0
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        var size = socklen_t(MemoryLayout<sockaddr_in>.size)
        let named = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &size) }
        }
        guard bound == 0, named == 0, listen(fd, 4) == 0 else {
            close(fd)
            throw Failed()
        }
        self.fd = fd
        redirect = "http://127.0.0.1:\(UInt16(bigEndian: address.sin_port))/callback"
    }

    /// Stops listening: a wait for the browser ends as cancelled.
    func stop() {
        lock.withLock { cancelled = true }
    }

    deinit { close(fd) }

    /// Waits for the browser to come back to `/callback` → the URL it came to, or nil if `stop` was called or
    /// `minutes` went by.
    func callback(appName: String, minutes: Int = 5) async -> URL? {
        let deadline = Date.now.addingTimeInterval(Double(minutes) * 60)
        return await Task.detached { [self] in
            while Date.now < deadline {
                if lock.withLock({ cancelled }) { return nil }
                var waiting = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
                guard poll(&waiting, 1, 250) > 0 else { continue }
                let client = accept(fd, nil, nil)
                guard client >= 0 else { continue }
                defer { close(client) }
                var timeout = timeval(tv_sec: 5, tv_usec: 0)
                setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
                var buffer = [UInt8](repeating: 0, count: 8192)
                let count = recv(client, &buffer, buffer.count, 0)
                guard count > 0 else { continue }
                let line = String(decoding: buffer[0..<count], as: UTF8.self).split(separator: "\r\n", maxSplits: 1, omittingEmptySubsequences: false).first ?? ""
                let parts = line.split(separator: " ")
                let target = parts.count > 1 ? String(parts[1]) : ""
                let found = target.hasPrefix("/callback?")
                let page = found ? "<!doctype html><meta charset=utf-8><title>\(Self.escaped(appName))</title><meta name=color-scheme content=\"light dark\"><body style=\"font:15px system-ui;display:grid;place-items:center;height:90vh;color:GrayText\"><p>You’re signed in. Go back to \(Self.escaped(appName)).</p>" : ""
                let body = Data(page.utf8)
                let head = "HTTP/1.1 \(found ? "200 OK" : "404 Not Found")\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: \(body.count)\r\nconnection: close\r\n\r\n"
                for part in [Data(head.utf8), body] {
                    part.withUnsafeBytes { _ = send(client, $0.baseAddress, part.count, 0) }
                }
                if found { return URL(string: "http://127.0.0.1\(target)") }
            }
            return nil
        }.value
    }

    private static func escaped(_ text: String) -> String {
        text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;")
    }
}
