import Foundation

// Small decisions the app makes about the Dock, the desktop and the schedule, kept apart from AppKit and the engine
// so the unit tests (which compile only this folder) can check them.

/// AutoPaper is a menu bar utility: in the Dock only while one of its windows is open, or when its menu bar item is
/// hidden, so there's always a way in (AudioPaper's `updateDockPresence`).
enum DockPolicy {
    static func needsDock(openWindows: Int, showInMenuBar: Bool) -> Bool {
        openWindows > 0 || !showInMenuBar
    }
}

/// How AutoPaper puts its wallpapers on the desktop: Settings → General → Show wallpapers (app-spec 3a; AudioPaper's
/// `WallpaperMode`, same author).
enum WallpaperMode: String, CaseIterable, Identifiable, Sendable {
    /// One click-through window per display, just above the desktop picture and below the icons. The person's own
    /// wallpaper (macOS's moving Aerials and dynamic ones too) is never touched, so pausing, quitting or Restore My
    /// Wallpaper uncovers it exactly. The default.
    case overlay
    /// The real desktop picture is set (`setDesktopImageURL`), so the wallpaper also shows in Mission Control and on
    /// the lock screen.
    case replace

    var id: Self { self }

    /// Where the choice is kept (the app's own preferences; the engine doesn't know about it).
    static let defaultsKey = "showWallpapers"

    static func stored(_ value: String?) -> WallpaperMode {
        value.flatMap(WallpaperMode.init(rawValue:)) ?? .overlay
    }

    var title: String {
        switch self {
        case .overlay: "Over my wallpaper"
        case .replace: "As my wallpaper"
        }
    }

    /// The picker's subtitle for the mode chosen.
    var explanation: String {
        switch self {
        case .overlay:
            "Your own wallpaper stays as it is underneath, including macOS's moving ones, and shows again when you quit AutoPaper or choose Restore My Wallpaper."
        case .replace:
            "Replaces your desktop picture, so AutoPaper's wallpapers also show in Mission Control and on the lock screen. macOS's moving wallpapers may not come back exactly."
        }
    }
}

/// Over my wallpaper: when AutoPaper's overlay covers the person's own wallpaper, and how it fades.
enum OverlayRules {
    /// What can uncover the person's wallpaper or cover it again. Pausing isn't one: it only stops new wallpapers
    /// (user, 2026-10-06).
    enum Event: Equatable {
        case restoreMyWallpaper
        /// A wallpaper the person made or asked for (New Wallpaper Now, a scheduled new one, Show on Desktop).
        case shown
        /// A liked one brought back by the engine instead of a new one: like a revisit in As my wallpaper mode, it
        /// doesn't cover a wallpaper the person uncovered themselves.
        case broughtBack
    }

    /// Whether the person's own wallpaper is uncovered after `event` (`before`: whether it was).
    static func uncovered(after event: Event, before: Bool) -> Bool {
        switch event {
        case .restoreMyWallpaper: true
        case .shown: false
        case .broughtBack: before
        }
    }

    /// The overlay shows AutoPaper's wallpaper only in Over my wallpaper mode, with a wallpaper to show, and not
    /// while the person's own is uncovered.
    static func covers(mode: WallpaperMode, hasWallpaper: Bool, uncovered: Bool) -> Bool {
        mode == .overlay && hasWallpaper && !uncovered
    }

    /// A pure opacity cross-fade (AudioPaper's 1.6 s); with Reduce Motion a short dissolve, which is what the HIG
    /// suggests in place of longer transitions.
    static func fadeDuration(reduceMotion: Bool) -> TimeInterval {
        reduceMotion ? 0.3 : 1.6
    }
}

/// Switching Show wallpapers releases the old way (app-spec 3a): what happens to the wallpaper showing now.
enum ModeSwitch: Equatable {
    /// Over → As: the wallpaper the overlay shows becomes the desktop picture, then the overlay fades out.
    /// As → Over: the overlay covers the desktop with the current wallpaper, then the person's own pictures are put
    /// back underneath it.
    case carryOver
    /// Nothing of AutoPaper's was showing (the person's own picture, after Restore My Wallpaper): the new mode
    /// starts uncovered. As → Over still puts the person's own pictures back where AutoPaper's renders remain.
    case leaveUncovered

    /// Paused or not: pausing keeps AutoPaper's wallpaper showing (user, 2026-10-06), so it carries over too.
    static func plan(from old: WallpaperMode, to new: WallpaperMode, autoPaperShowing: Bool) -> ModeSwitch? {
        guard old != new else { return nil }
        return autoPaperShowing ? .carryOver : .leaveUncovered
    }
}

/// Whose picture a display shows.
enum DesktopRules {
    /// AutoPaper may put its current wallpaper back on a display (launch, a Space change, a new display, a deleted
    /// wallpaper, a liked one brought back) only when that display shows no picture, a file that's gone, or one of
    /// AutoPaper's own renders. Anything else is a picture the person chose, and only a new wallpaper replaces it.
    static func mayReplace(picture: URL?, fileExists: Bool, rendersFolder: URL?) -> Bool {
        guard let picture, picture.isFileURL else { return true }
        guard fileExists else { return true }
        return isRender(picture, in: rendersFolder)
    }

