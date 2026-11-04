import SwiftUI
#if os(macOS)
import CoreImage.CIFilterBuiltins
#endif

/// "Waiting for Approval": the code one of the person's devices approves it by, or, for someone new, their inviter's;
/// and their browser where they're signed in, opened at Approve with this code, here or, from a Mac, by its QR on a
/// phone.
struct ApproveView: View {
    var code: String
    var signOut: () -> Void

    var body: some View {
        VStack(spacing: 14) {
            Spacer()
            Text("Waiting for Approval").font(.title.bold()).multilineTextAlignment(.center)
            Text(Auth.invite != nil ? "Whoever invited you approves it. Show them this code." : "Your other devices ask you to approve it. It shows this code there too.")
                .foregroundStyle(.secondary).multilineTextAlignment(.center)
            Text(code).font(.system(size: 40, weight: .semibold, design: .monospaced)).kerning(4).textSelection(.enabled).padding(.top, 14)
            if Auth.invite == nil, let link {
                Button("Approve in Your Browser") { open(link) }.padding(.top, 8)
                #if os(macOS)
                if let qr = Self.qr(link.absoluteString) {
                    Image(nsImage: qr).interpolation(.none).resizable().frame(width: 140, height: 140).padding(.top, 8)
                    Text("Or scan this with your phone.").font(.footnote).foregroundStyle(.secondary)
                }
                #endif
            }
            Spacer()
            Button("Sign Out", action: signOut)
        }
        .padding(28)
        .frame(maxWidth: 460)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// Approve in the web client at the install's address, with this code (`/approve#CODE`).
    private var link: URL? {
        guard let address = Auth.address?.absoluteString else { return nil }
        let base = address.hasSuffix("/") ? String(address.dropLast()) : address
        return URL(string: "\(base)/approve#\(code.replacingOccurrences(of: " ", with: ""))")
    }

    private func open(_ url: URL) {
        #if os(macOS)
        NSWorkspace.shared.open(url)
        #else
        UIApplication.shared.open(url)
        #endif
    }

    #if os(macOS)
    /// `text` as a QR code, sharp at any size.
    private static func qr(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        guard let image = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else { return nil }
        let rep = NSCIImageRep(ciImage: image)
        let out = NSImage(size: rep.size)
        out.addRepresentation(rep)
        return out
    }
    #endif
}
