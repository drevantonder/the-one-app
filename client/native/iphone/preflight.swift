// Asks Apple whether a TestFlight build can go through, before the twenty minutes it takes to find out, and installs its
// App Store profiles. Says everything missing at once. Reads the certificate's public half from the keychain only, so it
// never brings up a keychain prompt.
import CryptoKit
import Foundation

let env = ProcessInfo.processInfo.environment
func need(_ name: String) -> String { env[name] ?? "" }
let id = need("CONFIG_IPHONE"), name = need("CONFIG_NAME"), push = need("PUSH") == "YES"
var missing: [String] = []

func token() throws -> String {
    let b64 = { (d: Data) in d.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "") }
    let now = Int(Date().timeIntervalSince1970)
    let head = try JSONSerialization.data(withJSONObject: ["alg": "ES256", "kid": need("ASC_KEY_ID"), "typ": "JWT"])
    let body = try JSONSerialization.data(withJSONObject: ["iss": need("ASC_ISSUER"), "iat": now, "exp": now + 600, "aud": "appstoreconnect-v1"] as [String: Any])
    let signing = b64(head) + "." + b64(body)
    let key = try P256.Signing.PrivateKey(pemRepresentation: String(contentsOfFile: need("ASC_KEY"), encoding: .utf8))
    return signing + "." + b64(try key.signature(for: Data(signing.utf8)).rawRepresentation)
}

let jwt = try token()
func ask(_ path: String) throws -> [String: Any] {
    var request = URLRequest(url: URL(string: "https://api.appstoreconnect.apple.com/v1/" + path.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)!)!)
    request.setValue("Bearer \(jwt)", forHTTPHeaderField: "Authorization")
    var answer: (Data?, URLResponse?, Error?)
    let done = DispatchSemaphore(value: 0)
    URLSession.shared.dataTask(with: request) { answer = ($0, $1, $2); done.signal() }.resume()
    done.wait()
    if let error = answer.2 { throw error }
    let status = (answer.1 as? HTTPURLResponse)?.statusCode ?? 0
    guard status == 200, let data = answer.0, let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw NSError(domain: "App Store Connect answered \(status) to \(path): \(String(data: answer.0 ?? Data(), encoding: .utf8) ?? "")", code: status)
    }
    return json
}
typealias Row = [String: Any]
func rows(_ json: Row, _ key: String = "data") -> [Row] { json[key] as? [Row] ?? [] }
func attributes(_ row: Row) -> Row { row["attributes"] as? Row ?? [:] }
func links(_ row: Row, _ key: String) -> [String] {
    let data = ((row["relationships"] as? Row)?[key] as? Row)?["data"]
    return ((data as? [Row]) ?? (data as? Row).map { [$0] } ?? []).compactMap { $0["id"] as? String }
}

// The app and each extension's identifier, with what it signs with.
var targets = [(id, name + " App Store"), (id + ".widgets", name + " Widgets App Store")]
if push { targets.append((id + ".notifications", name + " Notifications App Store")) }
let identifiers = try ask("bundleIds?filter[identifier]=\(targets.map(\.0).joined(separator: ","))&include=bundleIdCapabilities&limit=20")
let capabilities = Dictionary(uniqueKeysWithValues: rows(identifiers, "included").map { ($0["id"] as? String ?? "", attributes($0)["capabilityType"] as? String ?? "") })
var bundleRows: [String: Row] = [:]
for row in rows(identifiers) { bundleRows[attributes(row)["identifier"] as? String ?? ""] = row }
for (identifier, _) in targets {
    guard let row = bundleRows[identifier] else { missing.append("No App ID \(identifier): register it at developer.apple.com"); continue }
    let has = Set(links(row, "bundleIdCapabilities").compactMap { capabilities[$0] })
    if !has.contains("APP_GROUPS") { missing.append("\(identifier) doesn't have App Groups on") }
    if push && identifier == id && !has.contains("PUSH_NOTIFICATIONS") { missing.append("\(identifier) doesn't have Push Notifications on") }
}
if rows(try ask("apps?filter[bundleId]=\(id)")).isEmpty { missing.append("No app for \(id) in App Store Connect") }

// The certificates this Mac can sign with, by fingerprint.
let list = Process(), pipe = Pipe()
list.executableURL = URL(fileURLWithPath: "/usr/bin/security")
list.arguments = ["find-identity", "-v", "-p", "codesigning"]
list.standardOutput = pipe
try list.run()
let identitiesHere = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
list.waitUntilExit()

let certificates = try ask("certificates?filter[certificateType]=DISTRIBUTION&limit=200")
let usable = Set(rows(certificates).filter { row in
    guard let content = attributes(row)["certificateContent"] as? String, let der = Data(base64Encoded: content) else { return false }
    let hash = Insecure.SHA1.hash(data: der).map { String(format: "%02X", $0) }.joined()
    return identitiesHere.contains(hash)
}.compactMap { $0["id"] as? String })
if usable.isEmpty { missing.append("No Apple Distribution certificate of this team is in a keychain this user searches") }

let names = targets.map(\.1).joined(separator: ",")
let profiles = try ask("profiles?filter[name]=\(names)&filter[profileType]=IOS_APP_STORE&include=bundleId,certificates&limit=20")
let folder = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/MobileDevice/Provisioning Profiles")
try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
let formatter = ISO8601DateFormatter()
let fractional = ISO8601DateFormatter()
fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
for (identifier, profileName) in targets {
    guard let row = rows(profiles).first(where: { attributes($0)["name"] as? String == profileName }) else { missing.append("No App Store profile named \"\(profileName)\" for \(identifier)"); continue }
    let about = attributes(row)
    let expires = (about["expirationDate"] as? String).flatMap { formatter.date(from: $0) ?? fractional.date(from: $0) }
    if about["profileState"] as? String != "ACTIVE" || (expires ?? .distantPast) < Date() { missing.append("The profile \"\(profileName)\" has expired or is invalid: make it again") }
    if !usable.isEmpty && usable.isDisjoint(with: links(row, "certificates")) { missing.append("The profile \"\(profileName)\" isn't on a certificate in this Mac's keychain") }
    guard missing.isEmpty, let content = about["profileContent"] as? String, let bytes = Data(base64Encoded: content), let uuid = about["uuid"] as? String else { continue }
    try bytes.write(to: folder.appendingPathComponent(uuid + ".mobileprovision"))
}

if !missing.isEmpty {
    print("Apple isn't ready for this build:")
    missing.forEach { print("  " + $0) }
    exit(1)
}
print("Apple has everything for \(name) \(id), and its profiles are installed.")
