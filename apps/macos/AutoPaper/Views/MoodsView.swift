import AutopaperCore
import Charts
import SwiftUI

// Moods (the user's layout, 2026-10-06): the main window's middle column lists every mood under its own title and +,
// and the right column is the selected mood's detail — its editable name, its keywords, its Surprise, what it has
// made, Delete Mood…. The detail toolbar holds New Wallpaper Now / Stop.
// A mood is a name with its own keywords and Surprise; exactly one is current, and switching never makes a
// wallpaper by itself.

/// The middle column: every mood in the person's order (drag to reorder, or Edit ▸ Move Mood Up / Down). Each row
/// shows only the mood's name. Double-click or
/// Return uses the selected mood; + adds one (⌘N); right-click offers Use, Duplicate, Rename…, Delete…; the Delete
/// key asks to delete the selected mood. The + and its title, "Moods" (the window's title too), lead the list's part
/// of the toolbar.
struct MoodList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        // Nothing selected (a click in the list's empty space, or the sidebar's Moods row) shows the summary of
        // every mood.
        List(selection: $model.moodSelection) {
            ForEach(model.moods) { mood in
                MoodRow(mood: mood, index: index(of: mood), count: model.moods.count)
                    .tag(mood.id)
            }
            .onMove { offsets, destination in
                guard let from = offsets.first else { return }
                model.moveMood(model.moods[from].id, to: Reorder.position(from: from, droppedAt: destination))
            }
        }
        .contextMenu(forSelectionType: Mood.ID.self) { ids in
            if let id = ids.first, let mood = model.mood(id) {
                MoodMenuItems(mood: mood)
            }
        } primaryAction: { ids in
            // Double-click or Return: use it.
            if let id = ids.first { model.useMood(id) }
        }
        .onDeleteCommand {
            if let id = model.moodSelection, model.moods.count > 1 { model.moodToDelete = id }
        }
        .focusedValue(\.reorderable, reorderable)
        .accessibilityLabel("Moods")
        .navigationTitle("Moods")
        .toolbar {
            // Leading the list column's part of the toolbar, before its title (where Mail and Notes put their own
            // "new" buttons).
            ToolbarItem(placement: .navigation) {
                Button {
                    model.newMood()
                } label: {
                    Label("New Mood", systemImage: "plus")
                }
                .help("New Mood (⌘N)")
                .disabled(model.phase != .ready)
            }
        }
    }

    private func index(of mood: Mood) -> Int {
        model.moods.firstIndex { $0.id == mood.id } ?? 0
    }

    /// The selected mood, for Edit ▸ Move Mood Up / Down (⌥⌘↑ / ⌥⌘↓).
    private var reorderable: ReorderableRow? {
        guard let id = model.moodSelection, let index = model.moods.firstIndex(where: { $0.id == id }) else { return nil }
        return ReorderableRow(
            noun: "Mood",
            canMoveUp: index > 0,
            canMoveDown: index < model.moods.count - 1,
            moveUp: { model.moveMood(id, to: index - 1) },
            moveDown: { model.moveMood(id, to: index + 1) }
        )
    }
}

/// One mood in the list: its name, with the same reorder actions as the Edit menu.
private struct MoodRow: View {
    @Environment(AppModel.self) private var model
    let mood: Mood
    let index: Int
    let count: Int

    var body: some View {
        Text(mood.name)
            .lineLimit(1)
        // Only the moves that apply (none up from the first row, none down from the last), as the menus disable them.
        .accessibilityActions {
            if index > 0 { Button("Move Up") { model.moveMood(mood.id, to: index - 1) } }
            if index < count - 1 { Button("Move Down") { model.moveMood(mood.id, to: index + 1) } }
        }
    }
}


/// The right column in Moods: the selected mood, or (none selected) the summary of every mood. What each mood has made
/// (`mood_stats`) is read here, once for both.
struct MoodDetailColumn: View {
    @Environment(AppModel.self) private var model
    @State private var stats: [Mood.ID: MoodStats] = [:]

    var body: some View {
        Group {
            if let mood = model.mood(model.moodSelection) {
                MoodDetail(mood: mood, stats: stats[mood.id])
                    // A fresh detail per mood: its title, Surprise and recent wallpapers belong to that mood.
                    .id(mood.id)
            } else {
                MoodsSummary(stats: stats)
            }
        }
        .task(id: StatsKey(revision: model.historyRevision, moods: model.moods.count, ready: model.phase == .ready)) {
            let list = await model.moodStats()
            stats = Dictionary(list.map { ($0.moodId, $0) }, uniquingKeysWith: { first, _ in first })
        }
    }

