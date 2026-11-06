import Foundation
import Security

/// The app's device keys, one for each person at each address, in the Keychain group it shares with this extension
/// (Info.plist's `KeychainGroup`, the App Group's), where the app keeps them (src/secrets.rs): read here only to open
/// what's pushed to this device; nothing copies one out.
enum DeviceKeys {
    private static let cli = Bundle.main.object(forInfoDictionaryKey: "CLI") as? String ?? "one"
    private static let group = Bundle.main.object(forInfoDictionaryKey: "KeychainGroup") as? String

    /// Every device key kept here, to find the one a note was sealed to.
    static func all() -> [Data] {
        var query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "\(cli) device",
                                    kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitAll]
        if let group { query[kSecAttrAccessGroup as String] = group }
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let found = out as? [Data] else { return [] }
        return found.filter { $0.count == 32 }
    }
}
