// Computer Use's helper on a Mac: a small app of its own, so that macOS asks the Mac's person for Screen Recording and
// Accessibility under its name and keeps what they allow it, through the server's updates. The app's service builds
// it and runs it once for each call, `helper <command> [<inputs as JSON>]`, and it prints one line, `{"ok": ...}`,
// `{"error": "..."}`, or `{"missing": "<the permission>"}` when macOS doesn't let it do that yet. A fresh process
// each time reads what macOS allows now, which a process that kept running wouldn't for Screen Recording.
//
// macOS asks for a permission, and keeps the answer, under the program it holds responsible. A program a server
// starts would be the server's, `one`, which is replaced by each update. So the helper first starts itself again as
// a process macOS holds responsible for itself, the way macOS's own Terminal starts a shell.
//
// The screen is the main display (the one with the menu bar), in a picture no bigger than a model sees unshrunk,
// and x and y are that picture's pixels.
import ApplicationServices
import Carbon.HIToolbox
import Foundation
import ImageIO
import ScreenCaptureKit

/// Starts this program again with the same arguments, macOS holding it responsible for itself, and gives its exit
/// status. `responsibility_spawnattrs_setdisclaim` is how Terminal and Xcode do this; it isn't in macOS's headers,
/// so it's looked up.
func startedAsItself() -> Int32 {
    typealias Disclaim = @convention(c) (UnsafeMutablePointer<posix_spawnattr_t?>, Int32) -> Int32
    guard let found = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "responsibility_spawnattrs_setdisclaim") else {
        answer(["error": "This macOS has no responsibility_spawnattrs_setdisclaim, so the helper can't ask for permissions under its own name"])
        return 1
    }
    var attributes: posix_spawnattr_t?
    posix_spawnattr_init(&attributes)
    defer { posix_spawnattr_destroy(&attributes) }
    _ = unsafeBitCast(found, to: Disclaim.self)(&attributes, 1)
    setenv("ONE_HELPER_OWN", "1", 1)
    let path = Bundle.main.executablePath ?? CommandLine.arguments[0]
    let argv: [UnsafeMutablePointer<CChar>?] = [strdup(path)] + CommandLine.arguments.dropFirst().map { strdup($0) } + [nil]
    var pid = pid_t()
    let started = posix_spawn(&pid, path, nil, &attributes, argv, environ)
    guard started == 0 else {
        answer(["error": "The helper couldn't start itself: \(String(cString: strerror(started)))"])
        return 1
    }
    var status: Int32 = 0
    while waitpid(pid, &status, 0) == -1 && errno == EINTR {}
    return (status & 0x7f) == 0 ? (status >> 8) & 0xff : 1
}