    private struct StatsKey: Equatable {
        let revision: Int
        let moods: Int
        let ready: Bool
    }
}

// MARK: - One mood

/// One mood: an editable name in the detail header (Rename… and File ▸ Rename Mood… focus it), its Surprise and
/// what it has made; one native grouped form for keywords, Surprise, recent wallpapers and Delete Mood….
private struct MoodDetail: View {
    @Environment(AppModel.self) private var model
    let mood: Mood
    let stats: MoodStats?
    /// Why the last name typed in the title wasn't saved; shown under the header until the name is edited again.
    @State private var renameProblem: String?

    var body: some View {
        GeometryReader { proxy in
            VStack(spacing: 0) {
                MoodHeader(mood: mood, stats: stats, problem: $renameProblem)
                Form {
                    MoodKeywords(mood: mood, viewportLimit: max(160, proxy.size.height / 2))
                    Section("Surprise") {
                        SurpriseControl(mood: mood)
                    }
                    Section("Made in this mood") {
                        RecentWallpapers(mood: mood)
                    }
                    Section {
                        LabeledContent {
                            Button("Delete Mood…", role: .destructive) { model.moodToDelete = mood.id }
                                .disabled(model.moods.count <= 1)
                        } label: {
                            Text(model.moods.count <= 1
                                 ? "There's always at least one mood, so this one stays."
                                 : "Its wallpapers stay in History.")
                        }
                    }
                }
                .formStyle(.grouped)
                .scrollBounceBehavior(.basedOnSize)
            }
        }
        // Now's reload/stop remains at the trailing end. The mood's editable name lives in its detail header.
        .toolbar {
            ToolbarSpacer(.flexible, placement: .primaryAction)
            ToolbarItem(placement: .primaryAction) {
                MakeOrStopButton(mood: mood)
            }
        }
    }
}

/// The mood's name at the top of the detail, looking like a title and renamed in place:
/// click it (or Rename…, File ▸ Rename Mood…, New Mood) and type; Return or leaving it saves, Esc puts the saved name
/// back. A name the engine refuses (another mood's, too long) is put back, and why is said once, under the header, and
/// announced. Reuses the AppKit title field's Return, Esc and requested-focus handling.
private struct MoodTitleField: View {
    @Environment(AppModel.self) private var model
    let mood: Mood
    @Binding var problem: String?
    @State private var name = ""
    @State private var saving: String?
    @State private var width: CGFloat = 160
    /// Bumped to put the keyboard in the field with the name selected (New Mood, Rename…).
    @State private var focusRequest = 0

    private static let font = Font.title3.weight(.semibold)

    var body: some View {
        TitleTextField(
            text: $name,
            font: .systemFont(ofSize: NSFont.preferredFont(forTextStyle: .title3).pointSize, weight: .semibold),
            label: "Mood name",
            help: problem ?? "Click to rename this mood. Return saves, Esc puts the name back.",
            focusRequest: focusRequest,
            onEdit: { problem = nil },
            onCommit: commit,
            onCancel: { name = mood.name; problem = nil }
        )
        .frame(width: width)
        // As wide as the name (within reason), so it sits like a title.
        .background {
            Text(name.isEmpty ? "Mood name" : name)
                .font(Self.font)
                .fixedSize()
                .hidden()
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { measured in
                    width = min(360, max(80, measured + 10))
                }
        }
        .onAppear {
            name = mood.name
            focusIfAsked()
        }
        .onChange(of: mood.name) { _, newValue in if saving == nil { name = newValue } }
        .onChange(of: model.moodToName) { focusIfAsked() }
    }

    /// New Mood and Rename… ask for the name: focused, with the name selected to type over.
    private func focusIfAsked() {
        guard model.moodToName == mood.id else { return }
        model.moodToName = nil
        focusRequest += 1
    }

    private func commit(_ typed: String) {
        let normalised = KeywordText.normalised(typed)
        guard !normalised.isEmpty, normalised != mood.name else {
            name = mood.name
            return
        }
        guard saving != normalised else { return }
        let original = mood.name, id = mood.id
        saving = normalised
        Task {
            defer { saving = nil }
            switch await model.renameMood(id, to: normalised) {
            case .done(let renamed):
                problem = nil
                name = renamed.name
            case .refused(let reason):
                // What's shown is what's saved (and what VoiceOver says); why is said under the header, and stays
                // there until the person edits the name again.
                name = original
                let said = "“\(normalised)” wasn't saved. \(reason)"
                problem = said
                model.announce(said)
            }
        }
    }
}