    /// The file is inside the engine's renders folder (symlinks resolved, so /private/var and /var agree).
    static func isRender(_ file: URL, in folder: URL?) -> Bool {
        guard let folder else { return false }
        let base = folder.standardizedFileURL.resolvingSymlinksInPath().path(percentEncoded: false)
        let path = file.standardizedFileURL.resolvingSymlinksInPath().path(percentEncoded: false)
        return path.hasPrefix(base.hasSuffix("/") ? base : base + "/")
    }

    /// What to put back for a picture macOS reports at `url` (AudioPaper's `restorableWallpaper`): the file itself
    /// when it exists. macOS's own pictures are reported as a file in `com.apple.mobileAssetDesktop` that may not
    /// exist; those map back to their descriptor in Desktop Pictures, which can be set again, dynamic behaviour
    /// included. Nil when there's nothing that can be set again (an Aerial, say).
    static func restorablePicture(
        for url: URL,
        fileExists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) },
        systemPictures: [URL] = [URL(filePath: "/System/Library/Desktop Pictures"), URL(filePath: "/Library/Desktop Pictures")]
    ) -> URL? {
        guard url.isFileURL else { return nil }
        if fileExists(url.path(percentEncoded: false)) { return url }
        let name = url.deletingPathExtension().lastPathComponent
        return systemPictures
            .map { $0.appending(path: name + ".madesktop") }
            .first { fileExists($0.path(percentEncoded: false)) }
    }
}

/// The status line under the wallpaper when the person's own picture is what the desktop shows.
enum DesktopStatus {
    static func ownPictureLine(mode: WallpaperMode, ownPictureShowing: Bool, paused: Bool) -> String? {
        guard ownPictureShowing else { return nil }
        switch mode {
        case .overlay:
            return paused
                ? "Your own wallpaper is showing. New wallpapers are paused, so it stays until you make one."
                : "Your own wallpaper is showing. AutoPaper's next new wallpaper covers it again."
        case .replace:
            return "Your own desktop picture is showing. AutoPaper will replace it with its next new wallpaper."
        }
    }
}

/// When the app's timer asks the engine for a scheduled wallpaper.
enum ScheduleRules {
    /// The engine budgets by UTC months, independently of the wallpaper cadence or the person's time zone.
    static func nextBudgetMonth(after now: Date) -> Date? {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0)!
        return calendar.dateInterval(of: .month, for: now)?.end
    }

    /// The engine's `next_start` (the due time less how long a wallpaper takes here, so it's ready on time), else
    /// `next_due`; never before the hold after launch, nor within `minimumGap` of the last attempt, so nothing can
    /// make the timer spin. Nil (paused, Only When I Ask, or not started) disarms it.
    static func timerDate(armed: Bool, nextStart: Date?, nextDue: Date?, holdUntil: Date?, lastAttempt: Date?, minimumGap: TimeInterval) -> Date? {
        guard armed, var date = nextStart ?? nextDue else { return nil }
        if let holdUntil { date = max(date, holdUntil) }
        if let lastAttempt { date = max(date, lastAttempt.addingTimeInterval(minimumGap)) }
        return date
    }
}

/// The main window's sections (View menu and sidebar).
enum MainSection: String, CaseIterable, Identifiable {
    case now, moods, history, console

    var id: Self { self }

    var title: String {
        switch self {
        case .now: "Now"
        case .moods: "Moods"
        case .history: "History"
        case .console: "Console"
        }
    }

    var symbol: String {
        switch self {
        case .now: "photo"
        case .moods: "rectangle.stack"
        case .history: "clock"
        case .console: "terminal"
        }
    }
}

/// A row of the main window's sidebar: a section, or one mood in Moods' disclosure group (user, 2026-10-06).
enum SidebarItem: Hashable {
    case section(MainSection)
    case mood(String)

    /// The row that marks where the window is: while Moods shows a mood, that mood's own row (or the Moods row
    /// while the group is closed, so the mark never sits on a hidden row); otherwise the section's row.
    static func showing(section: MainSection, mood: String?, moodsExpanded: Bool) -> SidebarItem {
        if section == .moods, let mood, moodsExpanded { return .mood(mood) }
        return .section(section)
    }

    /// Where choosing this row goes, as the section and the mood selected in Moods. The Moods row itself shows the
    /// summary of every mood (no mood selected); a mood's row shows that mood; Now and History keep the mood
    /// selected for when Moods comes back (View ▸ Moods).
    func destination(keeping mood: String?) -> (section: MainSection, mood: String?) {
        switch self {
        case .section(.moods): (.moods, nil)
        case .section(let section): (section, mood)
        case .mood(let id): (.moods, id)
        }
    }
}