func answer(_ fields: [String: Any]) {
    let data = (try? JSONSerialization.data(withJSONObject: fields)) ?? Data(#"{"error": "its answer isn't JSON"}"#.utf8)
    FileHandle.standardOutput.write(data + Data("\n".utf8))
}

struct Refusal: Error { let why: String }
struct Missing: Error { let permission: String }

/// The main display, and the size of its picture.
struct Display {
    let id: CGDirectDisplayID
    let bounds: CGRect
    let width: Int
    let height: Int

    init() {
        id = CGMainDisplayID()
        bounds = CGDisplayBounds(id)
        // A model is shown a picture whole up to 1568 pixels on its long side and about 1.15 megapixels (Anthropic's
        // vision limits); a bigger one is shrunk before it's seen, and the x and y it answers then miss. So the
        // display, in points, is scaled down to fit, never up.
        let scale = min(1, 1568 / max(bounds.width, bounds.height), (1_150_000 / (bounds.width * bounds.height)).squareRoot())
        width = Int((bounds.width * scale).rounded())
        height = Int((bounds.height * scale).rounded())
    }

    /// A pixel of the picture, as the point on the display that events are posted at.
    func point(_ inputs: [String: Any], _ x: String = "x", _ y: String = "y") throws -> CGPoint {
        guard let x = (inputs[x] as? NSNumber)?.doubleValue, let y = (inputs[y] as? NSNumber)?.doubleValue else {
            throw Refusal(why: "give x and y")
        }
        guard x >= 0, y >= 0, x < Double(width), y < Double(height) else {
            throw Refusal(why: "\(Int(x)), \(Int(y)) is off the screen, which is \(width)×\(height)")
        }
        return CGPoint(x: bounds.minX + (x + 0.5) * bounds.width / Double(width), y: bounds.minY + (y + 0.5) * bounds.height / Double(height))
    }
}

func screenRecording() -> Bool { CGPreflightScreenCaptureAccess() }
func accessibility() -> Bool { AXIsProcessTrusted() }

func screenshot(_ display: Display) async throws -> String {
    guard screenRecording() else { throw Missing(permission: "Screen Recording") }
    let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
    guard let shown = content.displays.first(where: { $0.displayID == display.id }) else { throw Refusal(why: "The Mac's display isn't there to capture") }
    let configuration = SCStreamConfiguration()
    configuration.width = display.width
    configuration.height = display.height
    configuration.showsCursor = true
    let image = try await SCScreenshotManager.captureImage(contentFilter: SCContentFilter(display: shown, excludingWindows: []), configuration: configuration)
    let png = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(png, "public.png" as CFString, 1, nil) else { throw Refusal(why: "couldn't make a PNG") }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else { throw Refusal(why: "couldn't make a PNG") }
    return (png as Data).base64EncodedString()
}

// Events are posted as the hardware's are, a little apart, as a hand would.
let source = CGEventSource(stateID: .hidSystemState)

func pause(_ milliseconds: UInt32 = 20) { usleep(milliseconds * 1000) }

func mouse(_ type: CGEventType, _ at: CGPoint, _ button: CGMouseButton = .left, clicks: Int64 = 1) {
    let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: at, mouseButton: button)!
    event.setIntegerValueField(.mouseEventClickState, value: clicks)
    event.post(tap: .cghidEventTap)
}

func click(_ at: CGPoint, _ button: String, clicks: Int64) throws {
    let (down, up, which): (CGEventType, CGEventType, CGMouseButton) =
        switch button {
        case "left": (.leftMouseDown, .leftMouseUp, .left)
        case "right": (.rightMouseDown, .rightMouseUp, .right)
        case "middle": (.otherMouseDown, .otherMouseUp, .center)
        default: throw Refusal(why: "button is left, right or middle")
        }
    mouse(.mouseMoved, at)
    pause(40)
    for click in 1...clicks {
        mouse(down, at, which, clicks: click)
        pause()
        mouse(up, at, which, clicks: click)
        pause(60)
    }
}

// macOS's virtual key codes (kVK_ANSI_*, kVK_*) for what xdotool names a key, with whether Shift makes it.
let keys: [String: (CGKeyCode, Bool)] = {
    var keys: [String: (CGKeyCode, Bool)] = [:]
    let letters: [Character: CGKeyCode] = [
        "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13,
        "e": 14, "r": 15, "y": 16, "t": 17, "o": 31, "u": 32, "i": 34, "p": 35, "l": 37, "j": 38, "k": 40, "n": 45, "m": 46,
    ]
    for (letter, code) in letters {
        keys[String(letter)] = (code, false)
        keys[letter.uppercased()] = (code, true)
    }
    let digits: [CGKeyCode] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25]
    let shifted = [")", "!", "@", "#", "$", "%", "^", "&", "*", "("]
    let shiftedNames = ["parenright", "exclam", "at", "numbersign", "dollar", "percent", "asciicircum", "ampersand", "asterisk", "parenleft"]
    for (digit, code) in digits.enumerated() {
        keys[String(digit)] = (code, false)
        keys[shifted[digit]] = (code, true)
        keys[shiftedNames[digit]] = (code, true)
    }
    // Punctuation, by xdotool's name and by itself, then what Shift makes of it.
    let punctuation: [(String, String, CGKeyCode, String, String)] = [
        ("minus", "-", 27, "underscore", "_"), ("equal", "=", 24, "plus", "+"),
        ("bracketleft", "[", 33, "braceleft", "{"), ("bracketright", "]", 30, "braceright", "}"),
        ("backslash", "\\", 42, "bar", "|"), ("semicolon", ";", 41, "colon", ":"),
        ("apostrophe", "'", 39, "quotedbl", "\""), ("comma", ",", 43, "less", "<"),
        ("period", ".", 47, "greater", ">"), ("slash", "/", 44, "question", "?"), ("grave", "`", 50, "asciitilde", "~"),
    ]
    for (name, character, code, shiftedName, shiftedCharacter) in punctuation {
        keys[name] = (code, false)
        keys[character] = (code, false)
        keys[shiftedName] = (code, true)
        keys[shiftedCharacter] = (code, true)
    }
    let named: [String: CGKeyCode] = [
        "return": 36, "enter": 36, "kp_enter": 76, "tab": 48, "iso_left_tab": 48, "space": 49, "backspace": 51,
        "delete": 117, "escape": 53, "esc": 53, "home": 115, "end": 119, "prior": 116, "page_up": 116, "pageup": 116,
        "next": 121, "page_down": 121, "pagedown": 121, "left": 123, "right": 124, "down": 125, "up": 126, "insert": 114,
        "help": 114, "caps_lock": 57, "f1": 122, "f2": 120, "f3": 99, "f4": 118, "f5": 96, "f6": 97, "f7": 98, "f8": 100,
        "f9": 101, "f10": 109, "f11": 103, "f12": 111,
    ]
    for (name, code) in named { keys[name] = (code, name == "iso_left_tab") }
    return keys
}()

