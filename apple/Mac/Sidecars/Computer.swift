import AppKit
import ApplicationServices

/// Lets the person's own agents use this Mac (its apps, keyboard and pointer) once they turn it on, through Cua Driver
/// (github.com/trycua/cua). It runs as this app's child, embedded, so the Accessibility and Screen Recording
/// permissions it needs are this app's, granted once, rather than Cua Driver's own. Its tools act in the background
/// where they can, with an agent cursor of their own, so the person's pointer stays theirs. The node serves a sidecar
/// only to its Mac's local user, so only the owner's agents reach it. Computer Use lists it beside the agent devices.
struct ComputerApp: Sidecar {
    let app = "computer"

    var description: [String: Any] {
        ["title": "Computer Use", "icon": "cursor-click", "color": "#ff9500",
         "description": "Lets your own agents use this Mac's apps, keyboard and pointer, once you turn it on",
         "instructions": "Agents use this Mac through Computer Use, as a device named after it.",
         "ops": [
            op("_status", "Whether agents may use this Mac, its name, the permissions macOS gave, and the agent using it now."),
            op("_allow", "Turns letting agents use this Mac on or off. Turning it on asks the person at the Mac first.", ["on": flag("on or off")], required: ["on"]),
            op("_tools", "This Mac's tools, or one in full.", ["name": str("a tool's name")]),
            op("_use", "Runs one of this Mac's tools.", ["tool": str("the tool's name"), "args": ["type": "object", "description": "the tool's input"]], required: ["tool"]),
            op("_screen", "A small picture of this Mac's screen.", ["width": num("its width in pixels; default 640")]),
         ],
         "widgets": [Any](),
         "version": Bundle.main.object(forInfoDictionaryKey: "BuildCommit") as? String ?? "",
         "install": "Comes with the Mac app, on each Mac it's open on."]
    }

