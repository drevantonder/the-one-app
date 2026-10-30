import CoreImage.CIFilterBuiltins
import SwiftUI
import AppKit

struct SheetView: View {
    var sheet: Sheet

    var body: some View {
        switch sheet {
        case .invite: InviteSheet()
        case .join: JoinSheet()
        case .newSpace: NewSpaceSheet()
        case .newTopic(let parent): NewTopicSheet(parent: parent)
        }
    }
}

private struct SheetLayout<Content: View, Actions: View>: View {
    var title: LocalizedStringKey
    var message: LocalizedStringKey
    @ViewBuilder var content: Content
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.title2.weight(.semibold))
                Text(message).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            content
            HStack {
                Spacer()
                actions
            }
        }
        .padding(24)
        .frame(width: 440)
    }
}

private struct ErrorLine: View {
    var error: String?

    var body: some View {
        if let error {
            Label(error, systemImage: "exclamationmark.circle.fill")
                .font(.callout)
                .foregroundStyle(.red)
        }
    }
}

struct InviteSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var mine = true
    @State private var ticket: String?
    @State private var error: String?
    @State private var copied = false

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 6) {
                Text("Invite a Device").font(.title2.weight(.semibold))
                Text("Bring another device into \(store.space?.title ?? "this space").")
                    .foregroundStyle(.secondary)
            }
            Picker("Who is it for?", selection: $mine) {
                Text("One of My Devices").tag(true)
                Text("Someone Else").tag(false)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            Text(mine
                 ? "It joins as you, with everything you can see and do."
                 : "They join as a new person and a member of this space.")
                .font(.callout)
                .foregroundStyle(.secondary)
            ZStack {
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(.white)
                    .shadow(color: .black.opacity(0.12), radius: 10, y: 3)
                if let ticket, let qr = qrCode(ticket) {
                    Image(decorative: qr, scale: 1)
                        .interpolation(.none)
                        .resizable()
                        .scaledToFit()
                        .padding(16)
                } else if let error {
                    ErrorLine(error: error).padding()
                } else {
                    ProgressView()
                }
            }
            .frame(width: 236, height: 236)
            .padding(.vertical, 4)
            HStack(spacing: 10) {
                Text(ticket ?? " ")
                    .font(.system(.callout, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .foregroundStyle(.secondary)
                Button(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc", action: copy)
                    .disabled(ticket == nil)
                    .contentTransition(.symbolEffect(.replace))
            }
            .padding(.leading, 12)
            .padding(.trailing, 4)
            .padding(.vertical, 4)
            .background(.quaternary.opacity(0.5), in: .capsule)
            Text("Scan it, or run \(store.brand?.cli ?? "") join with the ticket on the other device. It works once.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 440)
        .task(id: mine) {
            ticket = nil; error = nil; copied = false
            do { ticket = try await store.invite(mine: mine) } catch { self.error = error.localizedDescription }
        }
    }

    private func copy() {
        guard let ticket else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(ticket, forType: .string)
        copied = true
    }

    private func qrCode(_ text: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let image = filter.outputImage else { return nil }
        return CIContext().createCGImage(image, from: image.extent)
    }
}

struct JoinSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var ticket = ""
    @State private var error: String?
    @State private var joining = false

    var body: some View {
        SheetLayout(title: "Join a Space", message: "Paste the ticket from the device that invited you. Each ticket works once.") {
            TextField("Ticket", text: $ticket, prompt: Text("Ticket"))
                .font(.system(.body, design: .monospaced))
                .textFieldStyle(.roundedBorder)
                .controlSize(.large)
                .onChange(of: ticket) { error = nil }
            ErrorLine(error: error)
        } actions: {
            if joining { ProgressView().controlSize(.small).padding(.trailing, 4) }
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button("Join") {
                joining = true
                Task {
                    defer { joining = false }
                    do { try await store.join(ticket); dismiss() } catch { self.error = error.localizedDescription }
                }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(ticket.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || joining)
        }
        .onAppear {
            let clip = NSPasteboard.general.string(forType: .string) ?? ""
            let trimmed = clip.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.count > 60, trimmed.allSatisfy({ $0.isLetter || $0.isNumber }) { ticket = trimmed }
        }
    }
}

struct NewSpaceSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var error: String?

    var body: some View {
        SheetLayout(title: "New Space", message: "A space holds devices, their apps and their agents. Start with your own, then invite others.") {
            TextField("Name", text: $name, prompt: Text("Home"))
                .textFieldStyle(.roundedBorder)
                .onSubmit(create)
            ErrorLine(error: error)
        } actions: {
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button("Create", action: create)
                .keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
        }
    }

    private func create() {
        Task {
            do { try await store.createSpace(name); dismiss() } catch { self.error = error.localizedDescription }
        }
    }
}

struct NewTopicSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State var parent: String
    @State private var name = ""
    @State private var description = ""
    @State private var error: String?

    private var slug: String {
        name.lowercased()
            .split { !$0.isLetter && !$0.isNumber }
            .joined(separator: "-")
            .filter { $0.isASCII }
    }

    var body: some View {
        SheetLayout(title: "New Topic", message: "Topics keep the board’s posts about one thing together.") {
            Form {
                Picker("Inside", selection: $parent) {
                    Text("Board").tag("")
                    ForEach(store.topics) { Text($0.path).tag($0.path) }
                }
                TextField("Name", text: $name, prompt: Text("What it’s about, in a word or two"))
                TextField("About", text: $description, prompt: Text("One line saying what belongs here"))
            }
            .onSubmit(create)
            if !slug.isEmpty {
                Text(parent.isEmpty ? slug : "\(parent)/\(slug)")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            ErrorLine(error: error)
        } actions: {
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button("Create", action: create)
                .keyboardShortcut(.defaultAction)
                .disabled(slug.isEmpty || description.trimmingCharacters(in: .whitespaces).isEmpty)
        }
    }

    private func create() {
        guard !slug.isEmpty else { return }
        Task {
            do {
                try await store.createTopic(path: parent.isEmpty ? slug : "\(parent)/\(slug)", description: description)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