/// A borderless AppKit text field that reads as a window title until it's edited: Return and leaving it end the
/// edit (`onCommit`), Esc puts the saved text back and keeps the field (`onCancel`), and a new `focusRequest` puts
/// the keyboard in it with the text selected.
private struct TitleTextField: NSViewRepresentable {
    @Binding var text: String
    let font: NSFont
    let label: String
    let help: String
    let focusRequest: Int
    let onEdit: () -> Void
    let onCommit: (String) -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> RequestedTitleField {
        let field = RequestedTitleField(string: text)
        field.isBordered = false
        field.drawsBackground = false
        field.isEditable = true
        field.isSelectable = true
        field.font = font
        // The label colour, like the window titles it stands in for (the field's default control colour reads dimmer).
        field.textColor = .labelColor
        field.lineBreakMode = .byTruncatingTail
        field.usesSingleLineMode = true
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.placeholderString = label
        field.delegate = context.coordinator
        return field
    }

    func updateNSView(_ field: RequestedTitleField, context: Context) {
        context.coordinator.parent = self
        if field.currentEditor() == nil || context.coordinator.putBack, field.stringValue != text {
            field.stringValue = text
            context.coordinator.putBack = false
        }
        field.setAccessibilityLabel(label)
        field.setAccessibilityHelp(help)
        field.toolTip = help
        if focusRequest != context.coordinator.focused {
            field.requestFocus { [weak coordinator = context.coordinator] in
                coordinator?.focused = focusRequest
            }
        }
    }

    /// A requested edit stays pending until the native title field belongs to its window.
    final class RequestedTitleField: NSTextField {
        private var focusCompletion: (() -> Void)?

        func requestFocus(_ completion: @escaping () -> Void) {
            focusCompletion = completion
            DispatchQueue.main.async { [weak self] in self?.focusWhenAttached() }
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if focusCompletion != nil {
                DispatchQueue.main.async { [weak self] in self?.focusWhenAttached() }
            }
        }

        private func focusWhenAttached() {
            guard let completion = focusCompletion, let window,
                  window.makeFirstResponder(self) else { return }
            currentEditor()?.selectAll(nil)
            focusCompletion = nil
            completion()
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: TitleTextField
        var focused = 0
        /// Esc put the saved text back while editing: shown even though the field is being edited.
        var putBack = false

        init(_ parent: TitleTextField) { self.parent = parent }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            parent.text = field.stringValue
            parent.onEdit()
        }

        func controlTextDidEndEditing(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            parent.onCommit(field.stringValue)
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            guard selector == #selector(NSResponder.cancelOperation(_:)) else { return false }
            // Esc: the saved name comes back and nothing is saved; the keyboard stays in the field.
            putBack = true
            parent.onCancel()
            DispatchQueue.main.async { textView.selectAll(nil) }
            return true
        }
    }
}

/// The top of a mood: its editable name, its Surprise band, why a new name wasn't saved (if so),
/// then what it has made: wallpapers, liked, echoes and when the last one was made.
private struct MoodHeader: View {
    let mood: Mood
    let stats: MoodStats?
    @Binding var problem: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 5) {
                MoodTitleField(mood: mood, problem: $problem)
                Text(MoodText.surpriseLine(mood.surprise))
                    .font(.callout)
                    .quietText()
            }
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 10) {
                StatTile(title: "Wallpapers", symbol: "photo.on.rectangle", value: "\(stats?.wallpapers ?? 0)", compact: true)
                StatTile(title: "Liked", symbol: "hand.thumbsup", value: "\(stats?.liked ?? 0)", compact: true)
                StatTile(title: "Echoes", symbol: "arrow.trianglehead.2.clockwise.rotate.90", value: "\(stats?.echoes ?? 0)", compact: true)
                StatTile(title: "Last made", symbol: "clock", value: lastMade, compact: true)
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Made in \(mood.name)")
        }
        .padding(.horizontal, 20)
        .padding(.top, 14)
        .padding(.bottom, 14)
    }

    private var lastMade: String {
        guard let made = stats?.lastMadeAt else { return "Not yet" }
        let words = MoodText.relative(made)
        return words.prefix(1).uppercased() + words.dropFirst()
    }
}

