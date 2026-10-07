import AppKit
import AutopaperCore
import Observation
import SwiftUI

enum WindowID {
    static let main = "main"
    static let welcome = "welcome"
}

/// A status line under the wallpaper: something went wrong, or a past wallpaper came back.
struct Notice: Equatable {
    enum Kind { case error, info }
    let kind: Kind
    let text: String
    /// A problem with a setting (a key, the painter, the budget, a server address…): shown as its line and one link
    /// to the fix instead of `text` (spec 6a).
    var link: SettingsProblem?
}

/// AutoPaper's state and actions, shared by the menu bar menu, the window, Settings, App Intents and notifications.
/// Everything that decides *what* to make is the engine's; this holds view state, the timer and the desktop.
@MainActor
@Observable
final class AppModel {
    static let shared = AppModel()

    enum Phase: Equatable {
        case starting
        case ready
        case failed(String)
    }

    private(set) var phase: Phase = .starting
    @ObservationIgnored private(set) var core: CoreBridge?

    /// The wallpaper on the desktop, and the engine's description of it (title, summary, echo note).
    private(set) var current: Generation?
    private(set) var currentDescription = ""
    /// A wallpaper is being made (or put on the desktop); `stage` says where it is. A scheduled check counts only
    /// once it reports a stage, so a check that finds nothing due never flashes progress.
    var isWorking: Bool { manualInFlight || (scheduledInFlight && stage != nil) }
    /// A wallpaper is being made (not just put on the desktop), so Cancel can stop it.
    var isGenerating: Bool { !presenting && (manualGenerating || (scheduledInFlight && stage != nil)) }
    private(set) var stage: ProgressStage?
    /// While painting, how far along (0–1) and about how long is left, when the engine has real numbers for them
    /// (`ProgressDetail`); nil otherwise.
    private(set) var progressFraction: Float?
    private(set) var secondsLeft: UInt32?
    /// Something the person asked for (New Wallpaper Now, an echo, a replacement, Show on Desktop) is running.
    private var manualInFlight = false
    /// …and it's an engine generation (not Show on Desktop).
    private var manualGenerating = false
    /// A finished wallpaper is being rendered and set; nothing left to cancel.
    private var presenting = false
    /// The timer's `run_if_due` is running.
    private var scheduledInFlight = false
    private(set) var notice: Notice?
    private(set) var nextDue: Date?
    /// When the timer asks for the next scheduled wallpaper: `next_due` less how long one takes here.
    private(set) var nextStart: Date?
    private(set) var spend: SpendSummary?
    /// The engine's settings. Pausing only stops new wallpapers: the one showing stays, in both ways of showing
    /// wallpapers (user, 2026-10-06); only Restore My Wallpaper (and quitting, over the person's wallpaper) uncovers
    /// their own.
    private(set) var settings: EngineSettings?
    /// Every mood, in the person's order (each with its keywords); exactly one is active.
    private(set) var moods: [Mood] = []
    private(set) var narrow = false
    private(set) var taste: TasteSummary?
    private(set) var memory: MemoryStatus?
    /// Bumped whenever a wallpaper is made, rated, shown or deleted, so History reloads.
    private(set) var historyRevision = 0
    /// A settings change the engine refused, shown by the Settings window.
    var settingsError: String?
    /// Bumped when a key is saved or removed in Accounts, so what depends on keys (Providers' model lists) reloads.
    var keysChanged = 0
    /// The Keychain account whose field Accounts focuses next (set by a key link).
    var accountToFocus: String?
    /// A keyword or mood change that failed for a reason other than what was typed (moving, removing, a weight,
    /// switching), shown as an alert by the Moods view. Mistakes in what was typed are shown next to the field.
    var keywordError: String?
    /// The person's own picture is what the desktop shows, though AutoPaper has a wallpaper: As my wallpaper, a display
    /// shows a picture they chose (AutoPaper leaves it until its next new wallpaper); Over my wallpaper, the overlay is
    /// uncovered (Restore My Wallpaper). Now says so, and What's on My Desktop.
    private(set) var ownPictureShowing = false
    /// AutoPaper's wallpaper is on the desktop: the overlay covers it, or a display's picture is one of AutoPaper's
    /// renders. Restore My Wallpaper is offered then.
    private(set) var autoPaperShowing = false
    /// Settings → General → Show wallpapers (app-spec 3a): over the person's own wallpaper (the default) or as it.
    /// Changing it releases the old way (`wallpaperModeChanged`).
    var wallpaperMode: WallpaperMode {
        didSet {
            defaults.set(wallpaperMode.rawValue, forKey: WallpaperMode.defaultsKey)
            if wallpaperMode != oldValue { wallpaperModeChanged(from: oldValue) }
        }
    }
    /// Over my wallpaper: the person's own wallpaper is uncovered (Restore My Wallpaper) until AutoPaper
    /// shows a new wallpaper (`OverlayRules`). Kept across launches, so a login doesn't cover it again.
    private(set) var overlayUncovered: Bool {
        didSet { defaults.set(overlayUncovered, forKey: Keys.overlayUncovered) }
    }
    /// The Providers section whose server address field Settings focuses next (set by an address link).
    var providersFocus: ProviderJob?
    /// The main window's section (View menu ⌘1–⌘3, the sidebar).
    var section: MainSection = .now
    /// The mood selected in Moods' list (its detail shows on the right).
    var moodSelection: Mood.ID?
    /// A mood whose name field the detail should focus (New Mood, Rename…); cleared once focused.
    var moodToName: Mood.ID?
    /// A keyword (its text) the selected mood's keyword list should select, set by a keyword problem's link; cleared
    /// once selected.
    var keywordToSelect: String?
    /// A mood waiting for the person to confirm Delete… (the list's dialog asks).
    var moodToDelete: Mood.ID?
    /// History's mood filter (nil: all moods), set by "Show All in History" in a mood's detail.
    var historyMood: Mood.ID?
    /// A wallpaper waiting for the person to confirm Delete… (History, the Gallery, a mood's wallpapers); the main
    /// window asks.
    var wallpaperToDelete: Generation?
    /// The wallpaper whose original and echoes the main window's sheet shows (Show Original and Echoes).
    var lineageOf: Generation?
    /// What a wallpaper action couldn't do (deleting), shown as an alert by the main window.
    var wallpaperProblem: String?
    /// Model names by id from the providers' own lists (`list_models`' display names), read this session: the
    /// provenance line ("Painted by Nano Banana 2") prefers them.
    private(set) var modelNames: [String: String] = [:]

    // Preferences that are the app's own (the engine keeps everything else).
    /// The menu bar extra is shown. Changed by the Settings toggle, or by macOS when the person ⌘-drags the item
    /// out of the menu bar (`MenuBarExtra(isInserted:)`): either way the Dock icon must follow at once, so there's
    /// always a way back in, whether or not Settings is open.
    var showInMenuBar: Bool {
        didSet {
            defaults.set(showInMenuBar, forKey: Keys.showInMenuBar)
            if showInMenuBar != oldValue { menuBarVisibilityChanged?() }
        }
    }
    /// Set by the app delegate: updates the Dock presence when `showInMenuBar` changes.
    @ObservationIgnored var menuBarVisibilityChanged: (@MainActor () -> Void)?
    /// The first-run welcome was finished or skipped.
    var welcomeDone: Bool {
        didSet { defaults.set(welcomeDone, forKey: Keys.welcomeDone) }
    }
    /// Scheduled wallpapers start only once the person has made one or finished the welcome, so installing
    /// AutoPaper never replaces the wallpaper before they've chosen anything.
    private(set) var scheduleArmed: Bool {
        didSet { defaults.set(scheduleArmed, forKey: Keys.scheduleArmed) }
    }

