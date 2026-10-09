import AutopaperCore
import SwiftUI

/// The menu bar extra's menu (spec §1, top to bottom): what's on the desktop, Like / Dislike, Mood ▸, New Wallpaper
/// Now, the stage and Cancel New Wallpaper while one is being made, Pause / Resume, Restore My Wallpaper, Show
/// AutoPaper, Check for Updates…, Settings…, Quit.
/// HIG: "Display a menu — not a popover — when people click your menu bar extra."
struct MenuBarMenu: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        nowShowing
        Divider()
        moods
        Button("New Wallpaper Now") { model.newWallpaperNow() }
            .keyboardShortcut("n", modifiers: [.option, .command])
            .disabled(model.phase != .ready || model.isWorking)
        if let work = model.work {
            // The stage, with time left when the engine knows it (menu items don't wrap: one short line).
            Text(work.menuLine)
            if work.cancellable {
                Button("Cancel New Wallpaper") { model.cancel() }
            }
        }
        if model.budgetProblem != nil {
            Button("New wallpapers waiting for budget…") { model.openSettings(.budget) }
                .help(model.budgetStatus?.message ?? "")
        }
        Divider()
        let paused = model.settings?.paused ?? false
        Button(paused ? "Resume New Wallpapers" : "Pause New Wallpapers") { model.setPaused(!paused) }
            .disabled(model.settings == nil)
        // App-spec 3a: the person's own wallpaper, until AutoPaper's next new one.
        Button("Restore My Wallpaper") { model.restoreMyWallpaper() }
            .disabled(!model.autoPaperShowing)
        Divider()
        Button("Show AutoPaper") {
            model.showMainWindow()
        }
        Button("Show Console") {
            model.section = .console
            model.showMainWindow()
        }
        // As AudioPaper's menu: a menu bar app's app menu is only there while one of its windows is in front.
        Button("Check for Updates…") { Updates.shared.checkForUpdates() }
        Button("Settings…") { SettingsWindow.show(openSettings) }
            .keyboardShortcut(",", modifiers: .command)
        Divider()
        Button("Quit AutoPaper") { NSApp.terminate(nil) }
            .keyboardShortcut("q", modifiers: .command)
    }

    /// The current wallpaper's title (disabled text), its echo note on a second line (cut short: menu items don't
    /// wrap; Now has the whole note), and the rating items (checkmarked for the current rating; choosing it again
    /// clears it).
    @ViewBuilder
    private var nowShowing: some View {
        switch model.phase {
        case .starting:
            Text("Starting…")
        case .failed(let reason):
            Text(reason)
        case .ready:
            if let current = model.current {
                Text(MenuText.short(current.concept.title, limit: 60))
                if let note = current.echoNote, !note.isEmpty {
                    Text(MenuText.short(note))
                }
                Toggle("Like", isOn: Binding(
                    get: { current.rating == .liked },
                    set: { _ in model.rate(current, .liked) }
                ))
                Toggle("Dislike", isOn: Binding(
                    get: { current.rating == .disliked },
                    set: { _ in model.rate(current, .disliked) }
                ))
            } else {
                Text("No wallpaper yet")
            }
        }
    }

    /// Mood ▸ every mood (the current one checkmarked; choosing another makes it current without making a
    /// wallpaper), then Edit Moods….
    @ViewBuilder
    private var moods: some View {
        if model.phase == .ready, !model.moods.isEmpty {
            Menu("Mood") {
                ForEach(model.moods) { mood in
                    Toggle(MenuText.short(mood.name), isOn: Binding(
                        get: { mood.active },
                        set: { _ in model.useMood(mood.id) }
                    ))
                    .accessibilityLabel(MoodText.spokenName(mood))
                }
                Divider()
                Button("Edit Moods…") {
                    model.section = .moods
                    model.moodSelection = model.activeMood?.id ?? model.moodSelection
                    model.showMainWindow()
                }
            }
            Divider()
        }
    }
}
