import SwiftUI

struct RootView: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        Group {
            if store.state == nil {
                if case .unreachable(let message) = store.phase {
                    ContentUnavailableView {
                        Label("Can’t Reach \(appName)", systemImage: "bolt.horizontal.circle")
                    } description: {
                        Text(message)
                    } actions: {
                        Text("It keeps trying.").foregroundStyle(.secondary)
                    }
                } else {
                    ProgressView("Starting \(appName)…")
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else if store.space == nil {
                WelcomeView()
            } else {
                MainView()
            }
        }
        .sheet(item: $store.sheet) { SheetView(sheet: $0) }
    }
}

private struct MainView: View {
    @Environment(Store.self) private var store

    var body: some View {
        NavigationSplitView {
            Sidebar()
                .navigationSplitViewColumnWidth(min: 210, ideal: 240, max: 320)
        } detail: {
            Group {
                if store.space?.member == false {
                    ContentUnavailableView {
                        Label("Joining…", systemImage: "arrow.trianglehead.2.clockwise")
                    } description: {
                        Text("This device is syncing the space from the device it paired with.")
                    }
                } else {
                    @Bindable var store = store
                    NavigationStack(path: $store.path) {
                        Group {
                            if let selection = store.selection { RouteView(route: selection) }
                        }
                        .navigationDestination(for: Route.self) { RouteView(route: $0) }
                    }
                    .id(store.selection)
                }
            }
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
                    Button("Pair a Device", systemImage: "qrcode") { store.sheet = .pair }
                        .help("Show a pairing code that brings another device into this space")
                }
            }
        }
    }
}

private struct WelcomeView: View {
    @Environment(Store.self) private var store
    @State private var name = ""
    @State private var error: String?

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 14) {
                Image("Logo")
                    .resizable()
                    .frame(width: 112, height: 112)
                Text("Welcome to \(store.brand?.name ?? appName)")
                    .font(.largeTitle.weight(.semibold))
                Text("Your devices, and your team’s, as one place for apps and agents.\nStart a space here, or join one from another device.")
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 520)
            }
            VStack(spacing: 12) {
                HStack(spacing: 8) {
                    TextField("Space name", text: $name, prompt: Text("Name your space"))
                        .textFieldStyle(.roundedBorder)
                        .controlSize(.large)
                        .frame(width: 220)
                        .onSubmit(create)
                    Button("Create Space", action: create)
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                }
                Button("Have a pairing code? Join a space…") { store.sheet = .join }
                    .buttonStyle(.link)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func create() {
        guard !name.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        Task {
            do { try await store.createSpace(name) } catch { self.error = error.localizedDescription }
        }
    }
}