    func run(_ op: String, _ input: Input) async throws -> Any {
        let computer = await ComputerUse.shared
        switch op {
        case "_status":
            return json(await computer.status())
        case "_allow":
            if input.bool("on") == true {
                await computer.ask(from: input.caller("name"))
            } else {
                await computer.turnOff()
            }
            return json(await computer.status())
        case "_tools":
            try await computer.ready()
            return try await computer.driver.tools(input.string("name"))
        case "_use":
            let tool = try input.text("tool")
            guard !ComputerUse.withheld.contains(tool) else { throw Refusal("\(tool) isn't offered on a person's own Mac") }
            try await computer.ready()
            await computer.using(input.caller("agent") ?? input.caller("name") ?? "An agent")
            let args = try JSONSerialization.data(withJSONObject: input.value("args") as? [String: Any] ?? [:])
            let result = json(try await computer.driver.call(tool, args))
            let started = await computer.report()
            return object(["result": result, "started": started.map(iso), "title": await computer.title])
        case "_screen":
            try await computer.ready()
            let width = min(max(input.int("width", or: 640), 160), 1920)
            return json(try await computer.driver.call("get_desktop_state", Data(#"{"max_image_dimension":\#(width)}"#.utf8)))
        default:
            throw Refusal("no operation \(op)")
        }
    }

    private func json(_ data: Data) -> Any { (try? JSONSerialization.jsonObject(with: data)) ?? NSNull() }
}

/// Whether agents may use this Mac, and who is using it now. Off until the person turns it on, in the menu bar's
/// panel or from the Computer Use page; turning it off stops Cua Driver, so no call gets through after.
@MainActor @Observable
final class ComputerUse {
    static let shared = ComputerUse()

    /// Cua Driver's tools a person's own Mac doesn't offer: ones that change or update the driver itself, replay
    /// what it recorded, or force apps to quit, losing the person's work.
    nonisolated static let withheld: Set<String> = ["check_for_update", "install_extension", "install_ffmpeg", "set_config", "replay_trajectory", "kill_app"]

    private(set) var on = UserDefaults.standard.bool(forKey: "computerUse")
    /// The agent using this Mac now, and since when: from its first call until two minutes after its last; and
    /// whether a call's answer has said it started, so Computer Use logs it.
    private(set) var session: (agent: String, since: Date, reported: Bool)?
    private var last = Date.distantPast
    private var asking = false
    let driver = CuaDriver()
    private let indicator = Indicator()

    private init() {
        Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { _ in
            MainActor.assumeIsolated { ComputerUse.shared.expire() }
        }
    }

    var title: String { Host.current().localizedName ?? "This Mac" }

    func status() -> Data {
        let status = object(["on": on, "title": title, "kind": "mac", "asking": asking,
                "permissions": ["accessibility": AXIsProcessTrusted(), "screen_recording": CGPreflightScreenCaptureAccess()],
                "using": session.map { ["agent": $0.agent, "since": iso($0.since)] },
                "missing": CuaDriver.path == nil ? "Cua Driver isn't installed on this Mac: get it from github.com/trycua/cua, then turn this on again." : nil])
        return (try? JSONSerialization.data(withJSONObject: status)) ?? Data("{}".utf8)
    }

    /// Fails unless agents may use this Mac now. Screen capture is what an agent sees through, so both permissions
    /// are required: without Screen Recording, get_desktop_state can't produce a picture.
    func ready() throws {
        guard on else { throw Refusal("Agents can't use \(title): its owner hasn't let them. They can, in \(appName)'s menu on that Mac, or on the Computer Use page.") }
        guard CuaDriver.path != nil else { throw Refusal("Cua Driver isn't installed on \(title): get it from github.com/trycua/cua.") }
        guard AXIsProcessTrusted(), CGPreflightScreenCaptureAccess() else {
            throw Refusal("macOS hasn't let \(appName) use everything on \(title) yet: System Settings → Privacy & Security → Accessibility and Screen & System Audio Recording → \(appName) on.")
        }
    }

    /// Asked from another device, or by an agent: the person at this Mac says whether agents may use it. Nobody may
    /// be there, so it gives up after a minute, still off.
    func ask(from device: String?) {
        guard !on, !asking else { return }
        asking = true
        defer { asking = false }
        let alert = NSAlert()
        alert.messageText = "Let agents use this Mac?"
        alert.informativeText = "\(device.map { "\($0) asked to let" } ?? "Asked to let") your own agents use this Mac's apps, keyboard and pointer. You'll see in the menu bar whenever one does, and can stop it there."
        alert.addButton(withTitle: "Let Agents Use This Mac")
        alert.addButton(withTitle: "Don't Allow")
        NSApp.activate()
        let timeout = Timer(timeInterval: 60, repeats: false) { _ in MainActor.assumeIsolated { NSApp.abortModal() } }
        RunLoop.main.add(timeout, forMode: .modalPanel)
        let answer = alert.runModal()
        timeout.invalidate()
        if answer == .alertFirstButtonReturn { turnOn() }
    }

    /// The person turned it on: macOS asks them for what it needs, now, while they're at the Mac.
    func turnOn() {
        on = true
        UserDefaults.standard.set(true, forKey: "computerUse")
        if !AXIsProcessTrusted() {
            AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
        }
        if !CGPreflightScreenCaptureAccess() { CGRequestScreenCaptureAccess() }
    }

    func turnOff() {
        on = false
        UserDefaults.standard.set(false, forKey: "computerUse")
        session = nil
        indicator.hide()
        Task { await driver.stop() }
    }

    /// Notes a call by agent, showing it in the menu bar, before it acts.
    func using(_ agent: String) {
        last = .now
        if let session, session.agent == agent { return }
        session = (agent, .now, false)
        indicator.show(agent: agent, since: .now, stop: { ComputerUse.shared.turnOff() })
    }

    /// When the session started, if no answer has said so yet.
    func report() -> Date? {
        guard let s = session, !s.reported else { return nil }
        session?.reported = true
        return s.since
    }

    private func expire() {
        guard session != nil, last.distance(to: .now) > 120 else { return }
        session = nil
        indicator.hide()
    }
}

/// Shows in the menu bar while an agent uses this Mac: who, since when, and Stop, which turns it off.
@MainActor
private final class Indicator: NSObject {
    private var item: NSStatusItem?
    private var stop: (() -> Void)?

    func show(agent: String, since: Date, stop: @escaping () -> Void) {
        self.stop = stop
        let item = item ?? NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        self.item = item
        item.button?.image = NSImage(systemSymbolName: "cursorarrow.rays", accessibilityDescription: "\(agent) is using this Mac")
        item.button?.imagePosition = .imageLeading
        item.button?.title = " \(agent)"
        item.button?.contentTintColor = .systemOrange
        let menu = NSMenu()
        let who = NSMenuItem(title: "\(agent) is using this Mac", action: nil, keyEquivalent: "")
        who.isEnabled = false
        menu.addItem(who)
        let when = NSMenuItem(title: "Since \(since.formatted(date: .omitted, time: .shortened))", action: nil, keyEquivalent: "")
        when.isEnabled = false
        menu.addItem(when)
        menu.addItem(.separator())
        let button = NSMenuItem(title: "Stop Agents Using This Mac", action: #selector(stopped), keyEquivalent: "")
        button.target = self
        menu.addItem(button)
        item.menu = menu
    }

    func hide() {
        if let item { NSStatusBar.system.removeStatusItem(item) }
        item = nil
    }

    @objc private func stopped() { stop?() }
}

/// Cua Driver's MCP server, run as this app's child over stdio, started on the first call and kept until it's off.
actor CuaDriver {
    static let path: String? = {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return ["/Applications/CuaDriver.app/Contents/MacOS/cua-driver", "\(home)/Applications/CuaDriver.app/Contents/MacOS/cua-driver",
                "\(home)/.local/bin/cua-driver", "/opt/homebrew/bin/cua-driver"].first { FileManager.default.isExecutableFile(atPath: $0) }
    }()

    private var process: Process?
    private var input: FileHandle?
    private var next = 0
    private var waiting: [Int: CheckedContinuation<Data, Error>] = [:]
    private var listed: [[String: Any]]?

    /// Runs a tool with its input as JSON; its answer as JSON, MCP's `{ content, structuredContent }`.
    func call(_ tool: String, _ args: Data) async throws -> Data {
        let arguments = try JSONSerialization.jsonObject(with: args)
        let r = try await rpc("tools/call", ["name": tool, "arguments": arguments])
        if r["isError"] as? Bool == true {
            let texts = (r["content"] as? [[String: Any]] ?? []).compactMap { $0["text"] as? String }
            throw Refusal(texts.isEmpty ? "\(tool) failed" : texts.joined(separator: "\n"))
        }
        return try JSONSerialization.data(withJSONObject: r)
    }

    /// The tools, a line each, or one in full.
    func tools(_ name: String?) async throws -> String {
        if listed == nil { listed = try await rpc("tools/list", [:])["tools"] as? [[String: Any]] }
        let tools = (listed ?? []).filter { !ComputerUse.withheld.contains($0["name"] as? String ?? "") }
        if let name {
            guard let tool = tools.first(where: { $0["name"] as? String == name }) else { throw Refusal("no tool \(name)") }
            let schema = (try? JSONSerialization.data(withJSONObject: tool["inputSchema"] ?? [:], options: [.sortedKeys])).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
            return "\(tool["description"] as? String ?? "")\n\nInput: \(schema)"
        }
        return tools.map { tool in
            var gist = (tool["description"] as? String ?? "").components(separatedBy: "\n")[0]
            if let end = gist.range(of: ". ") { gist = String(gist[..<end.lowerBound]) + "." }
            return "\(tool["name"] as? String ?? ""): \(gist)"
        }.joined(separator: "\n")
    }

    func stop() {
        process?.terminate()
        process = nil
        input = nil
        for (_, c) in waiting { c.resume(throwing: Refusal("Its owner stopped agents using this Mac.")) }
        waiting = [:]
    }

    private func rpc(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        if process?.isRunning != true { try await start() }
        return try await send(method, params)
    }

    private func send(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        next += 1
        let id = next
        let line = try JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": id, "method": method, "params": params]) + Data("\n".utf8)
        let data = try await withCheckedThrowingContinuation { (c: CheckedContinuation<Data, Error>) in
            waiting[id] = c
            do { try input?.write(contentsOf: line) } catch {
                waiting[id] = nil
                c.resume(throwing: error)
            }
        }
        let reply = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        if let error = reply["error"] as? [String: Any] { throw Refusal(error["message"] as? String ?? "Cua Driver refused") }
        return reply["result"] as? [String: Any] ?? [:]
    }

    /// Starts it embedded: its permissions are this app's, and it never asks macOS for them itself (turning it on
    /// does). It may show macOS's one-time question about capturing the screen directly, which the person was asked
    /// to expect when they turned it on.
    private func start() async throws {
        guard let path = Self.path else { throw Refusal("Cua Driver isn't installed on this Mac: get it from github.com/trycua/cua.") }
        guard await ComputerUse.shared.on else { throw Refusal("Its owner stopped agents using this Mac.") }
        let process = Process()
        process.executableURL = URL(filePath: path)
        process.arguments = ["mcp", "--direct", "--embedded", "--host-bundle-id", Bundle.main.bundleIdentifier ?? ""]
        process.environment = ProcessInfo.processInfo.environment.merging(["CUA_DRIVER_EMBEDDED": "1", "CUA_DRIVER_ALLOW_CAPTURE_CONSENT_ALERT": "1"]) { $1 }
        let stdin = Pipe(), stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = FileHandle.nullDevice
        try process.run()
        self.process = process
        input = stdin.fileHandleForWriting
        listed = nil
        let reading = stdout.fileHandleForReading
        Task.detached { [weak self] in
            do {
                for try await line in reading.bytes.lines { await self?.received(Data(line.utf8)) }
            } catch {}
            await self?.ended(process)
        }
        _ = try await send("initialize", ["protocolVersion": "2025-06-18", "capabilities": [String: Any](), "clientInfo": ["name": appName, "version": "1"]])
        try input?.write(contentsOf: Data(#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.utf8) + Data("\n".utf8))
    }

    private func received(_ line: Data) {
        guard let message = try? JSONSerialization.jsonObject(with: line) as? [String: Any], let id = (message["id"] as? NSNumber)?.intValue,
              let c = waiting.removeValue(forKey: id) else { return }
        c.resume(returning: line)
    }

    private func ended(_ ended: Process) {
        guard ended === process else { return }
        process = nil
        for (_, c) in waiting { c.resume(throwing: Refusal("Cua Driver stopped on this Mac; try again.")) }
        waiting = [:]
    }
}
