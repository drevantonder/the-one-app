// Computer Use for The One App: the app macOS knows Computer Use by on a Mac. macOS allows Screen Recording and
// Accessibility to the program it holds responsible for a process, which for a program a launchd agent starts is
// that program: so this app runs Cua Driver's daemon (cua-driver serve --embedded) as its own child, and macOS asks
// for and keeps both in this app's name, with its icon, through Cua Driver's updates and this app's, as it's signed
// by a developer. See Cua Driver's guide to embedding it (Skills/cua-driver/EMBEDDING.md).
//
//     helper run <program> [<argument>...]   runs the program as this app's child until it ends, and ends as it did
//     helper ask <permission>                asks macOS for Accessibility or Screen Recording, which it does on the
//                                            Mac's screen; started with open(1), so this app is responsible for it
//     helper secure-input                    whether a password field has the focus: true or false
import ApplicationServices
import Carbon.HIToolbox
import Foundation

let arguments = CommandLine.arguments.dropFirst()

switch arguments.first {
case "run" where arguments.count > 1:
    let program = Array(arguments.dropFirst())
    var child = pid_t()
    let argv = program.map { strdup($0) } + [nil]
    let started = posix_spawn(&child, program[0], nil, nil, argv, environ)
    guard started == 0 else {
        FileHandle.standardError.write(Data("couldn't start \(program[0]): \(String(cString: strerror(started)))\n".utf8))
        exit(1)
    }
    // launchd stops this app with SIGTERM, which the child is stopped with too.
    for signal in [SIGTERM, SIGINT, SIGHUP] {
        let source = DispatchSource.makeSignalSource(signal: signal, queue: .main)
        Foundation.signal(signal, SIG_IGN)
        source.setEventHandler { kill(child, signal) }
        source.resume()
    }
    DispatchQueue.global().async {
        var status: Int32 = 0
        while waitpid(child, &status, 0) == -1 && errno == EINTR {}
        exit((status & 0x7f) == 0 ? (status >> 8) & 0xff : 128 + (status & 0x7f))
    }
    dispatchMain()
case "ask" where arguments.count == 2:
    if arguments.last == "Screen Recording" {
        _ = CGRequestScreenCaptureAccess()
    } else {
        _ = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue(): true] as CFDictionary)
    }
case "secure-input":
    print(IsSecureEventInputEnabled())
default:
    FileHandle.standardError.write(Data("usage: helper run <program> [<argument>...] | ask <permission> | secure-input\n".utf8))
    exit(64)
}
