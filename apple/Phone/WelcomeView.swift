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

    /// "Your One App", or a workplace's, "The PTC App" (project.sh).
    private var whose: String {
        let yours = Bundle.main.object(forInfoDictionaryKey: "Yours") as? String ?? ""
        return yours.prefix(1).uppercased() + yours.dropFirst()
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 36) {
                VStack(spacing: 14) {
                    Image("Logo")
                        .resizable()
                        .frame(width: 120, height: 120)
                        .accessibilityHidden(true)
                    Text(appName)
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
                        .background(Kit.fill, in: .rect(cornerRadius: 14))
                    Text("\(whose)’s address, or the invite link you were sent.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 16)
                }
                Button("Or Connect Your AI") { asking = true }
                    .font(.callout.weight(.medium))
            }
            .padding(.horizontal, 24)
        }
        .scrollDismissesKeyboard(.interactively)
        .groupedBackground()
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
            ConnectYourAISheet(prompt: agentPrompt(address: given.isEmpty ? nil : given))
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
