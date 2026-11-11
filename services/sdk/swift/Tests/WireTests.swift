import Foundation
import Testing
@testable import Helper

/// What a service and its app runtime say to each other is in service/wire.json, which tests/contracts.rs holds the app runtime, the
/// Rust service crate and WhatsApp's Go service to. This holds the Swift helpers to it (`swift test` on a Mac).
private func wire() -> [String: Any] { try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appending(path: "wire.json"))) as! [String: Any] }

@Test func readsACallAsTheNodeSendsIt() throws {
    let sent = wire()["call"] as! [String: Any]
    let call = try Wire.call(JSONSerialization.data(withJSONObject: sent))
    #expect(call.id == 7)
    #expect(call.function == "events")
    #expect(call.input.string("calendar") == "Home")
    #expect(call.input.int("limit", or: 0) == 5)
    #expect(call.input.caller("user") == "u1")
    #expect(call.input.caller("agent") == "Claude Code")
}

@Test func answersAsTheNodeReadsIt() async throws {
    let replies = wire()["replies"] as! [String: Any]
    let ok = await Wire.reply { [["title": "Lunch"]] }
    #expect(NSDictionary(dictionary: ok).isEqual(to: replies["ok"] as! [String: Any]))
    let error = await Wire.reply { throw Failure("give a title") }
    #expect(NSDictionary(dictionary: error).isEqual(to: replies["error"] as! [String: Any]))
    let refused = await Wire.reply { throw Refusal("only its owner may") }
    #expect(NSDictionary(dictionary: refused).isEqual(to: replies["refused"] as! [String: Any]))
}

@Test func registersOnlyWhatIsAboutItsMac() {
    let allowed = Set(wire()["registers"] as! [String])
    let said = Wire.registration(localApp: "com.apple.iCal")
    #expect(said["version"] != nil)
    #expect(Set(said.keys).isSubset(of: allowed))
}