    @ObservationIgnored let desktop = DesktopService()
    @ObservationIgnored private var scheduler: Scheduler?
    @ObservationIgnored var openWindowAction: OpenWindowAction?
    @ObservationIgnored var dismissWindowAction: DismissWindowAction?
    @ObservationIgnored var openSettingsAction: OpenSettingsAction?
    @ObservationIgnored private var readyWaiters: [CheckedContinuation<Void, Never>] = []
    @ObservationIgnored private var displayObserver: (any NSObjectProtocol)?
    @ObservationIgnored private var appActiveObserver: (any NSObjectProtocol)?
    @ObservationIgnored private var displayChange: Task<Void, Never>?
    @ObservationIgnored private let defaults = UserDefaults.standard
    /// No scheduled attempt before this (see `reschedule`).
    @ObservationIgnored private var holdUntil: Date?
    @ObservationIgnored private var lastScheduledAttempt: Date?
    /// A disliked wallpaper whose replacement waits for what's being made now (only one runs at a time).
    @ObservationIgnored private var replaceAfterCurrentRun: String?
    /// Progress is announced once a stage has lasted a moment, and never the same stage twice in a row.
    @ObservationIgnored private var stageAnnouncement: Task<Void, Never>?
    @ObservationIgnored private var lastAnnouncedStage: ProgressStage?
    /// The mood names Siri and Shortcuts were last told about.
    @ObservationIgnored private var shortcutMoodNames: [String] = []

    enum Keys {
        static let showInMenuBar = "showInMenuBar"
        static let welcomeDone = "welcomeDone"
        static let scheduleArmed = "scheduleArmed"
        static let mainWindowOpen = "mainWindowOpen"
        static let overlayUncovered = "overlayUncovered"
    }

    private init() {
        defaults.register(defaults: [Keys.showInMenuBar: true])
        showInMenuBar = defaults.bool(forKey: Keys.showInMenuBar)
        welcomeDone = defaults.bool(forKey: Keys.welcomeDone)
        scheduleArmed = defaults.bool(forKey: Keys.scheduleArmed)
        wallpaperMode = WallpaperMode.stored(defaults.string(forKey: WallpaperMode.defaultsKey))
        overlayUncovered = defaults.bool(forKey: Keys.overlayUncovered)
        desktop.mode = wallpaperMode
    }

    // MARK: Launch

    /// Opens the engine off the main thread, puts the current wallpaper back on any display that doesn't show
    /// it, and starts the schedule.
    func start() {
        guard core == nil, phase == .starting else { return }
        Notifier.shared.register()
        let detail = ProgressDetailRelay { detail in
            // In order, on the main thread (the engine calls this on its own thread).
            DispatchQueue.main.async {
                MainActor.assumeIsolated { AppModel.shared.progressed(detail) }
            }
        }
        Task {
            do {
                let core = try await CoreBridge.open(secrets: .shared, detail: detail)
                self.core = core
                await opened(core)
            } catch {
                log.error("The engine didn't open: \(String(describing: error), privacy: .public)")
                phase = .failed(sentence(for: error) ?? "AutoPaper couldn't open its library.")
                // App Intents waiting for the engine get their answer ("couldn't open") instead of waiting forever.
                readyWaiters.forEach { $0.resume() }
                readyWaiters.removeAll()
            }
        }
    }

    /// Waits until the engine is open (App Intents can run before it is). False when it couldn't open.
    func ready() async -> Bool {
        if phase == .ready { return true }
        if case .failed = phase { return false }
        await withCheckedContinuation { readyWaiters.append($0) }
        return phase == .ready
    }

