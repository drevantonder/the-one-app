import SwiftUI
import UniformTypeIdentifiers
#if os(iOS)
import QuickLook
import VisionKit
#else
import AppKit
#endif

/// A file a screen hands the shell, as `one.save`, `one.preview` and ops' results give one: `{ name, type, base64 }`,
/// or `{ name, type, blob }` for a blob the node has.
struct ScreenFile {
    var name: String
    var type: String
    var data: Data

    /// Reads it from a bridge message, fetching a blob through the node if it names one.
    static func from(_ value: Any?, node: NodeClient) async throws -> ScreenFile {
        guard let fields = value as? [String: Any] else { throw NodeError("No file was given.") }
        let type = fields["type"] as? String ?? "application/octet-stream"
        let ext = UTType(mimeType: type)?.preferredFilenameExtension
        var name = (fields["name"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? "File"
        if let ext, !name.lowercased().hasSuffix(".\(ext)"), !name.contains(".") { name += ".\(ext)" }
        if let text = fields["base64"] as? String, let data = Data(base64Encoded: text, options: .ignoreUnknownCharacters) {
            return ScreenFile(name: name, type: type, data: data)
        }
        if let hash = (fields["blob"] ?? fields["file"]) as? String {
            return ScreenFile(name: name, type: type, data: try await node.blob(hash))
        }
        throw NodeError("The file has neither base64 nor a blob.")
    }

    /// The file, written where the shell can hand it on: a folder of its own in the temporary directory.
    func write() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appending(path: "screen-files/\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appending(path: name.replacingOccurrences(of: "/", with: "-"))
        try data.write(to: url)
        return url
    }

    /// As a screen gets one back, from a file input or the scanner.
    var json: [String: Any] { ["name": name, "type": type, "size": data.count, "base64": data.base64EncodedString()] }
}

extension NodeClient {
    /// A blob's bytes: a server's local API gives them as they are, a phone's client as `{ blob }`, base64.
    func blob(_ hash: String) async throws -> Data {
        let data = try await transport.send("GET", "v1/blobs/\(hash)", body: nil, timeout: 60)
        if let reply = try? JSONSerialization.jsonObject(with: data) as? [String: Any], let text = reply["blob"] as? String,
           let bytes = Data(base64Encoded: text) {
            return bytes
        }
        return data
    }
}

/// What the shell does with files for a screen: save or share them, show them, scan paper into them.
@MainActor
enum ShellFiles {
    #if os(iOS)
    /// The view controller on top, to present from.
    static func top(_ view: UIView?) -> UIViewController? {
        var top = view?.window?.rootViewController
        while let next = top?.presentedViewController { top = next }
        return top
    }

    /// The share sheet, which also saves to Files.
    static func save(_ file: ScreenFile, from view: UIView?) throws {
        let url = try file.write()
        let share = UIActivityViewController(activityItems: [url], applicationActivities: nil)
        share.popoverPresentationController?.sourceView = view
        share.popoverPresentationController?.sourceRect = CGRect(x: (view?.bounds.midX ?? 0), y: (view?.bounds.maxY ?? 0) - 40, width: 1, height: 1)
        top(view)?.present(share, animated: true)
    }

    /// Quick Look, with its own share button.
    static func preview(_ file: ScreenFile, from view: UIView?) throws {
        let source = PreviewSource(url: try file.write())
        let preview = QLPreviewController()
        preview.dataSource = source
        objc_setAssociatedObject(preview, &PreviewSource.key, source, .OBJC_ASSOCIATION_RETAIN)
        top(view)?.present(preview, animated: true)
    }

    private final class PreviewSource: NSObject, QLPreviewControllerDataSource {
        nonisolated(unsafe) static var key = 0
        let url: URL
        init(url: URL) { self.url = url }
        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem { url as NSURL }
    }

    static var canScan: Bool { VNDocumentCameraViewController.isSupported }

    /// The document camera: finds each page's edges and straightens it, several in a row. Each page as a JPEG; none if
    /// the person cancels.
    static func scan(from view: UIView?) async throws -> [ScreenFile] {
        guard canScan else { throw NodeError("This device can’t scan documents.") }
        return try await withCheckedThrowingContinuation { done in
            let camera = VNDocumentCameraViewController()
            let delegate = Scanner { result in
                camera.dismiss(animated: true)
                done.resume(with: result)
            }
            camera.delegate = delegate
            objc_setAssociatedObject(camera, &Scanner.key, delegate, .OBJC_ASSOCIATION_RETAIN)
            top(view)?.present(camera, animated: true)
        }
    }

    private final class Scanner: NSObject, VNDocumentCameraViewControllerDelegate {
        nonisolated(unsafe) static var key = 0
        let finish: (Result<[ScreenFile], Error>) -> Void
        init(finish: @escaping (Result<[ScreenFile], Error>) -> Void) { self.finish = finish }

        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
            let stamp = Date.now.formatted(.iso8601.year().month().day())
            let pages = (0..<scan.pageCount).compactMap { i -> ScreenFile? in
                guard let jpeg = Self.shrunk(scan.imageOfPage(at: i)).jpegData(compressionQuality: 0.8) else { return nil }
                return ScreenFile(name: "Scan \(stamp) \(i + 1).jpg", type: "image/jpeg", data: jpeg)
            }
            finish(.success(pages))
        }

        func documentCameraViewControllerDidCancel(_ controller: VNDocumentCameraViewController) { finish(.success([])) }
        func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFailWithError error: Error) { finish(.failure(error)) }

        /// At most 2400 points on its long side: plenty for reading a receipt, and small enough to send.
        private static func shrunk(_ image: UIImage) -> UIImage {
            let long = max(image.size.width, image.size.height)
            guard long > 2400 else { return image }
            let scale = 2400 / long
            let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            return UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
        }
    }
    #else
    /// A save panel, as a sheet on the screen's window.
    static func save(_ file: ScreenFile, from view: NSView?) async throws {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = file.name
        if let type = UTType(mimeType: file.type) { panel.allowedContentTypes = [type] }
        let answer = if let window = view?.window { await panel.beginSheetModal(for: window) } else { panel.runModal() }
        guard answer == .OK, let url = panel.url else { return }
        try file.data.write(to: url)
    }

    /// The file, opened in the app that opens its kind (Preview, for a PDF or an image).
    static func preview(_ file: ScreenFile, from view: NSView?) throws {
        NSWorkspace.shared.open(try file.write())
    }

    static var canScan: Bool { false }

    static func scan(from view: NSView?) async throws -> [ScreenFile] {
        throw NodeError("Scanning documents needs an iPhone or iPad. Choose the files instead.")
    }
    #endif
}
