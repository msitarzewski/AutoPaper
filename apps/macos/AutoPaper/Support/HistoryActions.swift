import AutopaperCore

/// What a history item can do; shared by its context menu, its actions button, the menu Return opens and its
/// accessibility actions, so they always offer the same things. (Kept apart from `HistoryView` so the unit tests,
/// which compile only this folder, can check what's offered when.)
struct HistoryActions {
    let showOnDesktop: @MainActor (Generation) -> Void
    let rate: @MainActor (Generation, Rating) -> Void
    let makeEcho: @MainActor (Generation) -> Void
    let showLineage: @MainActor (Generation) -> Void
    let showInFinder: @MainActor (Generation) -> Void
    let delete: @MainActor (Generation) -> Void
    /// A wallpaper can be made or put on the desktop now (the engine is open and nothing is being made).
    let canMake: Bool

    /// One action as menus and VoiceOver show it.
    struct Item: Identifiable {
        /// The menu item's title ("Delete…").
        let title: String
        /// The accessibility action's name ("Remove Like", "Delete").
        let spokenTitle: String
        /// A checkmark item's state (Like, Dislike); nil for plain items.
        let isOn: Bool?
        let isEnabled: Bool
        var isDestructive = false
        let perform: @MainActor () -> Void
        var id: String { title }
    }

    /// The actions, in groups (a separator between groups). "Show Original and Echoes" only when there's an
    /// original or echoes to show (`has_echoes`).
    @MainActor
    func groups(for generation: Generation, hasEchoes: Bool) -> [[Item]] {
        let liked = generation.rating == .liked, disliked = generation.rating == .disliked
        var more = [
            Item(title: "Make an Echo", spokenTitle: "Make an Echo", isOn: nil, isEnabled: canMake) { makeEcho(generation) },
        ]
        if hasEchoes {
            more.append(Item(title: "Show Original and Echoes", spokenTitle: "Show Original and Echoes", isOn: nil, isEnabled: true) {
                showLineage(generation)
            })
        }
        more.append(Item(title: "Show in Finder", spokenTitle: "Show in Finder", isOn: nil,
                         isEnabled: generation.imagePath != nil || generation.thumbPath != nil) { showInFinder(generation) })
        return [
            [Item(title: "Show on Desktop", spokenTitle: "Show on Desktop", isOn: nil,
                  isEnabled: generation.imagePath != nil && canMake) { showOnDesktop(generation) }],
            [
                Item(title: "Like", spokenTitle: liked ? "Remove Like" : "Like", isOn: liked, isEnabled: true) { rate(generation, .liked) },
                Item(title: "Dislike", spokenTitle: disliked ? "Remove Dislike" : "Dislike", isOn: disliked, isEnabled: true) {
                    rate(generation, .disliked)
                },
            ],
            more,
            [Item(title: "Delete…", spokenTitle: "Delete", isOn: nil, isEnabled: true, isDestructive: true) { delete(generation) }],
        ]
    }
}
