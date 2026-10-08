import AutopaperCore
import ServiceManagement
import SwiftUI

/// General: how often, pausing, replacing disliked wallpapers, what to do when a new one can't be made, how wallpapers
/// are shown (over or as the person's own) and the lock screen (set by macOS, not by apps), Open at login, Show in
/// menu bar, notifications.
struct GeneralSettings: View {
    @Environment(AppModel.self) private var model
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var loginProblem: String?
    @AppStorage(Notifier.enabledKey) private var notify = false
    @State private var notificationsDenied = false

    var body: some View {
        @Bindable var model = model
        SettingsPane { _ in
            Section {
                Picker("New wallpaper", selection: model.setting(\.cadence, .daily)) {
                    ForEach(Cadence.all, id: \.self) { Text($0.title).tag($0) }
                }
                Toggle(isOn: model.setting(\.paused, false)) {
                    Text("Pause new wallpapers")
                    Text("The wallpaper you have stays up. New Wallpaper Now still works.")
                }
                Toggle(isOn: model.setting(\.replaceDisliked, true)) {
                    Text("Replace wallpapers I dislike")
                    Text("Disliking the wallpaper on your desktop makes a new one right away, within the budget.")
                }
                Picker(selection: model.setting(\.fallback, .revisitLiked)) {
                    Text(Fallback.revisitLiked.title).tag(Fallback.revisitLiked)
                    Text(Fallback.keepCurrent.title).tag(Fallback.keepCurrent)
                } label: {
                    Text("When a new one can't be made")
                    Text("When you're offline or a provider is down. Budget limits always keep your current wallpaper.")
                }
                .pickerStyle(.radioGroup)
                .accessibilityLabel("When a new one can't be made")
            }

            // App-spec 3a: over the person's own wallpaper (the default) or as it. Spec: set the lock screen where the
            // OS allows, otherwise say why it's unavailable. macOS has no API for an app to set the lock screen picture,
            // so there's no toggle here and the engine's `set_lock_screen` setting is intentionally unused on macOS
            // (DesktopService); macOS shows the desktop picture there, which is AutoPaper's only As my wallpaper.
            Section {
                Picker(selection: $model.wallpaperMode) {
                    ForEach(WallpaperMode.allCases) { Text($0.title).tag($0) }
                } label: {
                    Text("Show wallpapers")
                    Text(model.wallpaperMode.explanation)
                }
                // Pausing keeps AutoPaper's wallpaper up (user, 2026-10-06); this is how the person's own comes back
                // (as in the menu bar menu and the Wallpaper menu), until AutoPaper's next new one.
                LabeledContent {
                    Button("Restore My Wallpaper") { model.restoreMyWallpaper() }
                        .disabled(!model.autoPaperShowing)
                } label: {
                    Text("Your own wallpaper")
                    Text(model.autoPaperShowing
                         ? "Shows your own wallpaper again until AutoPaper's next new one."
                         : "Your own wallpaper is showing.")
                }
                LabeledContent {
                    Text("Set by macOS")
                        .foregroundStyle(.primary)
                } label: {
                    Text("Lock screen")
                    Text(model.wallpaperMode == .overlay
                         ? "macOS shows your own desktop picture there. The lock screen shows AutoPaper's wallpapers only with Show wallpapers set to As my wallpaper."
                         : "Apps can't set the Mac's lock screen picture on its own. macOS shows your desktop picture there, which is AutoPaper's wallpaper.")
                }
            }

            Section {
                Toggle("Open at login", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, enabled in setLaunchAtLogin(enabled) }
                if let loginProblem {
                    Label(loginProblem, systemImage: "exclamationmark.triangle")
                }
                Toggle(isOn: $model.showInMenuBar) {
                    Text("Show in menu bar")
                    Text("When hidden, AutoPaper appears in the Dock instead. Open it again from Finder to get back here.")
                }
                Toggle(isOn: $notify) {
                    Text("Notify me about new wallpapers")
                    Text(notificationsDenied
                         ? "Notifications for AutoPaper are turned off in System Settings → Notifications."
                         : "A notification with Like and Dislike for each new scheduled wallpaper, and when the budget runs out or a key stops working.")
                }
                .onChange(of: notify) { _, enabled in
                    guard enabled else { return }
                    Task {
                        let allowed = await Notifier.shared.requestPermission()
                        notificationsDenied = !allowed
                        if !allowed { notify = false }
                    }
                }
            }
        }
    }

    private func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            loginProblem = nil
        } catch {
            launchAtLogin = SMAppService.mainApp.status == .enabled
            loginProblem = SMAppService.mainApp.status == .requiresApproval
                ? "Allow AutoPaper in System Settings → General → Login Items."
                : "macOS didn't change the login item: \(error.localizedDescription)"
        }
    }
}
