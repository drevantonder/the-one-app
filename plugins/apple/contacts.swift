// one-contacts: the Mac side of the contacts plugin.
// Usage: one-contacts <command> < input.json   →   {"result": …} or {"error": "…"}
import Contacts
import Foundation

func fail(_ msg: String) -> Never {
    FileHandle.standardOutput.write(try! JSONSerialization.data(withJSONObject: ["error": msg]))
    exit(1)
}

func done(_ result: Any) -> Never {
    FileHandle.standardOutput.write(try! JSONSerialization.data(withJSONObject: ["result": result], options: [.sortedKeys]))
    exit(0)
}

let store = CNContactStore()
let keys: [CNKeyDescriptor] = [
    CNContactIdentifierKey, CNContactGivenNameKey, CNContactFamilyNameKey, CNContactNicknameKey,
    CNContactOrganizationNameKey, CNContactJobTitleKey, CNContactPhoneNumbersKey, CNContactEmailAddressesKey,
    CNContactPostalAddressesKey, CNContactBirthdayKey, CNContactUrlAddressesKey,
] as [CNKeyDescriptor]

func access() {
    let sem = DispatchSemaphore(value: 0)
    var ok = false
    store.requestAccess(for: .contacts) { granted, _ in ok = granted; sem.signal() }
    if sem.wait(timeout: .now() + 60) == .timedOut {
        fail("waiting for macOS permission: allow access in the prompt on the Mac's screen, or in System Settings → Privacy & Security → Contacts")
    }
    if !ok { fail("macOS denied access to Contacts. Allow it in System Settings → Privacy & Security → Contacts.") }
}

func label(_ l: String?) -> String {
    guard let l else { return "" }
    return CNLabeledValue<NSString>.localizedString(forLabel: l)
}

func json(_ c: CNContact) -> [String: Any] {
    var o: [String: Any] = ["id": c.identifier, "name": [c.givenName, c.familyName].filter { !$0.isEmpty }.joined(separator: " ")]
    if !c.nickname.isEmpty { o["nickname"] = c.nickname }
    if !c.organizationName.isEmpty { o["organization"] = c.organizationName }
    if !c.jobTitle.isEmpty { o["jobTitle"] = c.jobTitle }
    if !c.phoneNumbers.isEmpty { o["phones"] = c.phoneNumbers.map { ["label": label($0.label), "number": $0.value.stringValue] } }
    if !c.emailAddresses.isEmpty { o["emails"] = c.emailAddresses.map { ["label": label($0.label), "address": $0.value as String] } }
    if !c.postalAddresses.isEmpty {
        o["addresses"] = c.postalAddresses.map { ["label": label($0.label), "address": CNPostalAddressFormatter.string(from: $0.value, style: .mailingAddress)] }
    }
    if let b = c.birthday { o["birthday"] = [b.year, b.month, b.day].compactMap { $0 == NSDateComponentUndefined ? nil : $0 }.map(String.init).joined(separator: "-") }
    if !c.urlAddresses.isEmpty { o["urls"] = c.urlAddresses.map { $0.value as String } }
    return o
}

let args = CommandLine.arguments
guard args.count == 2 else { fail("usage: one-contacts <command> < input.json") }
let raw = FileHandle.standardInput.readDataToEndOfFile()
let input = (raw.isEmpty ? [:] : (try? JSONSerialization.jsonObject(with: raw)) as? [String: Any]) ?? [:]

do {
    access()
    switch args[1] {
    case "contacts.search":
        guard let q = (input["query"] as? String)?.trimmingCharacters(in: .whitespaces), !q.isEmpty else { fail("query is required") }
        var found: [String: CNContact] = [:]
        for c in try store.unifiedContacts(matching: CNContact.predicateForContacts(matchingName: q), keysToFetch: keys) { found[c.identifier] = c }
        if q.contains("@") {
            for c in try store.unifiedContacts(matching: CNContact.predicateForContacts(matchingEmailAddress: q), keysToFetch: keys) { found[c.identifier] = c }
        }
        if q.filter(\.isNumber).count >= 6 {
            for c in try store.unifiedContacts(matching: CNContact.predicateForContacts(matching: CNPhoneNumber(stringValue: q)), keysToFetch: keys) { found[c.identifier] = c }
        }
        let limit = input["limit"] as? Int ?? 25
        done(Array(found.values.map(json).sorted { ($0["name"] as! String) < ($1["name"] as! String) }.prefix(limit)))
    case "contacts.get":
        guard let id = input["id"] as? String else { fail("id is required") }
        done(json(try store.unifiedContact(withIdentifier: id, keysToFetch: keys)))
    case "contacts.me":
        done(json(try store.unifiedMeContactWithKeys(toFetch: keys)))
    default:
        fail("unknown command \(args[1])")
    }
} catch {
    fail(error.localizedDescription)
}
