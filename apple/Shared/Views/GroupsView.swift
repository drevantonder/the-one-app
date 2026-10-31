import SwiftUI

/// Every group this user is in, and making one.
struct GroupsView: View {
    @Environment(Store.self) private var store
    var open: (Route) -> Void
    @State private var creating = false

    var body: some View {
        let mine = store.install?.groups.filter { g in store.me.map { g.role(of: $0.id) != nil } ?? false } ?? []
        List {
            Section {
                ForEach(mine) { group in
                    Button { open(.group(group.id)) } label: {
                        HStack(spacing: 12) {
                            SymbolTile(symbol: "person.3.fill", color: .green, size: 36)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(group.name).font(.body.weight(.semibold)).foregroundStyle(.primary)
                                Text(group.members.count == 1 ? "1 member" : "\(group.members.count) members")
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            if group.role(of: store.me?.id ?? "") == "admin" {
                                Text("Admin").font(.subheadline).foregroundStyle(.secondary)
                            }
                            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                        }
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
            } footer: {
                Text(mine.isEmpty
                     ? "You’re in no groups yet. A group shares its own apps and data, and has a page of its own."
                     : "Each group has a page of its own. Being in a group never shows its members your own data.")
            }
        }
        .navigationTitle("Groups")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("New Group", systemImage: "plus") { creating = true }
            }
        }
        .sheet(isPresented: $creating) {
            NameSheet(title: "New Group", prompt: "Name", action: "Create", footer: "You’ll be its first admin. Invite people once it exists.") { name in
                try await store.change { _ = try await $0.createGroup(name) }
            }
        }
    }
}

/// A group's members and invites; its admins change roles, invite and remove.
struct GroupView: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var id: String
    @State private var inviting = false
    @State private var error: String?

    var body: some View {
        let group = store.install?.group(id)
        let admin = group?.role(of: store.me?.id ?? "") == "admin"
        List {
            if let group {
                Section("Members") {
                    ForEach(group.members) { member in
                        HStack {
                            Text(store.userName(member.user))
                            Spacer()
                            Text(member.role == "admin" ? "Admin" : "Member").foregroundStyle(.secondary)
                        }
                        .contextMenu {
                            if admin && member.user != store.me?.id {
                                Button(member.role == "admin" ? "Make Member" : "Make Admin") {
                                    run { try await $0.setMember(group: id, user: member.user, role: member.role == "admin" ? "member" : "admin") }
                                }
                                Button("Remove from \(group.name)", role: .destructive) { run { try await $0.setMember(group: id, user: member.user, role: nil) } }
                            }
                        }
                    }
                }
                if let invited = group.invited, !invited.isEmpty {
                    Section("Invited") {
                        ForEach(invited) { member in
                            HStack {
                                Text(store.userName(member.user))
                                Spacer()
                                Text("Waiting").foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                Section {
                    if admin {
                        Button("Invite Someone…", systemImage: "person.badge.plus") { inviting = true }
                    }
                    Button("Leave \(group.name)", systemImage: "rectangle.portrait.and.arrow.right", role: .destructive) {
                        run { try await $0.setMember(group: id, user: store.me?.id ?? "", role: nil) }
                    }
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else if admin {
                        Text("As an admin you arrange its page, add apps to it and set its secrets.")
                    }
                }
            }
        }
        .navigationTitle(group?.name ?? "")
        .sheet(isPresented: $inviting) {
            InviteSheet(group: id)
        }
    }

    private func run(_ work: @escaping @MainActor (NodeClient) async throws -> Void) {
        error = nil
        Task {
            do { try await store.change(work) } catch { self.error = error.localizedDescription }
        }
    }
}

/// Inviting someone already in the install into a group.
private struct InviteSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var group: String
    @State private var role = "member"
    @State private var error: String?

    var body: some View {
        let g = store.install?.group(group)
        let others = (store.install?.users ?? []).filter { u in g?.role(of: u.id) == nil && g?.invited?.contains { $0.user == u.id } != true }
        NavigationStack {
            Form {
                Picker("Role", selection: $role) {
                    Text("Member").tag("member")
                    Text("Admin").tag("admin")
                }
                Section {
                    ForEach(others) { user in
                        Button(user.name) {
                            Task {
                                do {
                                    try await store.change { try await $0.invite(group: group, user: user.id, role: role) }
                                    dismiss()
                                } catch { self.error = error.localizedDescription }
                            }
                        }
                    }
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else if others.isEmpty {
                        Text("Everyone in \(store.install?.name ?? "the install") is in this group already. An admin adds new people in Users.")
                    } else {
                        Text("They join once they accept, from Requests.")
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Invite to \(g?.name ?? "")")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 380, height: 400)
        #endif
    }
}

/// Asks for one name, then runs `done` with it.
struct NameSheet: View {
    @Environment(\.dismiss) private var dismiss
    var title: LocalizedStringKey
    var prompt: LocalizedStringKey
    var action: LocalizedStringKey
    var footer: LocalizedStringKey
    var initial = ""
    var done: (String) async throws -> Void
    @State private var name = ""
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(prompt, text: $name)
                        .onSubmit(submit)
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else {
                        Text(footer)
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(title)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(action, action: submit).disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || busy)
                }
            }
            .onAppear { name = initial }
        }
        #if os(macOS)
        .frame(width: 380, height: 200)
        #else
        .presentationDetents([.medium])
        #endif
    }

    private func submit() {
        let name = name.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, !busy else { return }
        busy = true
        Task {
            defer { busy = false }
            do {
                try await done(name)
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}
