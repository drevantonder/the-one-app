import Foundation
import Security

/// This app's device keys, one for each person at each address, in the Keychain: on this device only, readable once it
/// has been unlocked since it started. Where its notification extension reads them too (`KeychainGroup`: the app group's
/// on an iPhone, the team's on a Mac built with push), to open what's pushed to it there; nothing copies one out.
enum DeviceKeys {
    private static let cli = Bundle.main.object(forInfoDictionaryKey: "CLI") as? String ?? "one"
    private static let group = Bundle.main.object(forInfoDictionaryKey: "KeychainGroup") as? String
    /// The key a sign-in here is for, until it knows whose (`signingInFor`).
    private static let signingIn = "signing in"

    /// A short name for `id`, for its folder.
    static func name(_ id: String) -> String {
        String(id.utf8.reduce(UInt64(14_695_981_039_346_656_037)) { ($0 ^ UInt64($1)) &* 1_099_511_628_211 }, radix: 16)
    }

    /// The keys' items, `id`'s alone if given, where they're kept now; or where an older build kept them (`shared`
    /// false): the app's own keychain.
    private static func query(_ id: String?, shared: Bool = true) -> [String: Any] {
        var query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "\(cli) device"]
        if let id { query[kSecAttrAccount as String] = id }
        if shared, let group {
            query[kSecAttrAccessGroup as String] = group
            #if os(macOS)
            query[kSecUseDataProtectionKeychain as String] = true
            #endif
        }
        return query
    }

    private static func read(_ query: [String: Any]) -> (data: Data, group: String?)? {
        var out: CFTypeRef?
        let asked = query.merging([kSecReturnData as String: true, kSecReturnAttributes as String: true]) { $1 }
        guard SecItemCopyMatching(asked as CFDictionary, &out) == errSecSuccess, let item = out as? [String: Any],
              let data = item[kSecValueData as String] as? Data, data.count == 32 else { return nil }
        return (data, item[kSecAttrAccessGroup as String] as? String)
    }

    @discardableResult
    private static func add(_ id: String, _ secret: Data) -> Bool {
        var item = query(id).merging([kSecValueData as String: secret]) { $1 }
        #if os(iOS)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        #else
        if group != nil { item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly }
        #endif
        return SecItemAdd(item as CFDictionary, nil) == errSecSuccess
    }

    /// `id`'s key, if it has one: moved first from where an older build kept it, if it's there.
    private static func kept(_ id: String) -> Data? {
        if let found = read(query(id)) { return found.data }
        guard group != nil, let old = read(query(id, shared: false)) else { return nil }
        // Moved, not copied: the old one goes only once the new one is kept. On an iPhone, only from the group it was in.
        guard add(id, old.data) else { return old.data }
        var gone = query(id, shared: false)
        #if os(iOS)
        guard let from = old.group, from != group else { return old.data }
        gone[kSecAttrAccessGroup as String] = from
        #endif
        SecItemDelete(gone as CFDictionary)
        return old.data
    }

    /// `id`'s key, made the first time: for a person's device (`person`), the one its sign-in was for (`signingInFor`).
    static func secret(_ id: String, person: Bool = true) -> Data {
        if let secret = kept(id) { return secret }
        let secret = (person ? kept(signingIn) : nil) ?? newDeviceSecret()
        SecItemDelete(query(id) as CFDictionary)
        add(id, secret)
        forget(signingIn)
        return secret
    }

    /// `id`'s key goes: a key the install removed can't be let in again, so the next sign-in makes a new one.
    static func forget(_ id: String) {
        SecItemDelete(query(id) as CFDictionary)
        if group != nil { SecItemDelete(query(id, shared: false) as CFDictionary) }
    }

    /// What a sign-in here names this device by (the authorization's `join`), so the person's devices can let it in by
    /// its sign-in: the key the first person it signs in as takes (`secret`), made before it knows whose.
    static func signingInFor() -> String? {
        let secret = kept(signingIn) ?? {
            let made = newDeviceSecret()
            add(signingIn, made)
            return made
        }()
        return try? deviceJoin(secret: secret)
    }

    /// Every device key kept here, for the notification extension to find the one a note was sealed to.
    static func all() -> [Data] {
        var out: CFTypeRef?
        let asked = query(nil).merging([kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitAll]) { $1 }
        guard SecItemCopyMatching(asked as CFDictionary, &out) == errSecSuccess, let found = out as? [Data] else { return [] }
        return found.filter { $0.count == 32 }
    }
}
