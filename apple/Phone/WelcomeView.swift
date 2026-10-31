import SwiftUI
import VisionKit

/// The first run: name this iPhone, then join with the pairing code another device shows.
struct WelcomeView: View {
    @Environment(Phone.self) private var phone
    @State private var name = UIDevice.current.name
    @State private var scanning = false
    @State private var typing = false

    private var named: Bool { !name.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        ScrollView {
            VStack(spacing: 36) {
                VStack(spacing: 14) {
                    Image("Logo")
                        .resizable()
                        .frame(width: 120, height: 120)
                        .accessibilityHidden(true)
                    Text("Welcome to \(appName)")
                        .font(.largeTitle.bold())
                    Text("On a computer that’s already set up, open Settings, then Devices, and choose Pair a Phone or Computer. Scan its code here.")
                        .font(.body)
                        .foregroundStyle(.secondary)
                }
                .multilineTextAlignment(.center)
                .padding(.top, 48)

                VStack(alignment: .leading, spacing: 8) {
                    Text("Name")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.leading, 16)
                    TextField("Name", text: $name, prompt: Text("Your iPhone"))
                        .textContentType(.name)
                        .submitLabel(.done)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 13)
                        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
                    Text("Others see this iPhone by this name.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 16)
                }
            }
            .padding(.horizontal, 24)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Color(.systemGroupedBackground))
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 14) {
                if let error = phone.joinError {
                    Label(error, systemImage: "exclamationmark.circle.fill")
                        .font(.callout)
                        .foregroundStyle(.red)
                        .multilineTextAlignment(.leading)
                }
                Button { scanning = true } label: {
                    Group {
                        if phone.joining {
                            ProgressView()
                        } else {
                            Label("Scan Pairing Code", systemImage: "qrcode.viewfinder")
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                Button("Enter a Pairing Code Instead") { typing = true }
                    .font(.callout.weight(.medium))
            }
            .disabled(!named || phone.joining)
            .padding(.horizontal, 24)
            .padding(.bottom, 8)
        }
        .animation(.snappy, value: phone.joinError)
        .sheet(isPresented: $scanning) {
            ScannerSheet { code in
                scanning = false
                join(code)
            }
        }
        .sheet(isPresented: $typing) {
            CodeSheet { code in
                typing = false
                join(code)
            }
        }
    }

    private func join(_ code: String) {
        Task { await phone.join(code, name: name.trimmingCharacters(in: .whitespaces)) }
    }
}

/// The camera, looking for a pairing code's QR code.
private struct ScannerSheet: View {
    var found: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if DataScannerViewController.isSupported {
                    Scanner(found: found)
                        .ignoresSafeArea()
                        .overlay(alignment: .bottom) {
                            Text("Point the camera at the code on your other device.")
                                .font(.callout.weight(.medium))
                                .padding(.horizontal, 18)
                                .padding(.vertical, 12)
                                .glassEffect(.regular, in: .capsule)
                                .padding(.bottom, 24)
                        }
                } else {
                    ContentUnavailableView("No Camera", systemImage: "camera", description: Text("This device can’t scan codes. Enter the pairing code instead."))
                }
            }
            .navigationTitle("Scan Pairing Code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", systemImage: "xmark", role: .cancel) { dismiss() }
                }
            }
        }
    }
}

private struct Scanner: UIViewControllerRepresentable {
    var found: (String) -> Void

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
        context.coordinator.found = found
        if !scanner.isScanning { try? scanner.startScanning() }
    }

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        var found: (String) -> Void
        private var done = false

        init(found: @escaping (String) -> Void) { self.found = found }

        func dataScanner(_ scanner: DataScannerViewController, didAdd items: [RecognizedItem], allItems: [RecognizedItem]) {
            for case .barcode(let code) in items {
                guard !done, let text = code.payloadStringValue else { continue }
                done = true
                scanner.stopScanning()
                found(text)
            }
        }
    }
}

/// A pairing code, pasted.
private struct CodeSheet: View {
    var entered: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var code = ""
    @FocusState private var focused: Bool

    private var trimmed: String { code.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Pairing Code", text: $code, axis: .vertical)
                        .font(.system(.body, design: .monospaced))
                        .lineLimit(3...8)
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                        .focused($focused)
                } footer: {
                    Text("Copy the code from Pair a Phone or Computer on your other device, or run \(Bundle.main.object(forInfoDictionaryKey: "BrandCLI") as? String ?? "") pair there. Each code works once.")
                }
            }
            .navigationTitle("Enter a Pairing Code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", systemImage: "xmark", role: .cancel) { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Join", systemImage: "checkmark", role: .confirm) { entered(trimmed) }
                        .disabled(trimmed.isEmpty)
                }
            }
            .onAppear { focused = true }
        }
        .presentationDetents([.medium])
    }
}
