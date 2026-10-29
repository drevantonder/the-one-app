import SwiftUI

// A phone or iPad can't run a node, so the app reaches one of the user's
// devices over Tailscale.
@main
struct OneApp: App {
    var body: some Scene {
        WindowGroup {
            ShellView(shell: Shell(nodes: [URL(string: "http://mac:4747")!, URL(string: "http://linux:4747")!]))
                .modifier(PopoutCover())
                .onAppear { WidgetContext.popouts = { Popouts.shared.open($0, $1) } }
        }
    }
}