    private func opened(_ core: CoreBridge) async {
        await sendDisplayHint()
        await refreshAll()
        phase = .ready
        readyWaiters.forEach { $0.resume() }
        readyWaiters.removeAll()
        if let settings, settings.textProvider.kind != .demo || settings.imageProvider.kind != .demo {
            // Providers were set up already (an earlier version, or the CLI): no welcome. It may already be open
            // (its launch behaviour is decided before the engine opens), so it's closed.
            if !welcomeDone {
                welcomeDone = true
                dismissWindowAction?(id: WindowID.welcome)
            }
            scheduleArmed = true
        }
        if current != nil { scheduleArmed = true }
        displayObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.displaysChanged() }
        }
        holdUntil = .now.addingTimeInterval(Self.launchGrace)
        let scheduler = Scheduler { [weak self] in self?.runScheduled() }
        scheduler.start()
        self.scheduler = scheduler
        desktop.rendersDirectory = try? CoreBridge.dataDirectory().appending(path: "renders", directoryHint: .isDirectory)
        desktop.onChange = { [weak self] in self?.desktopChanged() }
        switch wallpaperMode {
        case .overlay:
            // A desktop picture AutoPaper set before (As my wallpaper, or a version before Show wallpapers) gets the
            // person's own back underneath; then the overlay covers it, unless they uncovered it (Restore My
            // Wallpaper). Paused or not: pausing only stops new wallpapers.
            desktop.releaseDesktop()
            if let current, !overlayUncovered {
                do {
                    try await desktop.show(current, using: core)
                } catch {
                    log.error("Covering the desktop failed: \(String(describing: error), privacy: .public)")
                }
            }
        case .replace:
            // Puts the current wallpaper back only where a display shows one of AutoPaper's own older renders (or a
            // file that's gone): a picture the person chose themselves stays until AutoPaper's next new wallpaper.
            if let current, desktop.displaysNeedingWallpaper() {
                do {
                    try await desktop.show(current, using: core, onlyReplacingAutoPaper: true)
                } catch {
                    log.error("Putting the current wallpaper back failed: \(String(describing: error), privacy: .public)")
                }
            }
        }
        desktopChanged()
        appActiveObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.desktopChanged() }
        }
        reschedule()
    }

    /// Re-checks whose picture the desktop shows (launch, Space changes, coming to the front, a mode switch).
    private func desktopChanged() {
        switch wallpaperMode {
        case .overlay:
            autoPaperShowing = desktop.isCovering
            ownPictureShowing = current != nil && !autoPaperShowing
        case .replace:
            autoPaperShowing = desktop.showsAutoPaperPicture()
            ownPictureShowing = current != nil && desktop.showsOwnPicture()
        }
    }

    // MARK: Show wallpapers (over or as the person's wallpaper)

    /// Restore My Wallpaper: the person's own wallpaper, until AutoPaper's next new one. Over my wallpaper, the
    /// overlay fades out; As my wallpaper, each display's own picture is put back where AutoPaper's shows.
    func restoreMyWallpaper() {
        switch wallpaperMode {
        case .overlay:
            overlayUncovered = OverlayRules.uncovered(after: .restoreMyWallpaper, before: overlayUncovered)
            desktop.uncover()
            desktopChanged()
            announce("Your own wallpaper is showing.", evenInBackground: true)
        case .replace:
            let all = desktop.releaseDesktop()
            if all {
                if notice?.link == .pictureNotPutBack { notice = nil }
                announce("Your own desktop picture is back.", evenInBackground: true)
            } else {
                let problem = SettingsProblem.pictureNotPutBack
                notice = Notice(kind: .error, text: problem.sentence, link: problem)
                announce(problem.spoken, evenInBackground: true)
            }
            desktopChanged()
            // macOS applies a desktop picture a moment later (WallpaperAgent): look again once it has.
            Task {
                try? await Task.sleep(for: .seconds(3))
                desktopChanged()
            }
        }
    }

    /// Show wallpapers changed: the wallpaper showing moves to the new way, then the old way is released (app-spec
    /// 3a). Over → As: it becomes the desktop picture, then the overlay fades out over it. As → Over: the overlay
    /// covers the desktop, then the person's own pictures go back underneath.
    private func wallpaperModeChanged(from old: WallpaperMode) {
        let new = wallpaperMode
        guard phase == .ready, let core,
              let plan = ModeSwitch.plan(from: old, to: new, autoPaperShowing: autoPaperShowing)
        else {
            desktop.mode = new
            return
        }
        desktop.mode = new
        let current = self.current
        Task {
            switch new {
            case .replace:
                if plan == .carryOver, let current {
                    do {
                        try await desktop.show(current, using: core)
                        // macOS applies the picture a moment later; the overlay stays until it has.
                        try? await Task.sleep(for: .seconds(1))
                    } catch {
                        log.error("Setting the desktop picture failed: \(String(describing: error), privacy: .public)")
                    }
                }
                desktop.uncover()
                overlayUncovered = false
            case .overlay:
                overlayUncovered = plan != .carryOver
                if plan == .carryOver, let current {
                    do {
                        try await desktop.show(current, using: core)
                    } catch {
                        log.error("Covering the desktop failed: \(String(describing: error), privacy: .public)")
                    }
                }
                desktop.releaseDesktop()
            }
            desktopChanged()
            try? await Task.sleep(for: .seconds(3))
            desktopChanged()
        }
    }

    // MARK: Making wallpapers

    func newWallpaperNow() {
        make { core, observer in try await core.engine.generate(trigger: .manual, observer: observer) }
    }

    /// Use This Mood and Make a New Wallpaper (a mood's detail, user 2026-10-06): makes the mood current, then a new
    /// wallpaper from it. If the switch fails, that's said and nothing is made.
    func newWallpaper(from id: Mood.ID) {
        Task {
            if let problem = await switchMood(id) {
                keywordError = problem
                return
            }
            newWallpaperNow()
        }
    }

    func makeEcho(of id: String) {
        make { core, observer in try await core.engine.makeEcho(id: id, observer: observer) }
    }

    func cancel() {
        core?.engine.cancel()
    }

    /// What the stage capsule and the menu show while AutoPaper works; nil when it isn't.
    var work: WorkPresentation? {
        guard isWorking else { return nil }
        return WorkPresentation(stage: stage?.title, fraction: progressFraction, secondsLeft: secondsLeft, generating: isGenerating)
    }

    /// A wallpaper the person asked for: made, then put on the desktop. Its outcome is announced even when
    /// AutoPaper isn't the active app (it may have been asked for from the menu bar).
    private func make(_ work: @escaping @Sendable (CoreBridge, ProgressObserver) async throws -> Generation) {
        guard let core, phase == .ready, !manualInFlight else { return }
        manualInFlight = true
        manualGenerating = true
        startRun()
        notice = nil
        scheduleArmed = true
        let observer = progressObserver()
        Task {
            var cancelled = false
            do {
                let generation = try await work(core, observer)
                try await present(generation, as: .new, evenInBackground: true)
                Notifier.shared.clearKeyWarning()
            } catch {
                cancelled = Self.isCancel(error)
                report(error, evenInBackground: true)
            }
            manualInFlight = false
            manualGenerating = false
            finishRun()
            replacePendingDislike(afterCancel: cancelled)
            await refreshStatus()
        }
    }

    /// A run starts: nothing from the last one shows.
    private func startRun() {
        stage = nil
        progressFraction = nil
        secondsLeft = nil
        lastAnnouncedStage = nil
    }

    /// Clears the stage once nothing is running.
    private func finishRun() {
        guard !manualInFlight, !scheduledInFlight else { return }
        stage = nil
        progressFraction = nil
        secondsLeft = nil
        presenting = false
        stageAnnouncement?.cancel()
        stageAnnouncement = nil
    }

    private static func isCancel(_ error: any Error) -> Bool {
        if case .Cancelled = error as? AutoPaperError { return true }
        return error is CancellationError
    }

    /// The timer, wake or unlock: a scheduled wallpaper if one is due (or a liked one brought back).
    func runScheduled() {
        guard let core, phase == .ready, scheduleArmed else { return }
        // What's being made restarts the schedule anyway; refreshStatus re-arms the timer when it's done.
        guard !scheduledInFlight, !manualInFlight else { return }
        if let holdUntil, holdUntil > .now { reschedule(); return }
        scheduledInFlight = true
        lastScheduledAttempt = .now
        startRun()
        let observer = progressObserver()
        Task {
            var cancelled = false
            do {
                if let shown = try await core.engine.runIfDue(observer: observer) {
                    notice = nil
                    try await present(shown.generation, as: shown.revisit.map(Presentation.revisit) ?? .new, evenInBackground: false)
                    switch shown.revisit {
                    case nil:
                        Notifier.shared.newWallpaper(shown.generation)
                    case .overBudget:
                        if let month = spend?.month { Notifier.shared.budgetSpent(month: month) }
                    default:
                        break
                    }
                }
            } catch {
                // A cancelled scheduled run fills its slot in the engine, so the next due time (re-read below)
                // has already moved on.
                cancelled = Self.isCancel(error)
                // Cancelling a scheduled one is something the person did, so it's said even from the menu bar.
                report(error, evenInBackground: cancelled)
                switch error as? AutoPaperError {
                case .BudgetReached:
                    if let month = spend?.month { Notifier.shared.budgetSpent(month: month) }
                case .MissingKey, .InvalidKey:
                    if let problem = KeyProblem(error, selection: keySelection(for: error)) { Notifier.shared.keyProblem(SettingsProblem(key: problem)) }
                default:
                    break
                }
            }
            scheduledInFlight = false
            finishRun()
            replacePendingDislike(afterCancel: cancelled)
            await refreshStatus()
        }
    }

    /// How a wallpaper came to be on the desktop, for what's announced.
    enum Presentation {
        /// Just made ("New wallpaper: …").
        case new
        /// Brought back by the engine instead of a new one (its reason is shown and announced).
        case revisit(RevisitReason)
        /// Put back by the person from History ("On your desktop: …").
        case shownAgain
    }

    /// Renders for every display, shows the wallpaper, then tells the engine it's showing. A wallpaper brought back by
    /// the engine (a revisit) isn't a new one, so it leaves a picture the person chose where it is (As my wallpaper)
    /// and doesn't cover a wallpaper they uncovered (Over my wallpaper); one they made or picked replaces it.
    private func present(_ generation: Generation, as presentation: Presentation, evenInBackground: Bool) async throws {
        guard let core else { return }
        presenting = true
        stage = .rendering
        progressFraction = nil
        secondsLeft = nil
        let broughtBack: Bool
        if case .revisit = presentation { broughtBack = true } else { broughtBack = false }
        let uncovered = OverlayRules.uncovered(after: broughtBack ? .broughtBack : .shown, before: overlayUncovered)
        if wallpaperMode == .replace || !uncovered {
            try await desktop.show(generation, using: core, onlyReplacingAutoPaper: broughtBack)
        }
        if wallpaperMode == .overlay { overlayUncovered = uncovered }
        let id = generation.id
        let (shown, description) = try await core.write { engine in
            try engine.markShown(id: id)
            return (try engine.generation(id: id), try engine.describe(id: id))
        }
        current = shown
        currentDescription = description
        historyRevision += 1
        desktopChanged()
        switch presentation {
        case .new:
            // Which providers and models made it (diagnostics: whether a model chosen in Settings was used).
            log.notice("\(Provenance.logLine(shown), privacy: .public)")
            announce("New wallpaper: \(shown.concept.title)", evenInBackground: evenInBackground)
        case .revisit(let reason):
            if reason == .overBudget {
                // The budget is a setting: its line comes with the link to raise it (spec 6a).
                let problem = SettingsProblem.overBudget(bringingBack: true)
                notice = Notice(kind: .info, text: problem.sentence, link: problem)
                announce(problem.spoken, evenInBackground: evenInBackground)
            } else {
                notice = Notice(kind: .info, text: Sentences.revisit(reason))
                announce(Sentences.revisit(reason), evenInBackground: evenInBackground)
            }
        case .shownAgain:
            announce("On your desktop: \(shown.concept.title)", evenInBackground: evenInBackground)
        }
    }

    private func progressObserver() -> ProgressRelay {
        ProgressRelay { stage in
            // In order, on the main thread (the engine calls this on its own thread).
            DispatchQueue.main.async {
                MainActor.assumeIsolated { AppModel.shared.progressed(stage) }
            }
        }
    }

    private func progressed(_ stage: ProgressStage) {
        guard manualInFlight || scheduledInFlight, stage != .done else { return }
        if stage != self.stage {
            progressFraction = nil
            secondsLeft = nil
        }
        self.stage = stage
        // A stage is announced only once it has lasted a moment, and never twice in a row: the engine may compose
        // and check memory more than once for one wallpaper, and Demo passes through every stage in a blink.
        stageAnnouncement?.cancel()
        stageAnnouncement = Task { [weak self] in
            try? await Task.sleep(for: Self.stageAnnouncementDelay)
            guard !Task.isCancelled, let self, self.stage == stage, self.lastAnnouncedStage != stage,
                  let title = stage.title
            else { return }
            self.lastAnnouncedStage = stage
            self.announce(title)
        }
    }

    /// The engine's progress in detail (every generation, from any call): the numbers for the ring and time left.
    /// Only real numbers are shown; a stage report without them clears the last ones.
    private func progressed(_ detail: ProgressDetail) {
        guard manualInFlight || scheduledInFlight, !presenting, detail.stage != .done else { return }
        guard stage == nil || detail.stage == stage else {
            // The stage moves on before its own stage report arrives: the old numbers no longer apply.
            if detail.fraction == nil { progressFraction = nil; secondsLeft = nil }
            return
        }
        progressFraction = detail.fraction
        secondsLeft = detail.secondsLeft
    }

    /// The sentence for an engine error, with the address in use when a local server isn't answering. `inSettings`
    /// when it's shown in Settings (a problem with a setting then doesn't say "in Settings → Providers").
    func sentence(for error: any Error, retrying: Bool = true, inSettings: Bool = false) -> String? {
        Sentences.error(error, retrying: retrying, inSettings: inSettings, addresses: addressesInUse)
    }

    /// The server address each chosen local or OpenAI-compatible provider talks to.
    private var addressesInUse: [ProviderKind: String] {
        var addresses: [ProviderKind: String] = [:]
        for selection in [settings?.imageProvider, settings?.textProvider].compactMap({ $0 }) {
            if let address = selection.baseUrl ?? selection.kind.defaultAddress { addresses[selection.kind] = address }
        }
        return addresses
    }

    /// The selection a key error is about (an OpenAI-compatible server's key belongs to its address).
    private func keySelection(for error: any Error) -> ProviderSelection? {
        let kind: ProviderKind
        switch error as? AutoPaperError {
        case .MissingKey(let provider), .InvalidKey(let provider): kind = provider
        default: return nil
        }
        return [settings?.textProvider, settings?.imageProvider].compactMap { $0 }.first { $0.kind == kind }
    }

    /// Shows what went wrong under the wallpaper and says it. A cancel shows nothing; it's announced only when
    /// the person asked for the cancel. A problem with a setting (a key, the painter, the budget, a server address)
    /// is its line and one link to the fix, and what's announced is what the link says (spec 6a), with no "try
    /// again later".
    private func report(_ error: any Error, evenInBackground: Bool = false) {
        log.error("\(String(describing: error), privacy: .public)")
        if let problem = settingsProblem(for: error) {
            notice = Notice(kind: .error, text: problem.sentence, link: problem)
            announce(problem.spoken, evenInBackground: evenInBackground)
            return
        }
        guard let sentence = sentence(for: error) else {
            if Self.isCancel(error) { announce("Cancelled.", evenInBackground: evenInBackground) }
            return
        }
        notice = Notice(kind: .error, text: sentence)
        announce(sentence, evenInBackground: evenInBackground)
    }

    /// The problem with a setting behind `error`, if it's one, worded for the providers in use.
    func settingsProblem(for error: any Error) -> SettingsProblem? {
        SettingsProblem(error, writing: settings?.textProvider, painting: settings?.imageProvider, ownWorkflow: settings?.comfyuiWorkflow != nil)
    }

    // MARK: The wallpaper showing

    /// Like or Dislike from a button or menu: choosing the rating it already has clears it.
    func rate(_ generation: Generation, _ rating: Rating) {
        rate(id: generation.id, rating, toggle: true, from: generation.rating)
    }

    /// Like or Dislike from App Intents and notifications: always sets the rating.
    func rate(id: String, _ rating: Rating, toggle: Bool, from previous: Rating? = nil) {
        guard let core else { return }
        let wanted = toggle && previous == rating ? Rating.unrated : rating
        Task {
            do {
                let replace = try await core.write { try $0.rate(id: id, rating: wanted) }
                await reloadGeneration(id)
                taste = try? await core.call { try $0.tasteSummary() }
                if replace {
                    if manualInFlight || scheduledInFlight {
                        // One wallpaper is made at a time. What's being made now usually takes its place anyway;
                        // if it doesn't (it failed), the replacement starts then.
                        replaceAfterCurrentRun = id
                    } else {
                        replaceDisliked()
                    }
                }
            } catch {
                report(error, evenInBackground: true)
            }
        }
    }

    private func replaceDisliked() {
        make { core, observer in try await core.engine.generate(trigger: .dislikeReplace, observer: observer) }
    }

    /// After a run: replaces a wallpaper disliked during it, if it's still the one showing (and still disliked).
    /// Not after a cancel: the person stopped what was being made.
    private func replacePendingDislike(afterCancel: Bool) {
        guard let id = replaceAfterCurrentRun, !manualInFlight, !scheduledInFlight else { return }
        replaceAfterCurrentRun = nil
        guard !afterCancel, current?.id == id, current?.rating == .disliked else { return }
        replaceDisliked()
    }

    /// "Show on Desktop" in History: brings a past wallpaper back (doesn't restart the schedule). Nothing to cancel:
    /// it's already made.
    func showOnDesktop(_ generation: Generation) {
        guard core != nil, !manualInFlight else { return }
        manualInFlight = true
        startRun()
        notice = nil
        Task {
            do {
                try await present(generation, as: .shownAgain, evenInBackground: true)
            } catch {
                report(error, evenInBackground: true)
            }
            manualInFlight = false
            finishRun()
            replacePendingDislike(afterCancel: false)
            await refreshStatus()
        }
    }

    func setPaused(_ paused: Bool) {
        updateSettings { $0.paused = paused }
    }

    /// Re-reads one generation after a change, updating what's showing.
    private func reloadGeneration(_ id: String) async {
        guard let core else { return }
        if current?.id == id, let fresh = try? await core.call({ engine in (try engine.generation(id: id), try engine.describe(id: id)) }) {
            current = fresh.0
            currentDescription = fresh.1
        }
        historyRevision += 1
    }

    // MARK: Settings

    /// Saves a settings change (controls in Settings, the menus). Shown at once; put back if the engine refuses it,
    /// with the reason in an alert in Settings, unless `refused` shows it itself (next to the field it's about) and
    /// returns true. Doesn't wait for the save: code that must act on the saved settings uses `saveSettings`.
    func updateSettings(
        _ change: (inout EngineSettings) -> Void,
        refused: (@MainActor (AutoPaperError) -> Bool)? = nil
    ) {
        guard let core, let previous = settings else { return }
        var next = previous
        change(&next)
        guard next != previous else { return }
        settings = next
        let wanted = next
        Task {
            do {
                try await save(wanted, replacing: previous, using: core)
            } catch {
                var shownByCaller = false
                if let refused, let engineError = error as? AutoPaperError { shownByCaller = refused(engineError) }
                if !shownByCaller {
                    settingsError = sentence(for: error, retrying: false, inSettings: true)
                }
            }
        }
    }

    /// Saves a settings change and returns once the engine has it (or throws why it didn't take it, with the change
    /// put back). `then` runs right after the save, in the same write, so it sees the new settings (pruning to a new
    /// storage limit); its error is thrown after the settings are saved.
    func saveSettings(
        _ change: (inout EngineSettings) -> Void,
        then follow: (@Sendable (Engine) throws -> Void)? = nil
    ) async throws {
        guard let core, let previous = settings else { return }
        var next = previous
        change(&next)
        guard next != previous else { return }
        settings = next
        try await save(next, replacing: previous, using: core, then: follow)
    }

    /// The write behind `updateSettings` and `saveSettings`. Writes run one at a time in the order they were asked
    /// for (`CoreBridge.write`), and this awaits its own, so whatever follows sees the saved settings.
    ///
    /// Surprise belongs to the active mood, and `update_settings` writes it to whichever mood is active when it runs.
    /// Settings never change it (the Moods pane uses `set_mood_surprise`), so the write takes the engine's own value
    /// at that moment: a copy read before a mood switch can't give the new mood the old one's Surprise.
    private func save(
        _ wanted: EngineSettings,
        replacing previous: EngineSettings,
        using core: CoreBridge,
        then follow: (@Sendable (Engine) throws -> Void)? = nil
    ) async throws {
        let result: (EngineSettings, (any Error)?)
        do {
            result = try await core.write { engine in
                var record = wanted
                record.surprise = try engine.settings().surprise
                try engine.updateSettings(settings: record)
                var followError: (any Error)?
                do { try follow?(engine) } catch { followError = error }
                return (try engine.settings(), followError)
            }
        } catch {
            if settings == wanted { settings = previous }
            await refreshStatus()
            throw error
        }
        let (saved, followError) = result
        if settings == wanted || settings?.surprise != saved.surprise { settings = saved }
        if saved.textProvider.kind != .demo || saved.imageProvider.kind != .demo {
            // A real provider chosen in Settings is the person's go-ahead, as at launch (`opened`): scheduled
            // wallpapers start now rather than after the next launch.
            scheduleArmed = true
        }
        await refreshStatus()
        if let followError { throw followError }
    }

    // MARK: Moods

    /// The mood in use: its keywords and Surprise make the next wallpaper.
    var activeMood: Mood? { moods.first(where: \.active) }

    /// The active mood's keywords (the welcome adds to them).
    var keywords: [Keyword] { activeMood?.keywords ?? [] }

    func mood(_ id: Mood.ID?) -> Mood? {
        guard let id else { return nil }
        return moods.first { $0.id == id }
    }

    /// Makes a mood the one in use. Nothing is made by itself: the next wallpaper uses it.
    func useMood(_ id: Mood.ID) {
        Task {
            if let problem = await switchMood(id) { keywordError = problem }
        }
    }

    /// Makes a mood the one in use and returns once that's saved: nil when it is (or already was), else what went
    /// wrong, in words. Switch Mood (App Intents) waits for it, so Siri never says it switched when it didn't.
    func switchMood(_ id: Mood.ID) async -> String? {
        guard let core else { return Sentences.internalProblem }
        guard let target = mood(id), !target.active else { return nil }
        do {
            try await core.write { try $0.setActiveMood(id: id) }
            await reloadMoods()
            // Surprise is the active mood's; the settings record now carries the new mood's.
            if let fresh = try? await core.call({ try $0.settings() }) { settings = fresh }
            announce("\(target.name) is the current mood.", evenInBackground: true)
            await refreshStatus()
            return nil
        } catch {
            await reloadMoods()
            return sentence(for: error) ?? Sentences.internalProblem
        }
    }

    /// What creating or renaming a mood did.
    enum MoodNamed: Equatable {
        case done(Mood)
        /// Refused (empty, too long, a name another mood has), with the sentence to show next to the field.
        case refused(String)
    }

    /// New Mood: "New mood" (or "New mood 2"…), at the end of the list, selected, with its name field focused. Not
    /// made active.
    func newMood() {
        createMood(named: MoodText.unique(MoodText.newMoodName, among: moods.map(\.name)), copying: nil)
    }

    /// Duplicate: the mood's keywords and Surprise under "<name> copy".
    func duplicateMood(_ id: Mood.ID) {
        guard let original = mood(id) else { return }
        createMood(named: MoodText.copyName(of: original.name, among: moods.map(\.name)), copying: id)
    }

    private func createMood(named name: String, copying source: Mood.ID?) {
        guard let core else { return }
        Task {
            do {
                let created = try await core.write { try $0.createMood(name: name, copyFrom: source) }
                await reloadMoods()
                section = .moods
                moodSelection = created.id
                moodToName = created.id
                announce(source == nil ? "New mood added." : "\(created.name) added.")
            } catch {
                keywordError = sentence(for: error)
            }
        }
    }

    func renameMood(_ id: Mood.ID, to name: String) async -> MoodNamed {
        guard let core else { return .refused(Sentences.internalProblem) }
        do {
            let renamed = try await core.write { try $0.renameMood(id: id, name: name) }
            await reloadMoods()
            return .done(renamed)
        } catch {
            await reloadMoods()
            return .refused(sentence(for: error) ?? Sentences.internalProblem)
        }
    }

    /// Deletes a mood and its keywords (its wallpapers stay in History). The engine refuses the last one.
    func deleteMood(_ id: Mood.ID) {
        guard let core, let doomed = mood(id) else { return }
        let index = moods.firstIndex { $0.id == id } ?? 0
        Task {
            do {
                try await core.write { try $0.deleteMood(id: id) }
                await reloadMoods()
                if moodSelection == id || moodSelection == nil {
                    // The row that took its place, else the one before it.
                    moodSelection = moods.indices.contains(index) ? moods[index].id : moods.last?.id
                }
                if doomed.active, let fresh = try? await core.call({ try $0.settings() }) { settings = fresh }
                announce("\(doomed.name) deleted." + (doomed.active ? " \(activeMood?.name ?? "") is the current mood." : ""))
                historyRevision += 1
                await refreshStatus()
            } catch {
                keywordError = sentence(for: error)
            }
        }
    }

    /// Moves a mood to `position` (0-based) in the list.
    func moveMood(_ id: Mood.ID, to position: Int) {
        guard let core, let from = moods.firstIndex(where: { $0.id == id }) else { return }
        let target = max(0, min(position, moods.count - 1))
        guard from != target else { return }
        let name = moods[from].name
        // Shown at once; the engine's order replaces it when the move is saved.
        moods = Reorder.moved(moods, from: from, to: target)
        Task {
            do {
                try await core.write { try $0.moveMood(id: id, toPosition: UInt32(target)) }
                announce("\(name) moved to position \(target + 1) of \(moods.count).")
            } catch {
                keywordError = sentence(for: error)
            }
            await reloadMoods()
        }
    }

    /// Sets a mood's Surprise (0–1). For the active mood that's also `settings().surprise`.
    func setSurprise(_ value: Float, of id: Mood.ID) {
        guard let core else { return }
        Task {
            do {
                try await core.write { try $0.setMoodSurprise(moodId: id, surprise: value) }
            } catch {
                keywordError = sentence(for: error)
            }
            await reloadMoods()
            if mood(id)?.active == true, let fresh = try? await core.call({ try $0.settings() }) { settings = fresh }
        }
    }

    /// The newest wallpapers made under a mood, for its detail.
    func recentWallpapers(of id: Mood.ID, limit: UInt32) async -> [Generation] {
        guard let core else { return [] }
        return (try? await core.call { try $0.historyByMood(filter: .all, moodId: id, limit: limit, offset: 0) }) ?? []
    }

    /// Rename…: Moods shows the mood and its title (the window's title) starts editing.
    func startRenaming(_ id: Mood.ID) {
        section = .moods
        moodSelection = id
        moodToName = id
    }

    /// What every mood has made (one per mood, in the person's order), for the summary and a mood's detail.
    func moodStats() async -> [MoodStats] {
        guard let core else { return [] }
        return (try? await core.call { try $0.moodStats() }) ?? []
    }

    /// Wallpapers per local day and mood over the last `MoodActivity.days` days, for the summary's chart.
    func activity() async -> [DayCount] {
        guard let core else { return [] }
        let bounds = MoodActivity.dayBounds()
        return (try? await core.call { try $0.activity(dayBounds: bounds) }) ?? []
    }

    /// Opens Moods in the main window with the current mood selected (the menu's Edit Moods…).
    func editMoods() {
        section = .moods
        moodSelection = activeMood?.id ?? moodSelection
        showMainWindow()
    }

    /// Opens a mood in Moods with `keyword` selected in its keywords, ready to reword (Return), weigh (⌃⌘1–3) or
    /// remove (Delete): the link of a keyword the writing model kept breaking (spec 6a). A mood deleted since then
    /// falls back to the current one.
    func showMood(_ id: Mood.ID, selecting keyword: String?) {
        section = .moods
        moodSelection = mood(id) != nil ? id : (activeMood?.id ?? moodSelection)
        keywordToSelect = keyword
        showMainWindow()
    }

    private func reloadMoods() async {
        guard let core else { return }
        if let (list, isNarrow) = try? await core.call({ engine in (try engine.moods(), try engine.keywordsAreNarrow()) }) {
            moods = list
            narrow = isNarrow
            moodsChanged()
        }
    }

    /// Keeps the selection on a mood that exists, and tells Siri and Shortcuts when the moods' names change (Switch
    /// Mood's phrases name them).
    private func moodsChanged() {
        if let selection = moodSelection, mood(selection) == nil { moodSelection = activeMood?.id }
        let names = moods.map(\.name)
        if names != shortcutMoodNames {
            shortcutMoodNames = names
            AutoPaperShortcuts.updateAppShortcutParameters()
        }
    }

    // MARK: Keywords (of any mood)

    /// What adding a keyword did.
    enum KeywordAdded {
        case added(Keyword)
        /// Already there (case-insensitively): not sent to the engine, which would change its weight.
        case duplicate(of: Keyword)
        /// Refused, with the sentence to show next to the field.
        case refused(String)
    }

    /// Adds a keyword (Must unless said otherwise) to a mood (nil: the active one), and says what happened (unless
    /// `announcing` is false: the welcome adds several at once and says the outcome itself).
    func addKeyword(_ text: String, to moodID: Mood.ID? = nil, weight: KeywordWeight = .must, announcing: Bool = true) async -> KeywordAdded? {
        guard let core else { return nil }
        let target = moodID.flatMap(mood) ?? activeMood
        if let existing = KeywordText.duplicate(of: text, in: target?.keywords ?? keywords) {
            if announcing { announce("\(existing.text) is already in this mood.") }
            return .duplicate(of: existing)
        }
        do {
            let id = target?.id
            let added = try await core.write { engine in
                if let id { try engine.addMoodKeyword(moodId: id, text: text, weight: weight) } else { try engine.addKeyword(text: text, weight: weight) }
            }
            await reloadMoods()
            if announcing { announce("\(added.text) added as \(added.weight.title).") }
            return .added(added)
        } catch {
            let sentence = sentence(for: error) ?? Sentences.internalProblem
            if announcing { announce(sentence) }
            return .refused(sentence)
        }
    }

    /// Renames a keyword. Returns the reason when the engine refuses (another keyword has that text, too long),
    /// so the row can put its text back and say (and announce) why next to it.
    func renameKeyword(_ keyword: Keyword, to text: String) async -> String? {
        guard let core else { return nil }
        let id = keyword.id
        do {
            _ = try await core.write { try $0.renameKeyword(id: id, text: text) }
            await reloadMoods()
            return nil
        } catch {
            await reloadMoods()
            return sentence(for: error) ?? Sentences.internalProblem
        }
    }

    /// Sets a keyword's weight. `announce` for the Edit menu and context menu, where nothing else says it (the
    /// segmented control announces its own change). Edit ▸ Undo puts the old weight back (HIG: support undo).
    func setWeight(_ keyword: Keyword, _ weight: KeywordWeight, announce: Bool = false, undoManager: UndoManager? = nil) {
        changeWeight(id: keyword.id, text: keyword.text, from: keyword.weight, to: weight, announce: announce, undoManager: undoManager)
    }

    private func changeWeight(id: String, text: String, from old: KeywordWeight, to weight: KeywordWeight, announce: Bool, undoManager: UndoManager?) {
        guard weight != old else { return }
        undoManager?.registerUndo(withTarget: self) { model in
            // Undoing isn't seen on the control the person used, so it's said.
            model.changeWeight(id: id, text: text, from: weight, to: old, announce: true, undoManager: undoManager)
        }
        undoManager?.setActionName("Keyword Weight")
        keywordChange("\(text) is now \(weight.title).", announce: announce, undoManager: undoManager) { try $0.setKeywordWeight(id: id, weight: weight) }
    }

    /// Moves a keyword to `position` (0-based) within its own mood. Edit ▸ Undo moves it back.
    func moveKeyword(_ keyword: Keyword, in moodID: Mood.ID, to position: Int, undoManager: UndoManager? = nil) {
        guard let moodIndex = moods.firstIndex(where: { $0.id == moodID }),
              let from = moods[moodIndex].keywords.firstIndex(where: { $0.id == keyword.id })
        else { return }
        let count = moods[moodIndex].keywords.count
        let target = max(0, min(position, count - 1))
        guard from != target else { return }
        // Shown at once; the engine's order replaces it when the move is saved.
        moods[moodIndex].keywords = Reorder.moved(moods[moodIndex].keywords, from: from, to: target)
        undoManager?.registerUndo(withTarget: self) { model in
            model.moveKeyword(keyword, in: moodID, to: from, undoManager: undoManager)
        }
        undoManager?.setActionName("Move Keyword")
        let id = keyword.id
        let said = "\(keyword.text) moved to position \(target + 1) of \(count)."
        keywordChange(said, announce: true, undoManager: undoManager) { try $0.moveKeyword(id: id, toPosition: UInt32(target)) }
    }

    /// Removes a keyword (the Delete key, Remove, the context menu: no confirmation, so Edit ▸ Undo puts it back with
    /// its weight and place, and Redo removes it again).
    func removeKeyword(_ keyword: Keyword, from moodID: Mood.ID, undoManager: UndoManager? = nil) {
        let position = mood(moodID)?.keywords.firstIndex { $0.id == keyword.id } ?? Int(keyword.position)
        remove(RemovedKeyword(id: keyword.id, text: keyword.text, weight: keyword.weight, moodID: moodID, position: position), undoManager: undoManager)
    }

    private func remove(_ keyword: RemovedKeyword, undoManager: UndoManager?) {
        guard let core else { return }
        undoManager?.registerUndo(withTarget: self) { model in model.putBack(keyword, undoManager: undoManager) }
        undoManager?.setActionName("Remove Keyword")
        Task {
            // A Redo right after an Undo waits for the keyword to be back (it has a new id then).
            await keyword.restoring?.value
            let id = keyword.id
            do {
                try await core.write { try $0.removeKeyword(id: id) }
                announce("\(keyword.text) removed.")
            } catch {
                undoManager?.removeAllActions(withTarget: self)
                keywordError = sentence(for: error)
            }
            await reloadMoods()
        }
    }

    private func putBack(_ keyword: RemovedKeyword, undoManager: UndoManager?) {
        guard let core else { return }
        undoManager?.registerUndo(withTarget: self) { model in model.remove(keyword, undoManager: undoManager) }
        undoManager?.setActionName("Remove Keyword")
        let (moodID, text, weight, position) = (keyword.moodID, keyword.text, keyword.weight, keyword.position)
        keyword.restoring = Task {
            do {
                let added = try await core.write { engine in
                    let added = try engine.addMoodKeyword(moodId: moodID, text: text, weight: weight)
                    try engine.moveKeyword(id: added.id, toPosition: UInt32(position))
                    return added
                }
                keyword.id = added.id
                announce("\(text) put back as \(weight.title).")
            } catch {
                undoManager?.removeAllActions(withTarget: self)
                keywordError = sentence(for: error)
            }
            await reloadMoods()
        }
    }

    /// A keyword change that isn't typed text: `done` is announced once it's saved (when `announce`); a failure
    /// is shown as an alert (and its undo forgotten, since there's nothing to undo).
    private func keywordChange(_ done: String, announce: Bool, undoManager: UndoManager? = nil, _ work: @escaping @Sendable (Engine) throws -> Void) {
        guard let core else { return }
        Task {
            do {
                try await core.write(work)
                if announce { self.announce(done) }
            } catch {
                undoManager?.removeAllActions(withTarget: self)
                keywordError = sentence(for: error)
            }
            await reloadMoods()
        }
    }

    // MARK: Taste, history and storage

    func resetTaste() {
        guard let core else { return }
        Task {
            do {
                try await core.write { try $0.resetTaste() }
                taste = try await core.call { try $0.tasteSummary() }
                announce("AutoPaper has forgotten what it learned.")
            } catch {
                settingsError = sentence(for: error)
            }
        }
    }

    /// One page of History, newest first: every mood's wallpapers (`moodID` nil) or one mood's.
    func history(_ filter: HistoryFilter, mood moodID: Mood.ID?, limit: UInt32, offset: UInt32) async throws -> [Generation] {
        guard let core else { return [] }
        return try await core.call { try $0.historyByMood(filter: filter, moodId: moodID, limit: limit, offset: offset) }
    }

    func lineage(of id: String) async throws -> [Generation] {
        guard let core else { return [] }
        return try await core.call { try $0.lineage(id: id) }
    }

    /// What History's tiles need beyond the record, for a whole page in one engine call: each one's spoken
    /// description (the engine's `describe` plus the rating in AutoPaper's words) and whether it has an original or
    /// echoes to show.
    struct TileInfo: Equatable, Sendable {
        let spoken: String
        let hasEchoes: Bool
    }

    func tileInfo(for generations: [Generation]) async -> [String: TileInfo] {
        guard let core, !generations.isEmpty else { return [:] }
        let items = generations.map { ($0.id, $0.concept.title, $0.rating) }
        return (try? await core.call { engine in
            var info: [String: TileInfo] = [:]
            for (id, title, rating) in items {
                let description = (try? engine.describe(id: id)) ?? title
                info[id] = TileInfo(spoken: Spoken.wallpaper(description: description, rating: rating),
                                    hasEchoes: (try? engine.hasEchoes(id: id)) ?? false)
            }
            return info
        }) ?? [:]
    }

    /// Spoken description of what's showing now.
    var currentSpokenDescription: String {
        guard let current else { return "" }
        return Spoken.wallpaper(description: currentDescription.isEmpty ? current.concept.title : currentDescription, rating: current.rating)
    }

    /// A wallpaper's actions (History's grid and Gallery, a mood's wallpapers, the summary's thumbnails): one
    /// builder, so every place offers the same menu. Delete… and Show Original and Echoes are presented by the main
    /// window (`wallpaperToDelete`, `lineageOf`).
    var wallpaperActions: HistoryActions {
        HistoryActions(
            showOnDesktop: { [weak self] in self?.showOnDesktop($0) },
            rate: { [weak self] in self?.rate($0, $1) },
            makeEcho: { [weak self] generation in
                self?.makeEcho(of: generation.id)
                self?.section = .now
            },
            showLineage: { [weak self] in self?.lineageOf = $0 },
            showInFinder: { generation in
                if let path = generation.imagePath ?? generation.thumbPath {
                    NSWorkspace.shared.activateFileViewerSelecting([URL(filePath: path)])
                }
            },
            delete: { [weak self] in self?.wallpaperToDelete = $0 },
            canMake: phase == .ready && !isWorking
        )
    }

    /// Deletes the wallpaper waiting in `wallpaperToDelete` (the person confirmed); a failure is shown as an alert.
    func deleteConfirmed(_ generation: Generation) {
        Task {
            if let problem = await delete(generation) { wallpaperProblem = problem }
        }
    }

    func delete(_ generation: Generation) async -> String? {
        guard let core else { return nil }
        let id = generation.id
        do {
            try await core.write { try $0.deleteGeneration(id: id) }
            historyRevision += 1
            if current?.id == id {
                // The one on the desktop was deleted: put back the one shown before it, and tell the engine it's
                // showing again (as any wallpaper put on the desktop is). A display showing a picture the person
                // chose keeps it, and so does an uncovered one (Over my wallpaper).
                current = try await core.call { try $0.current() }
                if let previous = current {
                    let previousID = previous.id
                    currentDescription = (try? await core.call { try $0.describe(id: previousID) }) ?? ""
                    do {
                        if wallpaperMode == .replace || !overlayUncovered {
                            try await desktop.show(previous, using: core, onlyReplacingAutoPaper: true)
                        }
                        current = try await core.write { engine in
                            try engine.markShown(id: previousID)
                            return try engine.generation(id: previousID)
                        }
                    } catch {
                        log.error("Putting back the previous wallpaper failed: \(String(describing: error), privacy: .public)")
                    }
                } else {
                    currentDescription = ""
                }
                historyRevision += 1
                desktopChanged()
            }
            return nil
        } catch {
            return sentence(for: error)
        }
    }

    func storageUsage() async -> StorageUsage? {
        guard let core else { return nil }
        return try? await core.call { try $0.storageUsage() }
    }

    /// Saves the storage limit and prunes to it in the same write, so the prune can't run with the old limit.
    func setStorageLimit(_ megabytes: UInt32) async {
        do {
            try await saveSettings({ $0.storageLimitMb = megabytes }, then: { try $0.prune() })
            historyRevision += 1
        } catch {
            settingsError = sentence(for: error, retrying: false, inSettings: true)
        }
    }

    func clearHistory(keepMemory: Bool) async -> String? {
        guard let core else { return nil }
        do {
            try await core.write { try $0.clearHistory(keepMemory: keepMemory) }
            desktop.forget()
            historyRevision += 1
            await refreshAll()
            desktopChanged()
            return nil
        } catch {
            return sentence(for: error)
        }
    }

    // MARK: Providers

    /// The provider's models. Settings' Test uses it too (the engine's `test_provider` is this same call), so the
    /// result can say how many models answered.
    func listModels(_ selection: ProviderSelection, job: ProviderJob) async throws -> [ModelInfo] {
        guard let core else { return [] }
        let models = try await core.engine.listModels(selection: selection, job: job)
        modelNames.merge(ModelMenu.names(models)) { _, new in new }
        return models
    }

    /// How long each model takes here, from the engine's recorded timings.
    struct Estimates: Equatable, Sendable {
        /// By model id ("" = the default); a model with no timings yet is left out.
        var seconds: [String: UInt32] = [:]
        /// Painting: the writing provider's estimate is included (a wallpaper is both); false while the writer hasn't
        /// been timed, so the line says "to paint" rather than "per wallpaper".
        var includesWriting = true
    }

    /// How long each model takes here (`estimate`, at the size AutoPaper would ask for now), by model id. For
    /// painting, a wallpaper is the writing and the painting together, so the writing provider's estimate is added
    /// when known — and when it isn't, that's said (`Estimates.includesWriting`), never left out silently.
    func estimates(_ selection: ProviderSelection, job: ProviderJob, models ids: [String]) async -> Estimates {
        guard let core else { return Estimates() }
        let writer = settings?.textProvider
        return (try? await core.call { engine in
            let writing = job == .images ? writer.flatMap { try? engine.estimate(selection: $0, job: .concepts, width: 0, height: 0) } : nil
            var result = Estimates(includesWriting: job == .concepts || writing != nil)
            for id in Set(ids + [""]) {
                var choice = selection
                choice.model = id
                if let seconds = try engine.estimate(selection: choice, job: job, width: 0, height: 0) {
                    result.seconds[id] = seconds + (writing ?? 0)
                }
            }
            return result
        }) ?? Estimates()
    }

    // MARK: First run

    enum Start: String, CaseIterable, Identifiable {
        case openAI, gemini, local, demo
        var id: Self { self }

        var title: String {
            switch self {
            case .openAI: "OpenAI"
            case .gemini: "Google Gemini"
            case .local: "Local"
            case .demo: "Try it without AI"
            }
        }

        var detail: String {
            switch self {
            case .openAI: "Writes ideas and paints with your OpenAI API key."
            case .gemini: "Writes ideas and paints with your Gemini API key."
            case .local: "Ollama writes ideas and ComfyUI paints, on this Mac. Free."
            case .demo: "Gradients instead of paintings, for trying AutoPaper. Free."
            }
        }

        var text: ProviderKind {
            switch self {
            case .openAI: .openAi
            case .gemini: .google
            case .local: .ollama
            case .demo: .demo
            }
        }

        var image: ProviderKind {
            switch self {
            case .openAI: .openAi
            case .gemini: .google
            case .local: .comfyUi
            case .demo: .demo
            }
        }

        /// The Keychain account for this choice's key, if it needs one.
        var keyAccount: String? { text.keyAccount }
    }

    /// A keyword from the welcome that the engine didn't take, and why.
    struct KeywordRefusal: Equatable, Sendable {
        let keyword: String
        let reason: String
    }

    /// What "Make My First Wallpaper" did.
    enum WelcomeOutcome: Equatable {
        /// Saved; the first wallpaper is being made.
        case started
        /// Some keywords weren't taken (the others are saved), with why; nothing else was done.
        case refused([KeywordRefusal])
        /// The providers couldn't be saved; the sentence to show.
        case failed(String)
    }

    /// "Make My First Wallpaper": saves the keywords (to the current mood) and providers, then makes one. When the
    /// engine refuses a keyword (too long, no letters), the others are saved and nothing else happens: the refusals
    /// come back for the welcome to show, so no keyword is dropped without a word.
    func finishWelcome(keywords newKeywords: [String], start: Start) async -> WelcomeOutcome {
        var refusals: [KeywordRefusal] = []
        for text in newKeywords {
            if case .refused(let reason) = await addKeyword(text, announcing: false) {
                refusals.append(KeywordRefusal(keyword: text, reason: reason))
            }
        }
        guard refusals.isEmpty else { return .refused(refusals) }
        // Awaited, so the first wallpaper is made with the providers chosen here, not the Demo still saved.
        do {
            try await saveSettings { settings in
                settings.textProvider = ProviderSelection(kind: start.text, model: "", baseUrl: nil)
                settings.imageProvider = ProviderSelection(kind: start.image, model: "", baseUrl: nil)
            }
        } catch {
            return .failed(sentence(for: error, retrying: false) ?? Sentences.internalProblem)
        }
        welcomeDone = true
        scheduleArmed = true
        newWallpaperNow()
        return .started
    }

    func skipWelcome() {
        welcomeDone = true
    }

    // MARK: Windows

    /// Opens the main window in front: the person asked for it (a menu, the Dock, a notification).
    func showMainWindow() {
        NSApp.activate()
        openWindowAction?(id: WindowID.main)
    }

    // MARK: Refreshing

    struct Snapshot: Sendable {
        let settings: EngineSettings
        let current: Generation?
        let description: String
        let moods: [Mood]
        let narrow: Bool
        let spend: SpendSummary
        let nextDue: Int64?
        let nextStart: Int64?
        let taste: TasteSummary
        let memory: MemoryStatus
    }

    func refreshAll() async {
        guard let core else { return }
        do {
            let snapshot = try await core.call { engine in
                let current = try engine.current()
                return Snapshot(
                    settings: try engine.settings(),
                    current: current,
                    description: try current.map { try engine.describe(id: $0.id) } ?? "",
                    moods: try engine.moods(),
                    narrow: try engine.keywordsAreNarrow(),
                    spend: try engine.spendSummary(),
                    nextDue: try engine.nextDue(),
                    nextStart: try engine.nextStart(),
                    taste: try engine.tasteSummary(),
                    memory: engine.memoryStatus()
                )
            }
            settings = snapshot.settings
            current = snapshot.current
            currentDescription = snapshot.description
            moods = snapshot.moods
            narrow = snapshot.narrow
            spend = snapshot.spend
            nextDue = snapshot.nextDue.map(Self.date)
            nextStart = snapshot.nextStart.map(Self.date)
            taste = snapshot.taste
            memory = snapshot.memory
            moodsChanged()
        } catch {
            report(error)
        }
        reschedule()
    }

    /// Spend, the next due and start times and the narrow-keywords flag: after every generation and settings change.
    func refreshStatus() async {
        guard let core else { return }
        if let (summary, due, start, isNarrow) = try? await core.call({ engine in
            (try engine.spendSummary(), try engine.nextDue(), try engine.nextStart(), try engine.keywordsAreNarrow())
        }) {
            spend = summary
            nextDue = due.map(Self.date)
            nextStart = start.map(Self.date)
            narrow = isNarrow
        }
        reschedule()
    }

    private static func date(_ unix: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(unix))
    }

    /// Arms the timer for the engine's next start (the due time less how long a wallpaper takes here, so it's ready
    /// on time), never earlier than the hold after launch or a minute after the last scheduled attempt.
    private func reschedule() {
        scheduler?.schedule(at: ScheduleRules.timerDate(
            armed: scheduleArmed && phase == .ready, nextStart: nextStart, nextDue: nextDue, holdUntil: holdUntil,
            lastAttempt: lastScheduledAttempt, minimumGap: Self.minimumGap
        ))
    }

    /// No automatic attempt within this long after launch (the network may not be up yet at login).
    static let launchGrace: TimeInterval = 15
    /// At least this long between two scheduled attempts, whatever the engine's due time says.
    static let minimumGap: TimeInterval = 60
    /// A progress stage is announced once it has lasted this long.
    static let stageAnnouncementDelay: Duration = .seconds(1)

    // MARK: Displays

    private func displaysChanged() {
        displayChange?.cancel()
        displayChange = Task {
            // Displays settle over a second or two after a change (arrangement, sleep, a dock connecting).
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled, let core else { return }
            await sendDisplayHint()
            // Every display gets a render at its new size (cached per size by the engine), and a new display its
            // own overlay window. A display showing the person's own picture keeps it, and an uncovered desktop
            // stays uncovered.
            if let current, wallpaperMode == .replace || !overlayUncovered {
                do {
                    try await desktop.show(current, using: core, onlyReplacingAutoPaper: true)
                } catch {
                    log.error("Re-rendering for new displays failed: \(String(describing: error), privacy: .public)")
                }
            }
            desktopChanged()
        }
    }

    private func sendDisplayHint() async {
        guard let core, let (width, height) = DesktopService.largestDisplay() else { return }
        do {
            try await core.call { try $0.setDisplayHint(width: width, height: height) }
        } catch {
            log.error("Display hint refused: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: Accessibility

    /// Says `text` politely to VoiceOver (progress stages, errors, a new wallpaper, a change made). Only while
    /// AutoPaper is the active app, so scheduled wallpapers don't talk over whatever the person is doing
    /// (notifications cover them, if on); `evenInBackground` for the outcome of something the person asked for
    /// from the menu bar, where AutoPaper doesn't become active.
    func announce(_ text: String, evenInBackground: Bool = false) {
        guard NSApp.isActive || evenInBackground else { return }
        NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested, userInfo: [
            .announcement: text,
            .priority: NSAccessibilityPriorityLevel.medium.rawValue,
        ])
    }
}


/// A removed keyword, for Undo and Redo: putting it back gives it a new id, which a later Redo removes.
@MainActor
private final class RemovedKeyword {
    var id: String
    let text: String
    let weight: KeywordWeight
    let moodID: Mood.ID
    let position: Int
    /// Putting it back, while that's being saved.
    var restoring: Task<Void, Never>?

    init(id: String, text: String, weight: KeywordWeight, moodID: Mood.ID, position: Int) {
        self.id = id
        self.text = text
        self.weight = weight
        self.moodID = moodID
        self.position = position
    }
}

extension Generation: @retroactive Identifiable {}
extension Keyword: @retroactive Identifiable {}
extension Mood: @retroactive Identifiable {}