/// A number with what it counts, on a softly filled rounded tile: a small symbol and label above, the number below.
/// Spoken as one line ("12 liked").
struct StatTile: View {
    let title: String
    let symbol: String
    let value: String
    /// What VoiceOver says; "<value> <title in lower case>" when not given.
    var spoken: String?
    /// Smaller, for a mood's own header.
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 3 : 6) {
            Label(title, systemImage: symbol)
                .font(compact ? .caption : .callout)
                .labelStyle(.titleAndIcon)
                .lineLimit(1)
                .quietText()
            Text(value)
                .font((compact ? Font.title3 : Font.title).weight(.semibold))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
        }
        .padding(.horizontal, compact ? 10 : 14)
        .padding(.vertical, compact ? 8 : 12)
        // Wide enough for its label at full size, so a row that can't give each tile this much wraps to two by two.
        .frame(minWidth: compact ? nil : 136, maxWidth: .infinity, alignment: .leading)
        .background(.fill.quaternary, in: RoundedRectangle(cornerRadius: compact ? 9 : 12, style: .continuous))
        .accessibilityRepresentation {
            Text(spoken ?? "\(value) \(title.lowercased())")
        }
    }
}

/// The newest wallpapers made in a mood: each shows itself on the desktop when clicked and has History's menu; and a
/// way to see the rest in History.
private struct RecentWallpapers: View {
    @Environment(AppModel.self) private var model
    let mood: Mood
    @State private var items: [Generation] = []
    @State private var info: [String: AppModel.TileInfo] = [:]
    @State private var loaded = false

    private static let count: UInt32 = 6

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if items.isEmpty {
                Text(loaded ? "Nothing yet. Wallpapers made while this mood is current appear here." : "Loading…")
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                ScrollView(.horizontal) {
                    HStack(spacing: 10) {
                        ForEach(items) { generation in
                            WallpaperThumbnail(generation: generation, info: info[generation.id])
                        }
                    }
                    .padding(.vertical, 4)
                    .padding(.horizontal, 2)
                }
                .scrollIndicators(.automatic)
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Wallpapers made in \(mood.name)")
            }
            HStack {
                Spacer()
                Button("Show All in History") {
                    model.historyMood = mood.id
                    model.section = .history
                }
                .disabled(items.isEmpty)
            }
        }
        .task(id: Key(mood: mood.id, revision: model.historyRevision)) {
            let recent = await model.recentWallpapers(of: mood.id, limit: Self.count)
            info = await model.tileInfo(for: recent)
            items = recent
            loaded = true
        }
    }

    private struct Key: Equatable {
        let mood: Mood.ID
        let revision: Int
    }
}

/// A wallpaper's thumbnail outside History (a mood's wallpapers, the summary's cards): clicking it (or Return or
/// Space) shows it on the desktop, and right-click (or VoiceOver's actions) offers exactly History's menu. Spoken as
/// the engine's description and the rating, as in History.
struct WallpaperThumbnail: View {
    @Environment(AppModel.self) private var model
    let generation: Generation
    let info: AppModel.TileInfo?
    var size = CGSize(width: 112, height: 70)

    var body: some View {
        let groups = model.wallpaperActions.groups(for: generation, hasEchoes: info?.hasEchoes ?? false)
        let show = groups.first?.first
        Button {
            if let show, show.isEnabled { show.perform() }
        } label: {
            WallpaperImage(generation, contentMode: .fill, maxPixelSize: 320, preferThumbnail: true)
                .frame(width: size.width, height: size.height)
                .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(.separator)
                }
                .contentShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .onKeyPress(.return) {
            if let show, show.isEnabled { show.perform() }
            return .handled
        }
        .contextMenu { HistoryMenuItems(groups: groups) }
        .help("Show on Desktop\n\(Provenance.line(generation, listed: model.modelNames).painted)")
        .accessibilityLabel(info?.spoken ?? Spoken.wallpaper(description: generation.concept.title, rating: generation.rating))
        .accessibilityAddTraits(.isImage)
        .accessibilityHint("Shows it on the desktop.")
        .accessibilityActions {
            // Only what can be done now, as in the menus.
            ForEach(groups.joined().filter(\.isEnabled)) { item in
                Button(item.spokenTitle) { item.perform() }
            }
        }
    }
}

// MARK: - Every mood

/// Moods with nothing selected (user, 2026-10-06, after their own app's library page): "Your Moods", the totals as
/// tiles (moods, wallpapers, liked, echoes), the keywords' weights in one line, a chart of the last 30 days' wallpapers
/// stacked by mood, and every mood from most to least used, each a card that opens the mood (with its keywords,
/// Surprise, what it made and its latest wallpapers).
private struct MoodsSummary: View {
    @Environment(AppModel.self) private var model
    let stats: [Mood.ID: MoodStats]
    @State private var activity: [DayCount] = []
    @State private var info: [String: AppModel.TileInfo] = [:]