/// Now's toolbar button, like Safari's reload/stop (user, 2026-10-06): New Wallpaper Now (⌘R) while nothing is
/// being made, Stop (Esc) while one is. One button, so keyboard focus stays on it when it changes. A mood's detail has
/// it too (user, 2026-10-06): the same for the current mood; for any other, Use This Mood and Make a New Wallpaper
/// (`otherMood`), which makes that mood current first.
enum MakeOrStop: Equatable {
    case make(enabled: Bool, otherMood: Bool = false)
    /// Disabled while a finished wallpaper is only being put on the desktop (nothing left to stop).
    case stop(enabled: Bool)

    init(ready: Bool, working: Bool, cancellable: Bool, otherMood: Bool = false) {
        self = working ? .stop(enabled: cancellable) : .make(enabled: ready, otherMood: otherMood)
    }

    var title: String {
        switch self {
        case .make(_, otherMood: false): "New Wallpaper Now"
        case .make(_, otherMood: true): "Use This Mood and Make a New Wallpaper"
        case .stop: "Stop"
        }
    }

    var symbol: String {
        switch self {
        case .make: "arrow.clockwise"
        case .stop: "xmark"
        }
    }

    var help: String {
        switch self {
        case .make: "\(title) (⌘R)"
        case .stop: "Stop Making This Wallpaper (Esc)"
        }
    }

    var isEnabled: Bool {
        switch self {
        case .make(let enabled, _), .stop(let enabled): enabled
        }
    }
}

/// History's two layouts (the toolbar's Grid | Gallery, like Finder's view options).
enum HistoryLayout: String, CaseIterable, Identifiable {
    case grid, gallery

    var id: Self { self }

    var title: String {
        switch self {
        case .grid: "Grid"
        case .gallery: "Gallery"
        }
    }

    var symbol: String {
        switch self {
        case .grid: "square.grid.2x2"
        case .gallery: "squares.below.rectangle"
        }
    }
}

/// The Gallery's filmstrip: where the arrow keys move the selection, and when to load the next page.
enum Filmstrip {
    /// The index after moving `step` from `index` (nil: nothing selected yet, so the first), kept within the
    /// `count` items; nil when there are none.
    static func move(from index: Int?, by step: Int, count: Int) -> Int? {
        guard count > 0 else { return nil }
        guard let index else { return 0 }
        return min(max(index + step, 0), count - 1)
    }

    /// Load the next page once the item at `index` is this close to the end of what's loaded.
    static let lookahead = 8

    static func needsMore(at index: Int, count: Int, exhausted: Bool) -> Bool {
        !exhausted && index >= count - lookahead
    }
}

/// What the stage capsule shows while AutoPaper works.
struct WorkPresentation: Equatable {
    /// The stage in words ("Painting…").
    let stage: String
    /// How far along (0–1) when the engine has a real number, else nil (an indeterminate spinner).
    let fraction: Float?
    /// "About 6 minutes left", when the engine knows.
    let timeLeft: String?
    /// Cancel is offered: only while a wallpaper is being made (an engine call `cancel` can stop), not while one
    /// already made is put on the desktop.
    let cancellable: Bool

    init(stage: String?, fraction: Float?, secondsLeft: UInt32?, generating: Bool) {
        self.stage = stage ?? (generating ? "Making a new wallpaper…" : "Preparing for your displays…")
        self.fraction = fraction.map { min(max($0, 0), 1) }
        timeLeft = Formatting.timeLeft(seconds: secondsLeft)
        cancellable = generating
    }

    /// What VoiceOver says for the ring and the line under the stage: "40 percent, about 6 minutes left".
    var spokenProgress: String? {
        let parts = [fraction.map(Formatting.percent), timeLeft.map { $0.prefix(1).lowercased() + $0.dropFirst() }].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }

    /// The stage with time left, on one line (the menu bar menu): "Painting… (about 6 minutes left)".
    var menuLine: String {
        guard let timeLeft else { return stage }
        return "\(stage) (\(timeLeft.prefix(1).lowercased() + timeLeft.dropFirst()))"
    }
}

/// What "What's on My Desktop" answers.
enum DesktopAnswer {
    /// AutoPaper's description of its wallpaper; when a display shows the person's own picture, it says so first
    /// rather than describing a wallpaper that isn't there.
    static func text(description: String?, title: String?, ownPictureShowing: Bool) -> String {
        guard let description, !description.isEmpty else {
            return ownPictureShowing ? "Your own picture is on the desktop." : "There's no AutoPaper wallpaper on the desktop yet."
        }
        guard ownPictureShowing else { return description }
        return "Your own picture is on the desktop. AutoPaper's last wallpaper was “\(title ?? description)”."
    }
}

/// Menu item titles don't wrap, so long text is cut at a word with "…" (the full text stays in the window).
enum MenuText {
    static func short(_ text: String, limit: Int = 50) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > limit else { return trimmed }
        let cut = trimmed.prefix(limit)
        let atWord = cut.lastIndex(of: " ").map { cut[..<$0] } ?? cut
        return atWord.trimmingCharacters(in: .punctuationCharacters.union(.whitespaces)) + "…"
    }
}
