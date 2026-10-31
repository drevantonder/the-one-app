import SwiftUI

/// The App Store, until the brand's store app exists: every app in the install, and where each is installed.
/// Adding one to a page installs it in that page's scope, with the user's own rights.
struct AppStoreView: View {
    @Environment(Store.self) private var store
    @State private var adding: AppInfo?

    var body: some View {
        List {
            Section {
                ForEach(store.apps) { app in
                    HStack(spacing: 12) {
                        AppTile(symbol: symbol(app.icon), seed: app.name, size: 44)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(app.label).font(.body.weight(.semibold))
                            Text(detail(app))
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                        Spacer()
                        Button("Add to…") { adding = app }
                            .buttonStyle(.bordered)
                            .buttonBorderShape(.capsule)
                    }
                    .padding(.vertical, 2)
                }
            } footer: {
                Text("Apps without a screen give their pages widgets and tools for agents, but no icon.")
            }
        }
        .navigationTitle("App Store")
        .sheet(item: $adding) { AddToSheet(app: $0) }
    }

    private func detail(_ app: AppInfo) -> String {
        let pages = store.allPages.filter { app.scopes.contains($0.scope) }.map(\.title)
        let on = pages.isEmpty ? String(localized: "Not on any of your pages") : String(localized: "On \(pages.formatted())")
        return app.hasScreens ? on : String(localized: "\(on) · no screen")
    }
}

/// The pages this user may add an app to: theirs, groups they admin, and everyone's if they're an admin.
struct AddToSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var app: AppInfo
    @State private var busy: String?
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ForEach(store.allPages.filter { store.canArrange($0.scope) }) { page in
                        let on = app.scopes.contains(page.scope)
                        Button {
                            toggle(page, on: on)
                        } label: {
                            HStack {
                                Text(page.title).foregroundStyle(.primary)
                                Spacer()
                                if busy == page.scope {
                                    ProgressView().controlSize(.small)
                                } else if on {
                                    Image(systemName: "checkmark").foregroundStyle(.tint)
                                }
                            }
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                    }
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else {
                        Text("Everyone on a page sees what you add to it. Groups you don’t admin are their admins’ to arrange.")
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Add \(app.label) to…")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 400, height: 360)
        #else
        .presentationDetents([.medium])
        #endif
    }

    private func toggle(_ page: Page, on: Bool) {
        busy = page.scope
        error = nil
        Task {
            defer { busy = nil }
            do { try await store.change { try await $0.install(app: app.name, scope: page.scope, !on) } } catch { self.error = error.localizedDescription }
        }
    }
}
