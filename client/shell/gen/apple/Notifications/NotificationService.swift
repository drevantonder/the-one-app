import CryptoKit
import Foundation
import UserNotifications

/// The app's notification extension. What APNs carries is the notification encrypted to this iPhone (`n`), with a
/// fixed line for Apple to see; this opens it before it shows, with the key the app keeps in the folder it shares
/// with this extension, its App Group. It's `client/src/cipher.rs` (`open_notification`), as CryptoKit does it:
///
/// - The notification key is HKDF-SHA256 of the device's key (the 32 bytes of `key`), for
///   `one/notifications/1/key`, and is an Ed25519 key as the device's is.
/// - What's sealed to it is HPKE (RFC 9180) in base mode, with X25519, HKDF-SHA256 and ChaCha20-Poly1305, to that
///   key's X25519 form, for `one/notifications/1/notification` and the key's public half in hex, on a line of its
///   own. The first 32 bytes are the encapsulated key.
///
/// One that doesn't open shows the fixed line, as an iPhone too old for HPKE does.
final class NotificationService: UNNotificationServiceExtension {
    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent,
              let sealed = (request.content.userInfo["n"] as? String).flatMap({ Data(base64Encoded: $0) }),
              let said = try? Self.open(sealed)
        else { return contentHandler(request.content) }
        content.title = said["title"] as? String ?? ""
        content.body = said["text"] as? String ?? ""
        content.userInfo["app"] = said["app"]
        contentHandler(content)
    }

    /// The notification `sealed` holds, opened with this iPhone's key: its `title`, `text` and `app`.
    static func open(_ sealed: Data) throws -> [String: Any] {
        guard #available(iOS 17, *) else { throw Unopened() }
        let group = Bundle.main.object(forInfoDictionaryKey: "AppGroup") as? String ?? ""
        guard let folder = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else { throw Unopened() }
        let device = try Data(contentsOf: folder.appendingPathComponent("key"))
        guard device.count == 32, sealed.count > 32 else { throw Unopened() }
        let seed = HKDF<SHA256>.deriveKey(inputKeyMaterial: SymmetricKey(data: device), info: Data("one/notifications/1/key".utf8), outputByteCount: 32)
            .withUnsafeBytes { Data($0) }
        let named = try Curve25519.Signing.PrivateKey(rawRepresentation: seed).publicKey.rawRepresentation.map { String(format: "%02x", $0) }.joined()
        let own = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: Data(SHA512.hash(data: seed)).prefix(32))
        var recipient = try HPKE.Recipient(
            privateKey: own,
            ciphersuite: .Curve25519_SHA256_ChachaPoly,
            info: Data("one/notifications/1/notification\n\(named)".utf8),
            encapsulatedKey: sealed.prefix(32)
        )
        let opened = try recipient.open(sealed.dropFirst(32), authenticating: Data())
        guard let said = try JSONSerialization.jsonObject(with: opened) as? [String: Any] else { throw Unopened() }
        return said
    }

    struct Unopened: Error {}
}
