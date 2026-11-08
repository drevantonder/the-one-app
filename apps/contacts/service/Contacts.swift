@preconcurrency import Contacts
import Helper
import Foundation

struct ContactsApp: Service {
    let app = "contacts"
    let localApp: String? = "com.apple.AddressBook"
    private static let store = Unshared(CNContactStore())

    func run(_ function: String, _ input: Input) async throws -> Any {
        try Self.access()
        let store = Self.store.value
        switch function {
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
                throw Refusal("this Mac has no card set as the user's own: choose one in Contacts → Card → Make This My Card")
            }
        default:
            throw Refusal("contacts has no function \(function)")
        }
    }

    func needs() async -> String? {
        Helper.needs("your contacts", status: Self.status(), ask: { Self.store.value.requestAccess(for: .contacts) { _, _ in } }, settings: "Contacts")
    }

    static func access() throws {
        try permission(status(), denied: "macOS doesn't let \(thisApp) use Contacts. Allow it in System Settings → Privacy & Security → Contacts: turn on \(thisApp).")
    }

    static func status() -> Access {
        switch CNContactStore.authorizationStatus(for: .contacts) {
        case .authorized, .limited: .granted
        case .notDetermined: .unasked
        default: .denied
        }
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
