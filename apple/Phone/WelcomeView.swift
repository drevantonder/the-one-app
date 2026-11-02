import SwiftUI
import AuthenticationServices

/// The first run: your One App's address, or an invite link to it, then your passkey.
struct WelcomeView: View {
    @Environment(Phone.self) private var phone
    @Environment(\.webAuthenticationSession) private var session
    @Environment(\.openURL) private var openURL
    @State private var address = Auth.address?.absoluteString.replacingOccurrences(of: "https://", with: "") ?? ""
    @State private var invited = false
    @State private var asking = false

    private var given: String { address.trimmingCharacters(in: .whitespacesAndNewlines) }

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
                    if invited {
                        Text("Made your passkey? Sign in with it.")
                            .font(.body)
                            .foregroundStyle(.secondary)
                    }
                }
                .multilineTextAlignment(.center)
                .padding(.top, 48)

                VStack(alignment: .leading, spacing: 8) {
                    Text("Address")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.leading, 16)
                    TextField("Address", text: $address, prompt: Text("one.example.com"))
                        .textContentType(.URL)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.go)
                        .onSubmit(signIn)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 13)
                        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
                    Text("Your One App’s address, or the invite link you were sent.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 16)
                }
                Button("Or Ask Your Agent") { asking = true }
                    .font(.callout.weight(.medium))
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
                Button(action: signIn) {
                    Group {
                        if phone.signingIn {
                            ProgressView()
                        } else {
                            Text(given.contains("/invite#") ? "Open Invite" : "Sign In")
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 30)
                }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                .disabled(given.isEmpty || phone.signingIn)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 8)
        }
        .animation(.snappy, value: phone.joinError)
        .sheet(isPresented: $asking) {
            AskYourAgentSheet(prompt: agentPrompt(address: given.isEmpty ? nil : given))
        }
    }

    /// An invite link opens in Safari, where the person makes their passkey; then they sign in with it here.
    private func signIn() {
        guard !given.isEmpty else { return }
        Task {
            if let invite = await phone.signIn(given, session: session) {
                invited = true
                address = Auth.address?.absoluteString.replacingOccurrences(of: "https://", with: "") ?? ""
                openURL(invite)
            }
        }
    }
}
