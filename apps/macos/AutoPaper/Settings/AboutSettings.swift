import SwiftUI

/// App identity and licence, the same layout as AudioPaper's About pane: icon, name, "Version x (build)", whether and
/// how AutoPaper updates itself, a width-capped description, Website · Privacy · Help, "MIT · © 2026 Michael
/// Sitarzewski".
struct AboutSettings: View {
    var body: some View {
        VStack(spacing: 10) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .scaledToFit()
                .frame(width: 72, height: 72)
                .accessibilityHidden(true)
            Text("AutoPaper")
                .font(.title2.weight(.semibold))
                .accessibilityAddTraits(.isHeader)
            Text("Version \(AppInfo.version) (\(AppInfo.build))")
                .font(.callout)
                .textSelection(.enabled)
            UpdateControls(settings: Updates.shared.settings)
                .padding(.vertical, 4)
            Text("New wallpapers from a few words you choose, painted by the provider you pick, and remembered so they don't repeat.")
                .font(.callout)
                .multilineTextAlignment(.center)
                .lineSpacing(4)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 360)
            HStack(spacing: 14) {
                Link("Website", destination: Website.home)
                Text("·").accessibilityHidden(true)
                Link("Privacy", destination: Website.privacy)
                Text("·").accessibilityHidden(true)
                Link("Help", destination: Website.help)
            }
            .font(.callout)
            .padding(.top, 2)
            Text("MIT · © 2026 Michael Sitarzewski")
                .font(.caption)
                .padding(.top, 6)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 28)
        .padding(.horizontal)
    }
}

/// Settings → About, under the version: whether and how AutoPaper updates itself (AudioPaper's `UpdateControls`),
/// bound to Sparkle's own preferences, with when it last checked and Check Now.
private struct UpdateControls: View {
    @Bindable var settings: UpdateSettings

    var body: some View {
        VStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 6) {
                Toggle("Check for updates automatically", isOn: $settings.automaticallyChecks)
                Toggle("Download and install updates automatically", isOn: $settings.automaticallyDownloads)
                    .disabled(!settings.automaticallyChecks)
            }
            .toggleStyle(.checkbox)
            HStack(spacing: 10) {
                Text(lastChecked)
                    .font(.callout)
                    .quietText()
                Button("Check Now") { Updates.shared.checkForUpdates() }
                    .controlSize(.small)
                    .disabled(!settings.canCheck)
                    // The visible title stays the name (Voice Control: "Click Check Now"); the hint says what for.
                    .accessibilityHint("Checks for a new version of AutoPaper.")
            }
        }
    }

    private var lastChecked: String {
        guard let date = settings.lastChecked else { return "Not checked yet" }
        return "Last checked \(date.formatted(.relative(presentation: .named)))"
    }
}
