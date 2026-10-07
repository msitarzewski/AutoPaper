import AutopaperCore
import SwiftUI

/// Accounts: the API keys, in the Keychain. Fields save as you type (no Save button), each with a "Get a key" link
/// and a spoken label naming its service. An OpenAI-compatible server's key belongs to that server (the core names
/// its Keychain account after the server's address), so its field appears once a server is chosen in Providers.
struct AccountsSettings: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        SettingsPane { settings in
            Section {
                KeyField(account: ProviderKind.openAi.keyAccount ?? "openai.api_key", label: "API key", spokenLabel: "OpenAI API key")
            } header: {
                Text("OpenAI")
            } footer: {
                Link("Get a key", destination: URL(string: "https://platform.openai.com/api-keys")!)
                    .accessibilityLabel("Get a key for OpenAI")
            }
            Section {
                KeyField(account: ProviderKind.google.keyAccount ?? "google.api_key", label: "API key", spokenLabel: "Google Gemini API key")
            } header: {
                Text("Google Gemini")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    Link("Get a key", destination: URL(string: "https://aistudio.google.com/apikey")!)
                        .accessibilityLabel("Get a key for Google Gemini")
                    Text("A free Gemini key may let Google use what you send to improve its products; a paid project doesn't.")
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Section {
                let servers = compatibleServers(settings)
                if servers.isEmpty {
                    Text("Choose an OpenAI-compatible provider and its address in Providers to add its key here. Many servers on your own network don't need one.")
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    ForEach(servers, id: \.account) { server in
                        KeyField(account: server.account, label: "Key for \(server.address)", spokenLabel: "Key for \(server.address)")
                    }
                }
            } header: {
                Text("OpenAI-compatible server")
            } footer: {
                Text("The key is sent only to the server it was entered for. Change the address and the server gets its own key.")
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    /// Each OpenAI-compatible server in use (writing, painting or both), with the account its key is saved under.
    private func compatibleServers(_ settings: EngineSettings) -> [(account: String, address: String)] {
        var servers: [(account: String, address: String)] = []
        for selection in [settings.textProvider, settings.imageProvider] where selection.kind == .openAiCompatible {
            guard let account = secretAccountFor(selection: selection), let address = selection.baseUrl,
                  !servers.contains(where: { $0.account == account })
            else { continue }
            servers.append((account, address))
        }
        return servers
    }
}

/// The one thing to do about a missing or refused key: a link that opens Settings on Accounts with that key's field
/// focused. Used wherever a key problem shows in Providers, instead of directions to follow.
struct KeyLink: View {
    let problem: KeyProblem

    var body: some View {
        SettingsProblemLink(problem: SettingsProblem(key: problem))
    }
}

/// A problem with a setting, said once: its line (if it has one) and one link that goes straight to where it's
/// fixed (spec 6a) — "Add your OpenAI key", "Check ComfyUI in Settings", "Raise the budget". VoiceOver hears the
/// link's title, with the problem as its hint.
struct SettingsProblemLink: View {
    @Environment(AppModel.self) private var model
    let problem: SettingsProblem

    var body: some View {
        Label {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if !problem.sentence.isEmpty {
                    Text(problem.sentence)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Button(problem.linkTitle) { model.open(problem.place) }
                    .buttonStyle(.link)
                    .fixedSize()
                    .accessibilityLabel(problem.linkTitle)
                    .accessibilityHint("\(problem.context) \(Self.destination(problem.place))")
            }
        } icon: {
            Image(systemName: problem.symbol)
                .accessibilityHidden(true)
        }
    }

    /// Where the link goes, for its hint.
    private static func destination(_ place: SettingsProblem.Place) -> String {
        switch place {
        case .accounts: "Opens Accounts in Settings."
        case .providers: "Opens Providers in Settings."
        case .budget: "Opens Budget in Settings."
        case .systemWallpaper: "Opens Wallpaper in System Settings."
        case .mood: "Opens the mood in Moods."
        }
    }
}

/// A key field that loads from and saves to the Keychain on its own, like other macOS settings: shortly after
/// typing stops, off the main thread (the Keychain can block on an access prompt).
struct KeyField: View {
    @Environment(AppModel.self) private var model
    let account: String
    let label: String
    /// Out of its section "API key" is ambiguous, so VoiceOver hears the service too ("OpenAI API key").
    let spokenLabel: String
    /// Told the field's value when it loads and as it changes (the welcome waits for a key).
    var onValue: ((String) -> Void)?
    @State private var text = ""
    @State private var loaded = false
    @State private var saved: Bool?
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            SecureField(label, text: $text, prompt: Text("Paste your key"))
                .accessibilityLabel(spokenLabel)
                .focused($focused)
                .onAppear(perform: takeFocusIfAsked)
                .onChange(of: model.accountToFocus) { takeFocusIfAsked() }
            if let saved {
                if saved {
                    if !text.isEmpty {
                        Label("Saved in Keychain", systemImage: "checkmark.circle")
                            .font(.callout)
                    }
                } else {
                    Label("Couldn't save to the Keychain", systemImage: "exclamationmark.triangle")
                        .font(.callout)
                }
            }
        }
        .task(id: account) {
            let account = account
            let value = await Task.detached { KeychainSecretStore.shared.get(account: account) }.value ?? ""
            text = value
            saved = value.isEmpty ? nil : true
            loaded = true
            onValue?(value)
        }
        .task(id: text) {
            guard loaded else { return }
            onValue?(text)
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            let account = account, value = text
            if value.trimmingCharacters(in: .whitespacesAndNewlines) == (await Task.detached { KeychainSecretStore.shared.get(account: account) }.value ?? "") {
                return
            }
            let ok = await Task.detached { KeychainSecretStore.shared.save(value, account: account) }.value
            saved = ok
            if ok { model.keysChanged += 1 }
            let removed = value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            model.announce(ok
                ? "\(spokenLabel) \(removed ? "removed from" : "saved in") Keychain."
                : "Couldn't save the \(spokenLabel) to the Keychain.")
        }
    }

    /// A key link asked for this field: focus it (once).
    private func takeFocusIfAsked() {
        guard model.accountToFocus == account else { return }
        model.accountToFocus = nil
        DispatchQueue.main.async { focused = true }
    }
}