/// A key by its name: as xdotool names it, or a character, case mattering only for a letter.
func key(_ name: String) -> (CGKeyCode, Bool)? { keys[name] ?? keys[name.lowercased()] }

enum Modifier { case command, control, option, shift }

let modifiers: [String: Modifier] = [
    "super": .command, "super_l": .command, "super_r": .command, "meta": .command, "meta_l": .command, "meta_r": .command,
    "cmd": .command, "command": .command, "control": .control, "alt": .option, "alt_l": .option, "alt_r": .option,
    "option": .option, "shift": .shift, "shift_l": .shift, "shift_r": .shift,
]

let modifierKeys: [Modifier: (CGKeyCode, CGEventFlags)] = [
    .command: (55, .maskCommand), .control: (59, .maskControl), .option: (58, .maskAlternate), .shift: (56, .maskShift),
]

/// Whether a key types a character (a letter, a digit or punctuation): ctrl with one is a shortcut, which is ⌘ on a Mac.
func character(_ code: CGKeyCode) -> Bool { code <= 50 && ![36, 48, 49].contains(code) }

/// One press, such as `ctrl+shift+t`: its modifiers held, in order, around its last key. `ctrl` with a character is
/// ⌘, as a shortcut written for Linux or Windows means on a Mac (ctrl+c copies), and with anything else ⌃ (ctrl+Tab);
/// `control` is always ⌃, and `super`, `cmd` and `meta` ⌘.
func press(_ combo: String) throws {
    let names = combo.split(separator: "+").map(String.init)
    guard let last = names.last else { throw Refusal(why: "There's no key in \(combo): + is plus") }
    let ctrl = ["ctrl", "control_l", "control_r"]
    var code: CGKeyCode
    var shift = false
    if let (found, shifted) = key(last) {
        (code, shift) = (found, shifted)
    } else if let modifier = modifiers[last.lowercased()] ?? (ctrl.contains(last.lowercased()) ? .control : nil) {
        code = modifierKeys[modifier]!.0
    } else {
        throw Refusal(why: "There's no key called \(last): name keys as xdotool does (Return, ctrl+l, BackSpace, F5)")
    }
    var held: [Modifier] = []
    for name in names.dropLast() {
        if ctrl.contains(name.lowercased()) {
            held.append(character(code) ? .command : .control)
        } else if let modifier = modifiers[name.lowercased()] {
            held.append(modifier)
        } else {
            throw Refusal(why: "There's no modifier called \(name): ctrl, shift, alt, super (cmd) or control")
        }
    }
    if shift && !held.contains(.shift) { held.append(.shift) }
    var flags = CGEventFlags()
    for modifier in held {
        flags.insert(modifierKeys[modifier]!.1)
        keyEvent(modifierKeys[modifier]!.0, down: true, flags)
    }
    keyEvent(code, down: true, flags)
    keyEvent(code, down: false, flags)
    for modifier in held.reversed() {
        flags.remove(modifierKeys[modifier]!.1)
        keyEvent(modifierKeys[modifier]!.0, down: false, flags)
    }
}

func keyEvent(_ code: CGKeyCode, down: Bool, _ flags: CGEventFlags) {
    let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)!
    event.flags = flags
    event.post(tap: .cghidEventTap)
    pause(down ? 15 : 30)
}

