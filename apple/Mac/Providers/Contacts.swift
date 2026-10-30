@preconcurrency import Contacts
import Foundation

struct ContactsApp: Provider {
    let app = "contacts"
    private static let store = Unshared(CNContactStore())

    var description: [String: Any] {
        describe(title: "Contacts", icon: "contact", instructions:
            "Contacts are the person's Apple Contacts, through their Mac. contacts_search finds people by name, email address or phone number; " +
            "contacts_me is the person's own card.", ops: [
            op("search", "Find contacts by name, email address or phone number.", [
                "query": str("a name or part of one, an email address, or a phone number"),
                "limit": num("at most this many; default 25"),
            ], required: ["query"]),
            op("get", "One contact, by the id search gives.", ["id": str("the contact's id")], required: ["id"]),
            op("me", "The person's own contact card."),
        ])
    }

    func run(_ op: String, _ input: Input) async throws -> Any {
        try await Self.access()
        let store = Self.store.value
        switch op {
        case "search":
            let q = try input.text("query")
            var predicates = [CNContact.predicateForContacts(matchingName: q)]
            if q.contains("@") { predicates.append(CNContact.predicateForContacts(matchingEmailAddress: q)) }
            if q.filter(\.isNumber).count >= 6 { predicates.append(CNContact.predicateForContacts(matching: CNPhoneNumber(stringValue: q))) }
            var found: [String: CNContact] = [:]
            for p in predicates {
                for c in try store.unifiedContacts(matching: p, keysToFetch: Self.keys) { found[c.identifier] = c }
            }
            return found.values.map(Self.json).sorted { ($0["name"] as! String) < ($1["name"] as! String) }.prefix(input.int("limit", or: 25)).map { $0 }
        case "get":
            let id = try input.text("id")
            do {
                return Self.json(try store.unifiedContact(withIdentifier: id, keysToFetch: Self.keys))
            } catch {
                throw Refusal("no contact with id \(id); contacts_search finds them")
            }
        case "me":
            do {
                return Self.json(try store.unifiedMeContactWithKeys(toFetch: Self.keys))
            } catch {
                throw Refusal("this Mac has no card set as the person's own: choose one in Contacts → Card → Make This My Card")
            }
        default:
            throw Refusal("contacts has no operation \(op)")
        }
    }

    static func access() async throws {
        try await permission("Contacts", status: status, ask: { store.value.requestAccess(for: .contacts) { _, _ in } },
            denied: "macOS doesn't let \(appName) use Contacts. Allow it in System Settings → Privacy & Security → Contacts: turn on \(appName).")
    }

    static func status() -> Access {
        switch CNContactStore.authorizationStatus(for: .contacts) {
        case .authorized, .limited: .granted
        case .notDetermined: .unasked
        default: .denied
        }
    }

    /// Everyone's name by each of their phone numbers and email addresses (see `handle`), if Contacts is
    /// allowed already. It never asks.
    static func names() -> [String: String] {
        guard status() == .granted else { return [:] }
        var names: [String: String] = [:]
        let request = CNContactFetchRequest(keysToFetch: [CNContactGivenNameKey, CNContactFamilyNameKey, CNContactOrganizationNameKey,
                                                          CNContactPhoneNumbersKey, CNContactEmailAddressesKey] as [CNKeyDescriptor])
        try? store.value.enumerateContacts(with: request) { c, _ in
            let name = displayName(c)
            guard !name.isEmpty else { return }
            for p in c.phoneNumbers { names[handle(p.value.stringValue)] = name }
            for e in c.emailAddresses { names[handle(e.value as String)] = name }
        }
        return names
    }

    /// A phone number or email address, as anyone may write it: an email lowercased, a number's last nine digits.
    static func handle(_ s: String) -> String {
        s.contains("@") ? s.lowercased() : String(s.filter(\.isNumber).suffix(9))
    }

    private static var keys: [CNKeyDescriptor] {
        [
            CNContactIdentifierKey, CNContactGivenNameKey, CNContactFamilyNameKey, CNContactNicknameKey,
            CNContactOrganizationNameKey, CNContactJobTitleKey, CNContactPhoneNumbersKey, CNContactEmailAddressesKey,
            CNContactPostalAddressesKey, CNContactBirthdayKey, CNContactUrlAddressesKey,
        ] as [CNKeyDescriptor]
    }

    private static func displayName(_ c: CNContact) -> String {
        let name = [c.givenName, c.familyName].filter { !$0.isEmpty }.joined(separator: " ")
        return name.isEmpty ? c.organizationName : name
    }

    private static func json(_ c: CNContact) -> [String: Any] {
        func label(_ l: String?) -> String { l.map { CNLabeledValue<NSString>.localizedString(forLabel: $0) } ?? "" }
        let birthday = c.birthday.map { b in
            [b.year, b.month, b.day].compactMap { $0 }.filter { $0 != NSDateComponentUndefined }.map { String(format: "%02d", $0) }.joined(separator: "-")
        }
        return object([
            "id": c.identifier, "name": displayName(c), "nickname": c.nickname.nonEmpty,
            "organization": c.organizationName.nonEmpty, "job": c.jobTitle.nonEmpty,
            "phones": c.phoneNumbers.isEmpty ? nil : c.phoneNumbers.map { ["label": label($0.label), "number": $0.value.stringValue] },
            "emails": c.emailAddresses.isEmpty ? nil : c.emailAddresses.map { ["label": label($0.label), "address": $0.value as String] },
            "addresses": c.postalAddresses.isEmpty ? nil : c.postalAddresses.map {
                ["label": label($0.label), "address": CNPostalAddressFormatter.string(from: $0.value, style: .mailingAddress)]
            },
            "birthday": birthday, "urls": c.urlAddresses.isEmpty ? nil : c.urlAddresses.map { $0.value as String },
        ])
    }
}

/// A framework object that's safe to use from any thread but isn't marked so.
struct Unshared<T>: @unchecked Sendable {
    let value: T
    init(_ value: T) { self.value = value }
}
