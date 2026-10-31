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

/// A group's members and invites. Each member's role is a button on their row: its admins change it or remove them
/// there. The last admin's role can't change.
struct GroupView: View {
    @Environment(Store.self) private var store
    var id: String
    @State private var inviting = false
    @State private var error: String?
    /// Roles changed but not yet taken in, shown at once.
    @State private var pending: [String: String?] = [:]

    var body: some View {
        let group = store.install?.group(id)
        let me = store.me?.id ?? ""
        let admin = group?.role(of: me) == "admin"
        let lastAdmin = group.map { $0.admins == [me] } ?? false
        List {
            if let group {
                Section {
                    ForEach(group.members.filter { pending[$0.user] != .some(nil) }) { member in
                        let role = pending[member.user].flatMap { $0 } ?? member.role
                        PersonRow(id: member.user, name: store.userName(member.user)) {
                            if admin && !(role == "admin" && group.admins.count == 1) {
                                RoleMenu(role: role, removeTitle: member.user == me ? "Leave \(group.name)" : "Remove from \(group.name)") { change(member.user, to: $0) }
                            } else {
                                Text(role == "admin" ? "Admin" : "Member").foregroundStyle(.secondary)
                            }
                        }
                    }
                } header: {
                    Text("Members")
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else if !admin {
                        Text("Only its admins can change who’s in it.")
                    } else if group.admins.count == 1 {
                        Text("A group always keeps one admin: make someone else an admin before you step down.")
                    }
                }
                if let invited = group.invited, !invited.isEmpty {
                    Section("Invited") {
                        ForEach(invited) { member in
                            PersonRow(id: member.user, name: store.userName(member.user)) {
                                Text("Waiting").foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                Section {
                    if admin {
                        Button("Invite Someone…", systemImage: "person.badge.plus") { inviting = true }
                    }
                    Button("Leave \(group.name)", systemImage: "rectangle.portrait.and.arrow.right", role: .destructive) { change(me, to: nil) }
                        .disabled(lastAdmin && group.members.count > 1)
                } footer: {
                    if lastAdmin && group.members.count > 1 {
                        Text("You’re its only admin: make someone else an admin before you leave.")
                    } else if admin {
                        Text("As an admin you arrange its page, add apps to it and set its secrets.")
                    }
                }
            }
        }
        .wholeTitle(group?.name ?? "")
        .sheet(isPresented: $inviting) {
            InviteSheet(group: id)
        }
    }

    /// Changes a member's role, or with none removes them: shown at once, and undone with why if it fails.
    private func change(_ user: String, to role: String?) {
        error = nil
        pending[user] = .some(role)
        Task {
            do { try await store.change { try await $0.setMember(group: id, user: user, role: role) } } catch { self.error = error.localizedDescription }
            pending[user] = nil
        }
    }
}

/// A person, with their avatar, and what goes at the end of their row.
struct PersonRow<Trailing: View>: View {
    var id: String
    var name: String
    var detail: String?
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 12) {
            Avatar(id: id, name: name, size: 32)
            VStack(alignment: .leading, spacing: 1) {
                Text(name).lineLimit(1)
                if let detail { Text(detail).font(.caption).foregroundStyle(.secondary) }
            }
            Spacer(minLength: 8)
            trailing
        }
        .frame(minHeight: 36)
    }
}

/// A role, as a button that offers the other role and removing them.
struct RoleMenu: View {
    var role: String
    var removeTitle: String
    var change: (String?) -> Void

    var body: some View {
        Menu {
            Picker("Role", selection: Binding(get: { role }, set: { change($0) })) {
                Text("Admin").tag("admin")
                Text("Member").tag("member")
            }
            .pickerStyle(.inline)
            Button(removeTitle, systemImage: "person.badge.minus", role: .destructive) { change(nil) }
        } label: {
            HStack(spacing: 3) {
                Text(role == "admin" ? "Admin" : "Member")
                Image(systemName: "chevron.up.chevron.down").font(.caption2.weight(.semibold))
            }
        }
        .menuStyle(.button)
        .buttonStyle(.bordered)
        .controlSize(.small)
        .fixedSize()
    }
}

/// Inviting someone already in the install into a group: search by name, tap (or Return for the first) to invite.
private struct InviteSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var group: String
    @State private var role = "member"
    @State private var query = ""
    @State private var error: String?

    var body: some View {
        let g = store.install?.group(group)
        let others = (store.install?.users ?? []).filter { u in g?.role(of: u.id) == nil && g?.invited?.contains { $0.user == u.id } != true }
        let matches = query.trimmingCharacters(in: .whitespaces).isEmpty ? others : others.filter { $0.name.localizedStandardContains(query.trimmingCharacters(in: .whitespaces)) }
        NavigationStack {
            Form {
                Picker("Invite as", selection: $role) {
                    Text("Member").tag("member")
                    Text("Admin").tag("admin")
                }
                Section {
                    ForEach(matches) { user in
                        Button { invite(user.id) } label: {
                            PersonRow(id: user.id, name: user.name, detail: user.admin ? String(localized: "Admin of \(store.install?.name ?? "")") : nil) {
                                Image(systemName: "plus.circle.fill").foregroundStyle(.tint).font(.title3)
                            }
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                    }
                    if matches.isEmpty && !others.isEmpty { Text("No one called “\(query)”.").foregroundStyle(.secondary) }
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
            .searchable(text: $query, prompt: "Find people")
            .onSubmit(of: .search) { if let first = matches.first { invite(first.id) } }
            .navigationTitle("Invite to \(g?.name ?? "")")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 400, height: 460)
        #endif
    }

    private func invite(_ user: String) {
        Task {
            do {
                try await store.change { try await $0.invite(group: group, user: user, role: role) }
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
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