    var body: some View {
        let all = model.moods.compactMap { stats[$0.id] }
        let totals = MoodText.totals(moods: model.moods.count, stats: all)
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Text("Your Moods")
                    .font(.largeTitle.weight(.bold))
                    .accessibilityAddTraits(.isHeader)
                VStack(alignment: .leading, spacing: 10) {
                    let tiles = [
                        StatTile(title: "Moods", symbol: "rectangle.stack", value: "\(totals[0].value)", spoken: totals[0].spoken),
                        StatTile(title: "Wallpapers", symbol: "photo.on.rectangle", value: "\(totals[1].value)", spoken: totals[1].spoken),
                        StatTile(title: "Liked", symbol: "hand.thumbsup", value: "\(totals[2].value)", spoken: totals[2].spoken),
                        StatTile(title: "Echoes", symbol: "arrow.trianglehead.2.clockwise.rotate.90",
                                 value: "\(all.reduce(0) { $0 + Int($1.echoes) })"),
                    ]
                    // A row of four, or two by two where the column is too narrow for their labels.
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 12) {
                            ForEach(tiles.indices, id: \.self) { tiles[$0] }
                        }
                        Grid(horizontalSpacing: 12, verticalSpacing: 12) {
                            GridRow { tiles[0]; tiles[1] }
                            GridRow { tiles[2]; tiles[3] }
                        }
                    }
                    Label(KeywordBreakdown.line(model.moods), systemImage: "tag")
                        .font(.callout)
                        .quietText()
                }
                MoodActivityChart(moods: model.moods, counts: activity)
                VStack(alignment: .leading, spacing: 10) {
                    Text("Most Used")
                        .font(.title3.weight(.semibold))
                        .accessibilityAddTraits(.isHeader)
                    ForEach(mostUsed) { mood in
                        MoodCard(mood: mood, stats: stats[mood.id], info: info, slot: MoodActivity.colorSlots(model.moods)[mood.id])
                    }
                }
            }
            .frame(maxWidth: 900, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .task(id: model.historyRevision) {
            activity = await model.activity()
        }
        .task(id: stats.values.flatMap { $0.latest.map(\.id) }.sorted()) {
            info = await model.tileInfo(for: stats.values.flatMap(\.latest))
        }
    }

    /// Most wallpapers first; equal ones in the person's order.
    private var mostUsed: [Mood] {
        model.moods.enumerated()
            .sorted { (-Int(stats[$0.element.id]?.wallpapers ?? 0), $0.offset) < (-Int(stats[$1.element.id]?.wallpapers ?? 0), $1.offset) }
            .map(\.element)
    }
}

/// One mood in the summary: a dot in its chart colour, its name (opens it), "Current mood" or Use, how many wallpapers
/// it made (on the right), its keywords by weight, its Surprise, liked and last made, and its latest wallpapers. A
/// click anywhere else on the card opens the mood too.
private struct MoodCard: View {
    @Environment(AppModel.self) private var model
    let mood: Mood
    let stats: MoodStats?
    let info: [String: AppModel.TileInfo]
    let slot: Int?

