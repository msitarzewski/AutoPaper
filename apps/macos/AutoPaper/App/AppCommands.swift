import AutopaperCore
import SwiftUI

/// The main window's menus: File ▸ New Mood (⌘N), Rename Mood… and New Wallpaper Now (⌥⌘N), a Wallpaper menu for what's
/// showing (with Restore My Wallpaper), View ▸ the three sections (⌘1–⌘3), and Edit ▸ Move Up / Down (⌥⌘↑ / ⌥⌘↓)
/// for the mood or keyword selected in Moods, and Keyword Weight (⌃⌘1–3) for the selected keyword.
///
/// ⌘N is New Mood (spec: "⌘N in Moods = New Mood"); New Wallpaper Now has ⌥⌘N everywhere, here and in the menu bar
/// menu, so one shortcut never means two things.
struct AppCommands: Commands {
    let model: AppModel

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            NewItemCommands(model: model)
        }
        CommandMenu("Wallpaper") {
            WallpaperCommands(model: model)
        }
        SidebarCommands()
        CommandGroup(before: .sidebar) {
            SectionCommands(model: model)
        }
        CommandGroup(after: .pasteboard) {
            ListCommands()
        }
        // HIG: the Help menu opens the app's help; AutoPaper's is the website's help page (AudioPaper's HelpCommands).
        CommandGroup(replacing: .help) {
            HelpCommands()
        }
        // Where macOS apps put it: the app menu, after About.
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") { Updates.shared.checkForUpdates() }
        }
    }
}

/// The project website (GitHub Pages, built from `site/`): Help ▸ AutoPaper Help and Settings → About's links.
enum Website {
    static let home = URL(string: "https://msitarzewski.github.io/AutoPaper/")!
    static let help = home.appending(path: "help.html")
    static let privacy = home.appending(path: "privacy.html")
}

private struct HelpCommands: View {
    @Environment(\.openURL) private var openURL

    var body: some View {
        Button("AutoPaper Help") { openURL(Website.help) }
            .keyboardShortcut("?", modifiers: .command)
    }
}

private struct NewItemCommands: View {
    let model: AppModel

    var body: some View {
        Button("New Mood") {
            model.showMainWindow()
            model.newMood()
        }
        .keyboardShortcut("n", modifiers: .command)
        .disabled(model.phase != .ready)
        // The selected mood's name is the window's title in Moods; this edits it there (as File ▸ Rename… does in
        // document apps), so renaming never needs the pointer.
        Button("Rename Mood…") {
            if let id = model.moodSelection { model.startRenaming(id) }
        }
        .disabled(model.section != .moods || model.mood(model.moodSelection) == nil)
        Button("New Wallpaper Now") { model.newWallpaperNow() }
            .keyboardShortcut("n", modifiers: [.option, .command])
            .disabled(model.phase != .ready || model.isWorking)
    }
}

private struct WallpaperCommands: View {
    let model: AppModel

    var body: some View {
        let current = model.current
        Toggle("Like", isOn: Binding(
            get: { current?.rating == .liked },
            set: { _ in if let current { model.rate(current, .liked) } }
        ))
        .disabled(current == nil)
        Toggle("Dislike", isOn: Binding(
            get: { current?.rating == .disliked },
            set: { _ in if let current { model.rate(current, .disliked) } }
        ))
        .disabled(current == nil)
        Button("Make an Echo") { if let current { model.makeEcho(of: current.id) } }
            .disabled(current == nil || model.isWorking)
        Divider()
        // ⌘. here; Esc is the toolbar's Stop (Safari's reload/stop, user 2026-10-06), so it only stops what's being
        // made while Now, where the stage shows, or a mood's detail is in front.
        Button("Cancel New Wallpaper") { model.cancel() }
            .keyboardShortcut(".", modifiers: .command)
            .disabled(!model.isGenerating)
        Divider()
        let paused = model.settings?.paused ?? false
        Button(paused ? "Resume New Wallpapers" : "Pause New Wallpapers") { model.setPaused(!paused) }
            .disabled(model.settings == nil)
        Button("Restore My Wallpaper") { model.restoreMyWallpaper() }
            .disabled(!model.autoPaperShowing)
    }
}

private struct SectionCommands: View {
    let model: AppModel

    var body: some View {
        ForEach(Array(MainSection.allCases.enumerated()), id: \.element) { index, section in
            Button(section.title) {
                model.section = section
                model.showMainWindow()
            }
            .keyboardShortcut(KeyEquivalent(Character(String(index + 1))), modifiers: .command)
        }
        Divider()
    }
}

/// Reorders the row selected in the focused list (a mood in Moods' list, or a keyword in a mood's keywords) and
/// sets a keyword's weight. Published by the lists as focused values; the weight is here because inside a list the
/// arrow keys select rows, not segments.
private struct ListCommands: View {
    @FocusedValue(\.reorderable) private var row
    @FocusedValue(\.selectedKeyword) private var keyword

    var body: some View {
        Divider()
        Button(row.map { "Move \($0.noun) Up" } ?? "Move Up") { row?.moveUp() }
            .keyboardShortcut(.upArrow, modifiers: [.command, .option])
            .disabled(row?.canMoveUp != true)
        Button(row.map { "Move \($0.noun) Down" } ?? "Move Down") { row?.moveDown() }
            .keyboardShortcut(.downArrow, modifiers: [.command, .option])
            .disabled(row?.canMoveDown != true)
        // A titled section rather than a submenu: SwiftUI leaves a submenu enabled even with nothing selected, while
        // each item here is enabled only when a keyword is.
        Section("Keyword Weight") {
            ForEach(Array(KeywordWeight.all.enumerated()), id: \.element) { index, weight in
                Toggle(weight.title, isOn: Binding(
                    get: { keyword?.weight == weight },
                    set: { _ in keyword?.setWeight(weight) }
                ))
                .keyboardShortcut(KeyEquivalent(Character(String(index + 1))), modifiers: [.control, .command])
                .disabled(keyword == nil)
            }
        }
    }
}

/// The selected row of a reorderable list, and moving it.
struct ReorderableRow {
    /// "Mood" or "Keyword", for the menu items' titles.
    let noun: String
    let canMoveUp: Bool
    let canMoveDown: Bool
    let moveUp: @MainActor () -> Void
    let moveDown: @MainActor () -> Void
}

/// The keyword selected in a mood's keywords, and setting its weight.
struct SelectedKeyword {
    let weight: KeywordWeight
    let setWeight: @MainActor (KeywordWeight) -> Void
}

extension FocusedValues {
    @Entry var reorderable: ReorderableRow?
    @Entry var selectedKeyword: SelectedKeyword?
}