/// Text, a character at a time, as the characters themselves: a line break is Return and a tab Tab.
func type(_ text: String) throws {
    // macOS turns on secure input while a password field has the focus.
    if IsSecureEventInputEnabled() {
        throw Refusal(why: "A password field has the focus on the Mac. Its person types passwords and codes themselves.")
    }
    for character in text {
        if character == "\n" || character == "\r\n" { keyEvent(36, down: true, []); keyEvent(36, down: false, []); continue }
        if character == "\t" { keyEvent(48, down: true, []); keyEvent(48, down: false, []); continue }
        let units = Array(String(character).utf16)
        for down in [true, false] {
            let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)!
            event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
            event.post(tap: .cghidEventTap)
        }
        pause(8)
    }
}

func act(_ display: Display, _ inputs: [String: Any]) throws -> Bool {
    guard accessibility() else { throw Missing(permission: "Accessibility") }
    switch inputs["action"] as? String {
    case "click":
        try click(display.point(inputs), inputs["button"] as? String ?? "left", clicks: 1)
    case "double_click":
        try click(display.point(inputs), "left", clicks: 2)
    case "move":
        mouse(.mouseMoved, try display.point(inputs))
    case "drag":
        guard let path = inputs["path"] as? [[String: Any]], path.count >= 2 else { throw Refusal(why: "give a path of at least two points") }
        let points = try path.map { try display.point($0) }
        mouse(.mouseMoved, points[0])
        pause(40)
        mouse(.leftMouseDown, points[0])
        // Between each two points it moves in steps, as a hand would, so what's dragged follows.
        for (from, to) in zip(points, points.dropFirst()) {
            for step in 1...10 {
                let t = Double(step) / 10
                mouse(.leftMouseDragged, CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t))
                pause(10)
            }
        }
        pause(40)
        mouse(.leftMouseUp, points.last!)
    case "scroll":
        let at = try display.point(inputs)
        let amount = (inputs["amount"] as? NSNumber)?.intValue ?? 5
        // A step of the wheel is 50 points: about three lines of text.
        let (dy, dx): (Int32, Int32) =
            switch inputs["direction"] as? String {
            case "up": (50, 0)
            case "down": (-50, 0)
            case "left": (0, 50)
            case "right": (0, -50)
            default: throw Refusal(why: "direction is up, down, left or right")
            }
        mouse(.mouseMoved, at)
        pause(40)
        for _ in 0..<amount {
            CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 2, wheel1: dy, wheel2: dx, wheel3: 0)!.post(tap: .cghidEventTap)
            pause(30)
        }
    case "type":
        try type(inputs["text"] as? String ?? "")
    case "key":
        for combo in (inputs["keys"] as? String ?? "").split(separator: " ") {
            try press(String(combo))
            pause(50)
        }
    case let other:
        throw Refusal(why: "There's no action \(other ?? "")")
    }
    return true
}

if ProcessInfo.processInfo.environment["ONE_HELPER_OWN"] == nil {
    exit(startedAsItself())
}

let arguments = CommandLine.arguments
let display = Display()
do {
    let inputs = arguments.count > 2 ? (try JSONSerialization.jsonObject(with: Data(arguments[2].utf8)) as? [String: Any] ?? [:]) : [:]
    switch arguments.count > 1 ? arguments[1] : "" {
    case "status":
        answer(["ok": ["width": display.width, "height": display.height, "screen_recording": screenRecording(), "accessibility": accessibility()]])
    case "screenshot":
        answer(["ok": try await screenshot(display)])
    case "act":
        answer(["ok": try act(display, inputs)])
    // Asks macOS for a permission, which it does on the Mac's screen, adding this app to its list in System Settings.
    case "ask":
        if inputs["permission"] as? String == "Screen Recording" {
            _ = CGRequestScreenCaptureAccess()
        } else {
            _ = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue(): true] as CFDictionary)
        }
        answer(["ok": true])
    case let command:
        throw Refusal(why: "the helper has no command \(command)")
    }
} catch let missing as Missing {
    answer(["missing": missing.permission])
} catch let refusal as Refusal {
    answer(["error": refusal.why])
} catch {
    answer(["error": "\(error.localizedDescription)"])
}