    var body: some View {
        let groups = MoodText.keywordGroups(mood.keywords)
        let made = stats?.wallpapers ?? 0
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Circle()
                    .fill(ChartPalette.color(slot: slot))
                    .frame(width: 9, height: 9)
                    .accessibilityHidden(true)
                Button {
                    model.moodSelection = mood.id
                } label: {
                    HStack(spacing: 4) {
                        Text(mood.name)
                            .font(.headline)
                        Image(systemName: "chevron.forward")
                            .font(.caption.weight(.semibold))
                            .quietText()
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(MoodText.spokenName(mood))
                .accessibilityHint("Shows its keywords and Surprise.")
                .help("Show \(mood.name)")
                if mood.active {
                    Label("Current mood", systemImage: "checkmark")
                        .font(.callout.weight(.medium))
                        .labelStyle(.titleAndIcon)
                        .fixedSize()
                        // Said with the name ("Rainy beach, current mood").
                        .accessibilityHidden(true)
                } else {
                    Button("Use") { model.useMood(mood.id) }
                        .controlSize(.small)
                        .fixedSize()
                        .accessibilityLabel("Use \(mood.name)")
                        .help("Make this the current mood. Nothing changes until the next wallpaper.")
                }
                Spacer(minLength: 8)
                VStack(alignment: .trailing, spacing: 0) {
                    Text("\(made)")
                        .font(.title2.weight(.semibold))
                    Text(made == 1 ? "wallpaper" : "wallpapers")
                        .font(.caption)
                        .quietText()
                }
                .accessibilityRepresentation { Text(made == 1 ? "1 wallpaper" : "\(made) wallpapers") }
            }
            VStack(alignment: .leading, spacing: 3) {
                if groups.isEmpty {
                    Text("No keywords yet")
                } else {
                    ForEach(groups, id: \.weight) { group in
                        Text(group.line)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .font(.callout)
            Text("\(MoodText.surpriseLine(mood.surprise)) · \(MoodText.madeLine(wallpapers: made, liked: stats?.liked ?? 0, lastMadeAt: stats?.lastMadeAt))")
                .font(.callout)
                .quietText()
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityRepresentation {
                    Text("\(MoodText.surpriseLine(mood.surprise)), \(MoodText.madeLine(wallpapers: made, liked: stats?.liked ?? 0, lastMadeAt: stats?.lastMadeAt, separator: ", "))")
                }
            if let latest = stats?.latest, !latest.isEmpty {
                HStack(spacing: 8) {
                    ForEach(latest) { generation in
                        WallpaperThumbnail(generation: generation, info: info[generation.id], size: CGSize(width: 96, height: 60))
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel(MoodText.spokenLatest(latest.map(\.concept.title)))
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.fill.quaternary, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .onTapGesture { model.moodSelection = mood.id }
        .contextMenu { MoodMenuItems(mood: mood) }
        .accessibilityElement(children: .contain)
    }
}

/// The chart's colours: the dataviz skill's validated categorical palette, its light and dark steps (checked with
/// its validator against these windows' surfaces: every adjacent pair ≥ 8.4 ΔE for colour-blind readers), in a fixed
/// order by mood (`MoodActivity.colorSlots`); "Other moods" is a neutral grey. Identity never rests on colour alone:
/// the legend names each colour, the hover readout and the table say every number, and VoiceOver hears each bar.
enum ChartPalette {
    static let light: [UInt32] = [0x2A78D6, 0xEB6834, 0x1BAF7A, 0xEDA100, 0xE87BA4, 0x008300, 0x4A3AA7, 0xE34948]
    static let dark: [UInt32] = [0x3987E5, 0xD95926, 0x199E70, 0xC98500, 0xD55181, 0x008300, 0x9085E9, 0xE66767]
    static let other: UInt32 = 0x8A8984

    static func color(slot: Int?) -> Color {
        guard let slot, light.indices.contains(slot) else { return Color(rgb: other) }
        return Color(light: light[slot], dark: dark[slot])
    }
}

extension Color {
    init(rgb: UInt32) {
        self.init(nsColor: NSColor(rgb: rgb))
    }

    /// A colour with its own step for Dark Mode.
    init(light: UInt32, dark: UInt32) {
        self.init(nsColor: NSColor(name: nil) { appearance in
            NSColor(rgb: appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark : light)
        })
    }
}

extension NSColor {
    convenience init(rgb: UInt32) {
        self.init(srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
                  blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }
}

/// "Wallpapers, last 30 days": a bar per day, stacked by mood, the scale on the trailing side; hovering a day shows
/// its numbers; a legend under it (two moods or more), the total, and the same numbers as a table.
private struct MoodActivityChart: View {
    let moods: [Mood]
    let counts: [DayCount]
    @State private var hovered: Date?

    private static let title = "Wallpapers, last 30 days"

    var body: some View {
        let slots = MoodActivity.colorSlots(moods)
        let segments = MoodActivity.segments(counts, order: moods.map(\.id), slots: slots)
        let days = Dictionary(grouping: segments, by: \.day)
        let total = segments.reduce(0) { $0 + $1.count }
        let ticks = MoodActivity.ticks(maximum: days.values.map { $0.reduce(0) { $0 + $1.count } }.max() ?? 0)
        let bounds = MoodActivity.dayBounds()
        let start = Date(timeIntervalSince1970: TimeInterval(bounds.first ?? 0))
        let end = Date(timeIntervalSince1970: TimeInterval(bounds.last ?? 0))
        let series = seriesShown(segments)
        VStack(alignment: .leading, spacing: 10) {
            Text(Self.title)
                .font(.title3.weight(.semibold))
                .accessibilityAddTraits(.isHeader)
            if total == 0 {
                Text("No wallpapers in the last \(MoodActivity.days) days.")
                    .quietText()
            } else {
                Chart {
                    if let hovered {
                        // The hovered day, behind its bar.
                        RectangleMark(x: .value("Day", hovered, unit: .day))
                            .foregroundStyle(Color.primary.opacity(0.07))
                            .accessibilityHidden(true)
                    }
                    ForEach(segments) { segment in
                        BarMark(
                            x: .value("Day", segment.day, unit: .day),
                            yStart: .value("Wallpapers", segment.base),
                            yEnd: .value("Wallpapers", segment.base + segment.count),
                            width: .ratio(0.62)
                        )
                        .foregroundStyle(color(segment.series, slots))
                        .clipShape(SegmentShape(topRadius: segment.isTop ? 4 : 0, gap: segment.base > 0 ? 2 : 0))
                        .accessibilityLabel("\(name(segment.series)), \(segment.day.formatted(.dateTime.weekday(.wide).month(.wide).day()))")
                        .accessibilityValue(segment.count == 1 ? "1 wallpaper" : "\(segment.count) wallpapers")
                    }
                }
                .chartXScale(domain: start...end)
                .chartYScale(domain: 0...(ticks.last ?? 1))
                .chartYAxis {
                    AxisMarks(position: .trailing, values: ticks) { value in
                        AxisGridLine(stroke: StrokeStyle(lineWidth: 1))
                        AxisValueLabel {
                            if let number = value.as(Int.self) {
                                // The quiet ink (5:1 or more), not the axis's default grey (under 4:1 in Light mode).
                                Text("\(number)").monospacedDigit().foregroundStyle(Color.primary.opacity(0.7))
                            }
                        }
                    }
                }
                .chartXAxis {
                    // A label a week apart, the last five days before today, so none runs into the scale.
                    AxisMarks(values: MoodActivity.weekLabels(bounds: bounds).map { Date(timeIntervalSince1970: TimeInterval($0)) }) { value in
                        AxisValueLabel {
                            if let day = value.as(Date.self) {
                                Text(day.formatted(.dateTime.month(.abbreviated).day())).foregroundStyle(Color.primary.opacity(0.7))
                            }
                        }
                    }
                }
                .chartLegend(.hidden)
                .chartOverlay { proxy in
                    GeometryReader { geometry in
                        Rectangle()
                            .fill(.clear)
                            .contentShape(Rectangle())
                            .onContinuousHover { phase in
                                switch phase {
                                case .active(let location):
                                    guard let plot = proxy.plotFrame else { return }
                                    let x = location.x - geometry[plot].origin.x
                                    if let date: Date = proxy.value(atX: x) {
                                        let day = Calendar.current.startOfDay(for: date)
                                        hovered = days[day] == nil ? nil : day
                                    }
                                case .ended:
                                    hovered = nil
                                }
                            }
                    }
                }
                .overlay(alignment: .topLeading) {
                    if let hovered, let stack = days[hovered] {
                        DayReadout(day: hovered, segments: stack, name: name, color: { color($0, slots) })
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }
                }
                .frame(height: 190)
                .accessibilityChartDescriptor(ActivityDescriptor(segments: segments, bounds: bounds, ticks: ticks, name: name,
                                                                 summary: MoodActivity.footnote(total: total)))
                if series.count > 1 {
                    FlowLayout(spacing: 14, lineSpacing: 6) {
                        ForEach(series, id: \.self) { item in
                            HStack(spacing: 6) {
                                Circle()
                                    .fill(color(item, slots))
                                    .frame(width: 8, height: 8)
                                Text(name(item))
                            }
                            .font(.callout)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("Legend: " + series.map(name).joined(separator: ", "))
                }
                Text(MoodActivity.footnote(total: total))
                    .font(.callout)
                    .quietText()
                DisclosureGroup("Show as Table") {
                    ActivityTable(days: days, name: name)
                }
                .font(.callout)
            }
        }
    }

    /// The moods (and "Other moods") that have bars, in stacking order.
    private func seriesShown(_ segments: [MoodActivity.Segment]) -> [MoodActivity.Series] {
        var seen: [MoodActivity.Series] = []
        let order = moods.map(\.id)
        for segment in segments where !seen.contains(segment.series) { seen.append(segment.series) }
        return seen.sorted { rank($0, order) < rank($1, order) }
    }

    private func rank(_ series: MoodActivity.Series, _ order: [String]) -> Int {
        if case .mood(let id) = series { return order.firstIndex(of: id) ?? Int.max - 1 }
        return Int.max
    }

    private func name(_ series: MoodActivity.Series) -> String {
        if case .mood(let id) = series, let mood = moods.first(where: { $0.id == id }) { return mood.name }
        return MoodActivity.otherName
    }

    private func color(_ series: MoodActivity.Series, _ slots: [String: Int]) -> Color {
        if case .mood(let id) = series { return ChartPalette.color(slot: slots[id]) }
        return ChartPalette.color(slot: nil)
    }
}

/// A stacked bar's segment: square at its base, its top corners rounded on the bar's top segment, and 2 points short
/// at the bottom when it sits on another segment, so the surface shows between them (the dataviz skill's surface gap).
private struct SegmentShape: Shape {
    let topRadius: CGFloat
    let gap: CGFloat

    func path(in rect: CGRect) -> Path {
        let bar = CGRect(x: rect.minX, y: rect.minY, width: rect.width, height: max(0, rect.height - gap))
        return Path(roundedRect: bar, cornerRadii: RectangleCornerRadii(topLeading: topRadius, bottomLeading: 0,
                                                                        bottomTrailing: 0, topTrailing: topRadius))
    }
}

/// The hovered day's numbers, values first, each keyed by a short stroke of its mood's colour.
private struct DayReadout: View {
    let day: Date
    let segments: [MoodActivity.Segment]
    let name: (MoodActivity.Series) -> String
    let color: (MoodActivity.Series) -> Color

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()))
                .font(.callout.weight(.semibold))
            ForEach(segments.reversed()) { segment in
                HStack(spacing: 6) {
                    Capsule()
                        .fill(color(segment.series))
                        .frame(width: 10, height: 3)
                    Text("\(segment.count)")
                        .font(.callout.weight(.semibold))
                        .monospacedDigit()
                    Text(name(segment.series))
                        .font(.callout)
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(.background, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
        .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
        .padding(6)
    }
}

/// The chart's numbers as a table (its accessible twin): each day with wallpapers, newest first.
private struct ActivityTable: View {
    let days: [Date: [MoodActivity.Segment]]
    let name: (MoodActivity.Series) -> String

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
            GridRow {
                Text("Day").fontWeight(.semibold)
                Text("Moods").fontWeight(.semibold)
                Text("Wallpapers").fontWeight(.semibold).gridColumnAlignment(.trailing)
            }
            .accessibilityAddTraits(.isHeader)
            Divider()
            ForEach(days.keys.sorted(by: >), id: \.self) { day in
                let stack = days[day] ?? []
                GridRow {
                    Text(day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()))
                    Text(MoodActivity.dayLine(stack, name: name))
                        .fixedSize(horizontal: false, vertical: true)
                    Text("\(stack.reduce(0) { $0 + $1.count })")
                        .monospacedDigit()
                }
                .accessibilityElement(children: .combine)
            }
        }
        .padding(.top, 6)
    }
}

/// The chart for VoiceOver's chart navigation and audio graph: one series per mood over the 30 days.
private struct ActivityDescriptor: AXChartDescriptorRepresentable {
    let segments: [MoodActivity.Segment]
    let bounds: [Int64]
    let ticks: [Int]
    let name: (MoodActivity.Series) -> String
    let summary: String

    func makeChartDescriptor() -> AXChartDescriptor {
        let days = bounds.dropLast().map { Date(timeIntervalSince1970: TimeInterval($0)) }
        let labels = days.map { $0.formatted(.dateTime.month(.abbreviated).day()) }
        let xAxis = AXCategoricalDataAxisDescriptor(title: "Day", categoryOrder: labels)
        let yAxis = AXNumericDataAxisDescriptor(title: "Wallpapers", range: 0...Double(ticks.last ?? 1),
                                                gridlinePositions: ticks.map(Double.init)) { value in
            Int(value) == 1 ? "1 wallpaper" : "\(Int(value)) wallpapers"
        }
        var order: [MoodActivity.Series] = []
        for segment in segments where !order.contains(segment.series) { order.append(segment.series) }
        let series = order.map { item in
            let byDay = Dictionary(segments.filter { $0.series == item }.map { ($0.day, $0.count) }, uniquingKeysWith: +)
            return AXDataSeriesDescriptor(name: name(item), isContinuous: false, dataPoints: zip(days, labels).map { day, label in
                AXDataPoint(x: label, y: Double(byDay[day] ?? 0))
            })
        }
        return AXChartDescriptor(title: "Wallpapers, last 30 days", summary: summary, xAxis: xAxis, yAxis: yAxis,
                                 additionalAxes: [], series: series)
    }
}
