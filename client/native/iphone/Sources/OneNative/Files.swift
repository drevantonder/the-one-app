import QuickLook
import UIKit
import VisionKit

/// Files for screens: shared (the share sheet, which saves to Files too), shown (Quick Look), or scanned from paper.
enum Files {
  static func share(_ url: URL, from top: UIViewController?) {
    let share = UIActivityViewController(activityItems: [url], applicationActivities: nil)
    if let view = top?.view {
      share.popoverPresentationController?.sourceView = view
      share.popoverPresentationController?.sourceRect = CGRect(x: view.bounds.midX, y: view.bounds.maxY - 40, width: 1, height: 1)
    }
    top?.present(share, animated: true)
  }

  /// Quick Look, with its own share button.
  static func preview(_ url: URL, from top: UIViewController?) {
    let source = PreviewSource(url: url)
    let preview = QLPreviewController()
    preview.dataSource = source
    objc_setAssociatedObject(preview, &PreviewSource.key, source, .OBJC_ASSOCIATION_RETAIN)
    top?.present(preview, animated: true)
  }

  private final class PreviewSource: NSObject, QLPreviewControllerDataSource {
    nonisolated(unsafe) static var key = 0
    let url: URL
    init(url: URL) { self.url = url }
    func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
    func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem { url as NSURL }
  }

  /// The document camera: finds each page's edges and straightens it, several in a row → each page as a JPEG
  /// `{ name, type, size, base64 }`, "Scan <date> <n>.jpg"; none if the person cancels.
  static func scan(from top: UIViewController?, done: @escaping (Result<[[String: Any]], Error>) -> Void) {
    guard VNDocumentCameraViewController.isSupported else {
      return done(.failure(NSError(domain: "one", code: 1, userInfo: [NSLocalizedDescriptionKey: "This device can’t scan documents. Choose the files instead."])))
    }
    let camera = VNDocumentCameraViewController()
    let scanner = Scanner { result in
      camera.dismiss(animated: true)
      done(result)
    }
    camera.delegate = scanner
    objc_setAssociatedObject(camera, &Scanner.key, scanner, .OBJC_ASSOCIATION_RETAIN)
    top?.present(camera, animated: true)
  }

  private final class Scanner: NSObject, VNDocumentCameraViewControllerDelegate {
    nonisolated(unsafe) static var key = 0
    let finish: (Result<[[String: Any]], Error>) -> Void
    init(finish: @escaping (Result<[[String: Any]], Error>) -> Void) { self.finish = finish }

    func documentCameraViewController(_ controller: VNDocumentCameraViewController, didFinishWith scan: VNDocumentCameraScan) {
      let stamp = Date.now.formatted(.iso8601.year().month().day())
      let pages = (0..<scan.pageCount).compactMap { i -> [String: Any]? in
        guard let jpeg = Self.shrunk(scan.imageOfPage(at: i)).jpegData(compressionQuality: 0.8) else { return nil }
        return ["name": "Scan \(stamp) \(i + 1).jpg", "type": "image/jpeg", "size": jpeg.count, "base64": jpeg.base64EncodedString()]
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
}
