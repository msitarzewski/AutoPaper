import AppIntents
import AutopaperCore

// Siri, Shortcuts and Spotlight (App Intents, run inside the app): New Wallpaper, Like This Wallpaper, Dislike This
// Wallpaper, What's on My Desktop (the engine's description, with the rating in AutoPaper's words) and Switch Mood.

struct NewWallpaperIntent: AppIntent {
    static let title: LocalizedStringResource = "New Wallpaper"
    static let description = IntentDescription("Makes a new wallpaper from your current mood now and puts it on the desktop.")

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        guard await model.ready() else { return .result(dialog: "AutoPaper couldn't open its library.") }
        guard !model.isWorking else { return .result(dialog: "AutoPaper is already making a new wallpaper.") }
        model.newWallpaperNow()
        return .result(dialog: "AutoPaper is making a new wallpaper.")
    }
}

struct LikeWallpaperIntent: AppIntent {
    static let title: LocalizedStringResource = "Like This Wallpaper"
    static let description = IntentDescription("Tells AutoPaper you like the wallpaper on the desktop, so it makes more like it.")

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        guard await model.ready(), let current = model.current else {
            return .result(dialog: "There's no AutoPaper wallpaper on the desktop yet.")
        }
        model.rate(id: current.id, .liked, toggle: false)
        return .result(dialog: IntentDialog(stringLiteral: "Liked “\(current.concept.title)”."))
    }
}

struct DislikeWallpaperIntent: AppIntent {
    static let title: LocalizedStringResource = "Dislike This Wallpaper"
    static let description = IntentDescription("Tells AutoPaper you don't like the wallpaper on the desktop. If Replace Wallpapers I Dislike is on, it makes a new one.")

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        guard await model.ready(), let current = model.current else {
            return .result(dialog: "There's no AutoPaper wallpaper on the desktop yet.")
        }
        model.rate(id: current.id, .disliked, toggle: false)
        let replacing = model.settings?.replaceDisliked == true
        return .result(dialog: IntentDialog(stringLiteral: replacing
            ? "Disliked “\(current.concept.title)”. AutoPaper will make a new one if the budget allows."
            : "Disliked “\(current.concept.title)”."))
    }
}

struct DesktopDescriptionIntent: AppIntent {
    static let title: LocalizedStringResource = "What's on My Desktop"
    static let description = IntentDescription("Says what AutoPaper has put on the desktop, in words.")

    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let model = AppModel.shared
        let ready = await model.ready()
        // A picture the person chose is said as theirs, not described as AutoPaper's wallpaper.
        let answer = DesktopAnswer.text(
            description: ready && model.current != nil ? model.currentSpokenDescription : nil,
            title: model.current?.concept.title,
            ownPictureShowing: ready && model.ownPictureShowing
        )
        return .result(value: answer, dialog: IntentDialog(stringLiteral: answer))
    }
}

/// A mood, for Switch Mood's parameter. Moods are the person's own (made, renamed and deleted in the app), so this is
/// an `AppEntity` with a query listing them — Shortcuts shows it as a menu of moods, and Siri matches their names —
/// rather than an `AppEnum`, whose cases are fixed when the app is built.
struct MoodEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Mood"
    static let defaultQuery = MoodQuery()

    let id: String
    let name: String

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)")
    }
}

struct MoodQuery: EnumerableEntityQuery, EntityStringQuery {
    @MainActor
    func allEntities() async throws -> [MoodEntity] {
        guard await AppModel.shared.ready() else { return [] }
        return AppModel.shared.moods.map { MoodEntity(id: $0.id, name: $0.name) }
    }

    @MainActor
    func entities(for identifiers: [MoodEntity.ID]) async throws -> [MoodEntity] {
        try await allEntities().filter { identifiers.contains($0.id) }
    }

    @MainActor
    func entities(matching string: String) async throws -> [MoodEntity] {
        let wanted = KeywordText.normalised(string).lowercased()
        return try await allEntities().filter { $0.name.lowercased().contains(wanted) }
    }

    @MainActor
    func suggestedEntities() async throws -> [MoodEntity] {
        try await allEntities()
    }
}

struct SwitchMoodIntent: AppIntent {
    static let title: LocalizedStringResource = "Switch Mood"
    static let description = IntentDescription("Makes a mood current. The next wallpaper uses its keywords and Surprise; nothing is made right away.")

    @Parameter(title: "Mood")
    var mood: MoodEntity

    static var parameterSummary: some ParameterSummary {
        Summary("Switch AutoPaper to \(\.$mood)")
    }

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        guard await model.ready() else { return .result(dialog: "AutoPaper couldn't open its library.") }
        guard let chosen = model.mood(mood.id) else {
            return .result(dialog: IntentDialog(stringLiteral: "AutoPaper has no mood called “\(mood.name)” any more."))
        }
        if chosen.active {
            return .result(dialog: IntentDialog(stringLiteral: "\(chosen.name) is already the current mood."))
        }
        // Answered once the switch is saved, so a failure is said rather than reported as done.
        if let problem = await model.switchMood(chosen.id) {
            return .result(dialog: IntentDialog(stringLiteral: "AutoPaper couldn't switch to \(chosen.name). \(problem)"))
        }
        return .result(dialog: IntentDialog(stringLiteral: "Switched to \(chosen.name). The next wallpaper uses it."))
    }
}

/// Phrases Siri knows without any setup. Apple requires the app's name in each one. Switch Mood's phrases name the
/// person's moods; the app updates them when moods change (`updateAppShortcutParameters`).
struct AutoPaperShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: NewWallpaperIntent(),
            phrases: ["New wallpaper in \(.applicationName)", "Make a new \(.applicationName) wallpaper"],
            shortTitle: "New Wallpaper",
            systemImageName: "sparkles"
        )
        AppShortcut(
            intent: LikeWallpaperIntent(),
            phrases: ["Like this \(.applicationName) wallpaper", "I like this \(.applicationName) wallpaper"],
            shortTitle: "Like This Wallpaper",
            systemImageName: "hand.thumbsup"
        )
        AppShortcut(
            intent: DislikeWallpaperIntent(),
            phrases: ["Dislike this \(.applicationName) wallpaper", "I don't like this \(.applicationName) wallpaper"],
            shortTitle: "Dislike This Wallpaper",
            systemImageName: "hand.thumbsdown"
        )
        AppShortcut(
            intent: DesktopDescriptionIntent(),
            // Commands, not questions: Siri answers "what's on my desktop" itself (AudioPaper's finding).
            phrases: ["Describe my \(.applicationName) wallpaper", "Read the \(.applicationName) wallpaper"],
            shortTitle: "What's on My Desktop",
            systemImageName: "text.below.photo"
        )
        AppShortcut(
            intent: SwitchMoodIntent(),
            phrases: [
                "Switch \(.applicationName) to \(\.$mood)",
                "Use the \(\.$mood) mood in \(.applicationName)",
                "Switch \(.applicationName) mood",
            ],
            shortTitle: "Switch Mood",
            systemImageName: "rectangle.stack"
        )
    }
}
