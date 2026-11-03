import SwiftUI

/// "Approve This iPhone" (or Mac): the code one of the person's devices approves it by, or, for someone new, their
/// inviter's.
struct ApproveView: View {
    var code: String
    var signOut: () -> Void

    var body: some View {
        VStack(spacing: 14) {
            Spacer()
            Text("Approve This \(Self.kind)").font(.title.bold()).multilineTextAlignment(.center)
            Text(Auth.invite != nil ? "Whoever invited you approves it. Show them this code." : "Open \(appName) on a device you already use, then Settings › Devices › Approve.")
                .foregroundStyle(.secondary).multilineTextAlignment(.center)
            Text(code).font(.system(size: 40, weight: .semibold, design: .monospaced)).kerning(4).textSelection(.enabled).padding(.top, 14)
            Spacer()
            Button("Sign Out", action: signOut)
        }
        .padding(28)
        .frame(maxWidth: 460)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private static var kind: String {
        #if os(macOS)
        "Mac"
        #else
        UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
        #endif
    }
}
