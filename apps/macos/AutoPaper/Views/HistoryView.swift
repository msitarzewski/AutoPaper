import AutopaperCore
import QuickLook
import SwiftUI

/// History: every wallpaper, newest first, as a grid of thumbnails or a gallery (the toolbar's Grid | Gallery, like
/// Finder's view options; remembered), paged, filtered by All · Liked · Disliked · Echoes and by mood (All Moods ·
/// each mood). Each item's actions are in its context menu and in an actions button, and are its accessibility
/// actions too: Show on Desktop, Like, Dislike, Make an Echo, Show Original and Echoes, Show in Finder, Delete… (the
/// model's `wallpaperActions`, which a mood's wallpapers offer too; the main window asks before deleting).
///
/// The grid is one stop in the window's Tab order, like a Finder or Photos grid: the arrow keys select a wallpaper
/// (Home, End, Page Up and Page Down jump), Return opens its actions, Space or a double-click shows it in Quick Look,
/// and Delete (or ⌘⌫) deletes it after asking; Tab then reaches the selected wallpaper's actions button. SwiftUI
/// has no stock selectable grid (only `LazyVGrid`), so selection and these keys are AutoPaper's own, modelled on
/// Finder's icon view; there's no multiple selection.
struct HistoryView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("historyLayout") private var layout: HistoryLayout = .grid
    @State private var filter: HistoryFilter = .all
    @State private var items: [Generation] = []
    /// Each item's spoken description and whether it has echoes, fetched with its page in one engine call.
    @State private var info: [String: AppModel.TileInfo] = [:]
    @State private var exhausted = false
    @State private var loading = false
    @State private var selection: Generation.ID?
    /// Bumped by Return: the selected item pops up its actions menu.
    @State private var menuRequest = 0
    @State private var quickLook: URL?
    @State private var columns = 1
    @FocusState private var gridFocused: Bool
    @AccessibilityFocusState private var spokenItem: Generation.ID?

    private static let page: UInt32 = 60
    private static let tileMinimum: CGFloat = 200
    private static let spacing: CGFloat = 18
    private static let padding: CGFloat = 20

    var body: some View {
        Group {
            if items.isEmpty {
                if loading || model.phase != .ready {
                    ProgressView()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView(emptyTitle, systemImage: "photo.on.rectangle", description: Text(emptyDescription))
                }
            } else if layout == .gallery {
                HistoryGallery(items: items, info: info, selection: $selection, quickLook: $quickLook) {
                    await loadMore()
                }
            } else {
                grid
            }
        }
        .navigationTitle("History")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Picker("View", selection: $layout) {
                    ForEach(HistoryLayout.allCases) { layout in
                        Label(layout.title, systemImage: layout.symbol).tag(layout)
                    }
                }
                .pickerStyle(.segmented)
                .labelStyle(.iconOnly)
                .help("Show as a grid or a gallery")
            }
            ToolbarItem(placement: .primaryAction) {
                Picker("Mood", selection: moodFilter) {
                    Text("All Moods").tag(Mood.ID?.none)
                    if !model.moods.isEmpty { Divider() }
                    ForEach(model.moods) { mood in
                        Text(mood.name).tag(Mood.ID?.some(mood.id))
                    }
                }
                .pickerStyle(.menu)
                .help("Wallpapers made in one mood, or in every mood")
            }
            ToolbarItem(placement: .primaryAction) {
                Picker("Show", selection: $filter) {
                    Text("All").tag(HistoryFilter.all)
                    Text("Liked").tag(HistoryFilter.liked)
                    Text("Disliked").tag(HistoryFilter.disliked)
                    Text("Echoes").tag(HistoryFilter.echoes)
                }
                .pickerStyle(.segmented)
                .help("Which wallpapers to show")
            }
        }
        .task(id: ReloadKey(filter: filter, mood: model.historyMood, revision: model.historyRevision, ready: model.phase == .ready)) {
            await reload()
        }
        .quickLookPreview($quickLook)
    }

    private var grid: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: Self.tileMinimum, maximum: 280), spacing: Self.spacing, alignment: .top)],
                    spacing: 22
                ) {
                    ForEach(items) { generation in
                        let selected = selection == generation.id
                        HistoryTile(
                            generation: generation,
                            info: info[generation.id],
                            actions: actions,
                            selected: selected,
                            gridFocused: gridFocused,
                            menuRequest: selected ? menuRequest : nil
                        )
                        .id(generation.id)
                        // Double-click: Quick Look, as in Finder (Space does the same from the keyboard).
                        .onTapGesture(count: 2) {
                            selection = generation.id
                            gridFocused = true
                            preview(generation)
                        }
                        .onTapGesture {
                            selection = generation.id
                            gridFocused = true
                        }
                        .accessibilityFocused($spokenItem, equals: generation.id)
                        .onAppear {
                            if generation.id == items.last?.id { Task { await loadMore() } }
                        }
                    }
                }
                .padding(Self.padding)
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in
                    // How many columns the adaptive grid lays out, for Up and Down.
                    let available = width - 2 * Self.padding
                    columns = max(1, Int((available + Self.spacing) / (Self.tileMinimum + Self.spacing)))
                }
            }
            .focusable()
            .focused($gridFocused)
            // The selected tile shows focus (accent outline) instead of a ring around the whole grid.
            .focusEffectDisabled()
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Wallpapers")
            .accessibilityValue(items.count == 1 ? "1 wallpaper" : "\(items.count) wallpapers\(exhausted ? "" : " so far")")
            .accessibilityHint(Self.keyboardHint)
            .help(Self.keyboardHint)
            .onChange(of: gridFocused) { _, focused in
                if focused, selection == nil || !items.contains(where: { $0.id == selection }) {
                    selection = items.first?.id
                }
            }
            .onMoveCommand { direction in
                moveSelection(direction)
                if let selection {
                    withAnimation(nil) { proxy.scrollTo(selection) }
                    spokenItem = selection
                }
            }
            .onKeyPress(.return) {
                guard selectedItem != nil else { return .ignored }
                menuRequest += 1
                return .handled
            }
            .onKeyPress(.space) {
                guard let item = selectedItem, item.imagePath ?? item.thumbPath != nil else { return .ignored }
                if quickLook == nil { preview(item) } else { quickLook = nil }
                return .handled
            }
            // Home and End go to the first and last wallpaper loaded; Page Up and Page Down by about a screenful.
            .onKeyPress(keys: [.home, .end, .pageUp, .pageDown]) { press in
                guard !items.isEmpty else { return .ignored }
                let index = items.firstIndex { $0.id == selection } ?? 0
                let page = columns * 3
                let target: Int
                switch press.key {
                case .home: target = 0
                case .end: target = items.count - 1
                case .pageUp: target = max(0, index - page)
                default: target = min(items.count - 1, index + page)
                }
                selection = items[target].id
                withAnimation(nil) { proxy.scrollTo(items[target].id) }
                spokenItem = items[target].id
                return .handled
            }
            // Delete, ⌘⌫ (as in Finder and Photos) and ⌦, with or without modifiers.
            .onKeyPress(keys: [.delete, .deleteForward]) { _ in
                guard let item = selectedItem else { return .ignored }
                model.wallpaperToDelete = item
                return .handled
            }
            .onDeleteCommand {
                if let item = selectedItem { model.wallpaperToDelete = item }
            }
        }
    }

    private var selectedItem: Generation? {
        items.first { $0.id == selection }
    }

    static let keyboardHint = "Arrow keys choose a wallpaper. Return shows its actions, Space previews it, and Delete deletes it."

    /// Quick Look on the original (or the thumbnail, when the original was removed to save space).
    private func preview(_ generation: Generation) {
        guard let path = generation.imagePath ?? generation.thumbPath else { return }
        quickLook = URL(filePath: path)
    }

    /// The toolbar's mood menu: History's own filter, also set by a mood's "Show All in History".
    private var moodFilter: Binding<Mood.ID?> {
        Binding(get: { model.historyMood }, set: { model.historyMood = $0 })
    }

    private func moveSelection(_ direction: MoveCommandDirection) {
        guard !items.isEmpty else { return }
        let index = items.firstIndex { $0.id == selection } ?? -1
        let step: Int
        switch direction {
        case .left: step = -1
        case .right: step = 1
        case .up: step = -columns
        case .down: step = columns
        @unknown default: step = 0
        }
        let target = index < 0 ? 0 : index + step
        guard items.indices.contains(target) else { return }
        selection = items[target].id
    }

    private var actions: HistoryActions {
        model.wallpaperActions
    }

    private var emptyTitle: String {
        switch filter {
        case .all: "No Wallpapers Yet"
        case .liked: "No Liked Wallpapers"
        case .disliked: "No Disliked Wallpapers"
        case .echoes: "No Echoes Yet"
        }
    }

    private var emptyDescription: String {
        if let mood = model.mood(model.historyMood), filter == .all {
            return "Wallpapers made while \(mood.name) is the current mood appear here."
        }
        return switch filter {
        case .all: "Every wallpaper AutoPaper makes appears here."
        case .liked: "Wallpapers you like appear here, and can come back when a new one can't be made."
        case .disliked: "Wallpapers you dislike appear here."
        case .echoes: "Once an idea's quiet period has passed, it may come back as an echo: a new take on the old one."
        }
    }

    /// Reloads what's shown (at least one page), keeping as many items as were loaded so the scroll position holds.
    private func reload() async {
        guard model.phase == .ready else { return }
        loading = true
        defer { loading = false }
        if let mood = model.historyMood, model.mood(mood) == nil {
            // That mood was deleted: back to every mood.
            model.historyMood = nil
            return
        }
        let count = max(Self.page, UInt32(items.count))
        do {
            let page = try await model.history(filter, mood: model.historyMood, limit: count, offset: 0)
            info = await model.tileInfo(for: page)
            items = page
            exhausted = page.count < Int(count)
            if let selection, !page.contains(where: { $0.id == selection }) { self.selection = nil }
        } catch {
            model.wallpaperProblem = model.sentence(for: error)
        }
    }

    private func loadMore() async {
        guard !exhausted, !loading else { return }
        loading = true
        defer { loading = false }
        do {
            let page = try await model.history(filter, mood: model.historyMood, limit: Self.page, offset: UInt32(items.count))
            let known = Set(items.map(\.id))
            let fresh = page.filter { !known.contains($0.id) }
            info.merge(await model.tileInfo(for: fresh)) { _, new in new }
            items.append(contentsOf: fresh)
            exhausted = page.count < Int(Self.page)
        } catch {
            model.wallpaperProblem = model.sentence(for: error)
        }
    }

    private struct ReloadKey: Equatable {
        let filter: HistoryFilter
        let mood: Mood.ID?
        let revision: Int
        let ready: Bool
    }
}

extension HistoryActions {
    /// The same actions as an AppKit menu, for Return in the grid.
    @MainActor
    func menu(for generation: Generation, hasEchoes: Bool) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        for (index, group) in groups(for: generation, hasEchoes: hasEchoes).enumerated() {
            if index > 0 { menu.addItem(.separator()) }
            for item in group {
                menu.addItem(ClosureMenuItem(item.title, enabled: item.isEnabled, checked: item.isOn == true, handler: item.perform))
            }
        }
        return menu
    }
}

/// One wallpaper in History: thumbnail, title, date and badges (icon and word: Liked, Disliked, Echo). One
/// accessibility element spoken as the engine's description plus the rating, with the actions attached.
private struct HistoryTile: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    let generation: Generation
    /// Its spoken description and whether it has echoes (loaded with the page; nil until then).
    let info: AppModel.TileInfo?
    let actions: HistoryActions
    let selected: Bool
    let gridFocused: Bool
    /// Set on the selected tile: a new value pops up its actions menu (Return).
    let menuRequest: Int?

    private var hasEchoes: Bool { info?.hasEchoes ?? false }

    var body: some View {
        let groups = actions.groups(for: generation, hasEchoes: hasEchoes)
        VStack(alignment: .leading, spacing: 8) {
            WallpaperImage(generation, contentMode: .fill, maxPixelSize: 640, preferThumbnail: true)
                .frame(minWidth: 0, maxWidth: .infinity)
                .aspectRatio(16 / 10, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(.separator)
                }
            Text(generation.concept.title)
                .font(.headline)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            // The badges drop below the date when a narrow tile can't fit them beside it.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 6) {
                    date
                    Spacer(minLength: 4)
                    Badges(generation: generation)
                }
                VStack(alignment: .leading, spacing: 4) {
                    date
                    Badges(generation: generation)
                }
            }
        }
        .padding(6)
        .background {
            if selected {
                // The selection: an accent outline while the grid has keyboard focus (grey when it doesn't), not
                // colour alone (the outline itself marks it).
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(gridFocused ? Color.accentColor.opacity(0.18) : Color.secondary.opacity(0.12))
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(gridFocused ? Color.accentColor : Color.secondary, lineWidth: 3)
            }
        }
        .background {
            if let menuRequest {
                ActionsMenuPresenter(request: menuRequest) { actions.menu(for: generation, hasEchoes: hasEchoes) }
            }
        }
        .contentShape(Rectangle())
        .contextMenu { HistoryMenuItems(groups: groups) }
        // Which models made it (user, 2026-10-06), in the help tag and after the date for VoiceOver.
        .help(provenance.text)
        .accessibilityElement(children: .ignore)
        .accessibilityAddTraits(selected ? [.isImage, .isSelected] : .isImage)
        .accessibilityLabel(info?.spoken ?? Spoken.wallpaper(description: generation.concept.title, rating: generation.rating))
        .accessibilityValue("\(Formatting.day(generation.createdAt)). \(provenance.spoken)")
        .accessibilityHint(HistoryView.keyboardHint)
        .accessibilityActions {
            // Only what can be done now, as in the menus.
            ForEach(groups.joined().filter(\.isEnabled)) { item in
                Button(item.spokenTitle) { item.perform() }
            }
        }
        .overlay(alignment: .topTrailing) {
            Menu {
                HistoryMenuItems(groups: groups)
            } label: {
                Label("Actions for \(generation.concept.title)", systemImage: "ellipsis")
                    .labelStyle(.iconOnly)
            }
            .menuStyle(.button)
            .menuIndicator(.hidden)
            .buttonStyle(.borderless)
            .fixedSize()
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            // An opaque plate: the glyph (and any focus ring) has the same contrast over every picture.
            .background(Color(nsColor: .windowBackgroundColor).opacity(reduceTransparency ? 1 : 0.92), in: Capsule())
            .overlay(Capsule().strokeBorder(.separator))
            .padding(12)
            .help("Actions")
            // Only the selected wallpaper's button is in the Tab order (grid, then its actions), so the grid isn't
            // one Tab stop per wallpaper; Return on the grid opens the same menu.
            .focusable(selected)
            // The focusable wrapper is what Tab lands on: it needs the name too (found in the VM's AX audit).
            .accessibilityLabel("Actions for \(generation.concept.title)")
        }
    }

    private var date: some View {
        Text(Formatting.day(generation.createdAt))
            .font(.callout)
            .lineLimit(1)
    }

    private var provenance: Provenance.Line {
        Provenance.line(generation, listed: model.modelNames)
    }

}

/// Pops up a history item's actions as a native menu under it (Return in the grid; a SwiftUI `Menu` can't be
/// opened from code). Shows the menu when `request` changes after it appeared.
private struct ActionsMenuPresenter: NSViewRepresentable {
    let request: Int
    let menu: @MainActor () -> NSMenu

    final class Coordinator {
        var shown: Int
        init(shown: Int) { self.shown = shown }
    }

    func makeCoordinator() -> Coordinator { Coordinator(shown: request) }

    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ view: NSView, context: Context) {
        guard request != context.coordinator.shown else { return }
        context.coordinator.shown = request
        let menu = menu()
        // After this update: the menu tracks in its own loop.
        DispatchQueue.main.async {
            guard view.window != nil else { return }
            let below = NSPoint(x: 6, y: view.isFlipped ? view.bounds.maxY : view.bounds.minY)
            menu.popUp(positioning: menu.items.first, at: below, in: view)
        }
    }
}

/// Liked / Disliked / Echo, each as an icon and a word, each kept on one line.
private struct Badges: View {
    let generation: Generation

    var body: some View {
        HStack(spacing: 4) {
            if generation.rating == .liked { badge("Liked", "hand.thumbsup.fill") }
            if generation.rating == .disliked { badge("Disliked", "hand.thumbsdown.fill") }
            if generation.echoOf != nil { badge("Echo", "arrow.trianglehead.2.clockwise.rotate.90") }
        }
    }

    private func badge(_ title: String, _ symbol: String) -> some View {
        Label(title, systemImage: symbol)
            .font(.caption)
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(.quaternary, in: Capsule())
    }
}

/// The actions of a history item, for its context menu and its actions button.
struct HistoryMenuItems: View {
    let groups: [[HistoryActions.Item]]

    var body: some View {
        ForEach(Array(groups.enumerated()), id: \.offset) { index, group in
            if index > 0 { Divider() }
            ForEach(group) { item in
                if let isOn = item.isOn {
                    Toggle(item.title, isOn: Binding(get: { isOn }, set: { _ in item.perform() }))
                        .disabled(!item.isEnabled)
                } else {
                    Button(item.title, role: item.isDestructive ? .destructive : nil) { item.perform() }
                        .disabled(!item.isEnabled)
                }
            }
        }
    }
}

/// "Show Original and Echoes": the original and every echo of it, oldest first.
struct LineageSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let generation: Generation
    @State private var lineage: [Generation]?
    @State private var problem: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Original and Echoes")
                .font(.title2.weight(.semibold))
                .padding([.horizontal, .top], 20)
                .padding(.bottom, 4)
                .accessibilityAddTraits(.isHeader)
            Text("Echoes are new takes on an idea after its quiet period: the same kind of place, at another hour, season or age.")
                .padding(.horizontal, 20)
                .padding(.bottom, 12)
                .fixedSize(horizontal: false, vertical: true)
            Divider()
            Group {
                if let lineage {
                    List(lineage) { item in
                        LineageRow(generation: item, isOriginal: item.echoOf == nil, names: model.modelNames) {
                            model.showOnDesktop(item)
                        }
                    }
                    .listStyle(.inset)
                } else if let problem {
                    ContentUnavailableView("Couldn't Load", systemImage: "exclamationmark.triangle", description: Text(problem))
                } else {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(minHeight: 280)
            Divider()
            HStack {
                if let lineage, lineage.count <= 1 {
                    Text("No echoes of this one yet.")
                }
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(16)
        }
        .frame(width: 560, height: 520)
        .task {
            do {
                lineage = try await model.lineage(of: generation.id)
            } catch {
                problem = model.sentence(for: error)
            }
        }
    }
}

private struct LineageRow: View {
    let generation: Generation
    let isOriginal: Bool
    /// Model names from the providers' lists (`AppModel.modelNames`).
    let names: [String: String]
    let show: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            WallpaperImage(generation, contentMode: .fill, maxPixelSize: 320, preferThumbnail: true)
                .frame(width: 112, height: 70)
                .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(generation.concept.title).font(.headline)
                Text(isOriginal ? "Original · \(Formatting.day(generation.createdAt))" : "Echo · \(Formatting.day(generation.createdAt))")
                    .font(.callout)
                if let note = generation.echoNote, !note.isEmpty {
                    Text(note).font(.callout).lineLimit(2)
                }
                Text(Provenance.line(generation, listed: names).text)
                    .font(.callout)
                    .quietText()
                    .lineLimit(2)
                    .accessibilityRepresentation { Text(Provenance.line(generation, listed: names).spoken) }
            }
            Spacer()
            Button("Show on Desktop", action: show)
                .disabled(generation.imagePath == nil)
                // Starts with the visible title (WCAG 2.5.3 Label in Name), so "Click Show on Desktop" finds it.
                .accessibilityLabel("Show on Desktop: \(generation.concept.title)")
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Gallery

/// History's Gallery (user, 2026-10-06; like Finder's Gallery view): the selected wallpaper large, a filmstrip of
/// thumbnails along the bottom, and its details beside it. The filmstrip is one stop in the Tab order: Left and Right
/// choose (Home and End jump; the selected one is kept in view, without animation under Reduce Motion), Return shows
/// it on the desktop, Space previews it in Quick Look, Delete deletes it (after asking). The next page loads as the
/// selection nears the end of what's loaded. Its details hold every action as buttons.
private struct HistoryGallery: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let items: [Generation]
    let info: [String: AppModel.TileInfo]
    @Binding var selection: Generation.ID?
    @Binding var quickLook: URL?
    let loadMore: () async -> Void
    @FocusState private var stripFocused: Bool
    @AccessibilityFocusState private var spokenItem: Generation.ID?

    var body: some View {
        let selected = items.first { $0.id == selection } ?? items.first
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                if let selected {
                    WallpaperImage(selected, maxPixelSize: 2_400)
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .overlay {
                            RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator)
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .padding(20)
                        .contentShape(Rectangle())
                        .onTapGesture(count: 2) { preview(selected) }
                        .accessibilityElement()
                        .accessibilityAddTraits(.isImage)
                        .accessibilityLabel(spoken(selected))
                        .id(selected.id)
                }
                Divider()
                filmstrip
            }
            .frame(minWidth: 360)
            Divider()
            if let selected {
                GalleryDetails(generation: selected, info: info[selected.id], items: items, selection: $selection)
                    .frame(width: 300)
            }
        }
        .onAppear {
            if selection == nil || !items.contains(where: { $0.id == selection }) { selection = items.first?.id }
        }
    }

    private var filmstrip: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                LazyHStack(spacing: 8) {
                    ForEach(items) { generation in
                        FilmstripItem(generation: generation, spoken: spoken(generation),
                                      selected: generation.id == (selection ?? items.first?.id), focused: stripFocused)
                            .id(generation.id)
                            .onTapGesture(count: 2) {
                                selection = generation.id
                                preview(generation)
                            }
                            .onTapGesture {
                                selection = generation.id
                                stripFocused = true
                            }
                            .accessibilityFocused($spokenItem, equals: generation.id)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
            }
            .frame(height: 104)
            .focusable()
            .focused($stripFocused)
            .focusEffectDisabled()
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Wallpapers")
            .accessibilityValue(items.count == 1 ? "1 wallpaper" : "\(items.count) wallpapers")
            .accessibilityHint(Self.keyboardHint)
            .help(Self.keyboardHint)
            // Key presses rather than move commands: the horizontal scroll view would take Left and Right to scroll.
            .onKeyPress(keys: [.leftArrow, .rightArrow]) { press in
                select(Filmstrip.move(from: index, by: press.key == .leftArrow ? -1 : 1, count: items.count))
                return .handled
            }
            .onKeyPress(keys: [.home, .end]) { press in
                select(press.key == .home ? 0 : items.count - 1)
                return .handled
            }
            .onKeyPress(.return) {
                guard let item = selectedItem else { return .ignored }
                let show = model.wallpaperActions.groups(for: item, hasEchoes: false).first?.first
                if let show, show.isEnabled { show.perform() }
                return .handled
            }
            .onKeyPress(.space) {
                guard let item = selectedItem else { return .ignored }
                if quickLook == nil { preview(item) } else { quickLook = nil }
                return .handled
            }
            .onKeyPress(keys: [.delete, .deleteForward]) { _ in
                guard let item = selectedItem else { return .ignored }
                model.wallpaperToDelete = item
                return .handled
            }
            .onChange(of: selection) { _, id in
                guard let id else { return }
                // Kept in view, scrolling only as far as needed (Finder's filmstrip).
                if reduceMotion {
                    proxy.scrollTo(id)
                } else {
                    withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(id) }
                }
                if let index, Filmstrip.needsMore(at: index, count: items.count, exhausted: false) {
                    Task { await loadMore() }
                }
            }
        }
    }

    private static let keyboardHint = "Left and right arrows choose a wallpaper. Return shows it on the desktop, Space previews it, and Delete deletes it."

    private var index: Int? {
        items.firstIndex { $0.id == selection }
    }

    private var selectedItem: Generation? {
        items.first { $0.id == selection } ?? items.first
    }

    private func select(_ target: Int?) {
        guard let target, items.indices.contains(target) else { return }
        selection = items[target].id
        spokenItem = items[target].id
    }

    private func spoken(_ generation: Generation) -> String {
        info[generation.id]?.spoken ?? Spoken.wallpaper(description: generation.concept.title, rating: generation.rating)
    }

    /// Quick Look on the original (or the thumbnail, when the original was removed to save space).
    private func preview(_ generation: Generation) {
        guard let path = generation.imagePath ?? generation.thumbPath else { return }
        quickLook = URL(filePath: path)
    }
}

/// A thumbnail in the filmstrip: the selection is an outline (accent while the filmstrip has the keyboard, grey when
/// not), never colour alone. Spoken as the engine's description and the rating.
private struct FilmstripItem: View {
    let generation: Generation
    let spoken: String
    let selected: Bool
    let focused: Bool

    var body: some View {
        WallpaperImage(generation, contentMode: .fill, maxPixelSize: 320, preferThumbnail: true)
            .frame(width: 120, height: 75)
            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(.separator)
            }
            .padding(3)
            .overlay {
                if selected {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(focused ? Color.accentColor : Color.secondary, lineWidth: 3)
                }
            }
            .contentShape(Rectangle())
            .accessibilityElement()
            .accessibilityAddTraits(selected ? [.isImage, .isSelected] : .isImage)
            .accessibilityLabel(spoken)
            .accessibilityValue(Formatting.day(generation.createdAt))
    }
}

/// The selected wallpaper's details in the Gallery: its title, summary, echo note (and its original), mood, the
/// keywords it was made with, which models made it, when, the rating, and every action as a button (Show on Desktop
/// first).
private struct GalleryDetails: View {
    @Environment(AppModel.self) private var model
    let generation: Generation
    let info: AppModel.TileInfo?
    let items: [Generation]
    @Binding var selection: Generation.ID?

    var body: some View {
        let groups = model.wallpaperActions.groups(for: generation, hasEchoes: info?.hasEchoes ?? false)
        let all = groups.joined()
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(generation.concept.title)
                    .font(.title3.weight(.semibold))
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                Text(generation.concept.summary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                if let note = generation.echoNote, !note.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        Label(note, systemImage: "arrow.trianglehead.2.clockwise.rotate.90")
                            .fixedSize(horizontal: false, vertical: true)
                        if let original = generation.echoOf {
                            Button("Show Original") {
                                if items.contains(where: { $0.id == original }) {
                                    selection = original
                                } else {
                                    model.lineageOf = generation
                                }
                            }
                            .buttonStyle(.link)
                            .help("The wallpaper this one echoes")
                        }
                    }
                }
                VStack(alignment: .leading, spacing: 6) {
                    LabeledContent("Made", value: Date(timeIntervalSince1970: TimeInterval(generation.createdAt))
                        .formatted(date: .abbreviated, time: .shortened))
                    LabeledContent("Mood", value: generation.moodName ?? "A deleted mood")
                    let keywords = MoodText.keywordGroups(snapshots: generation.keywords)
                    if !keywords.isEmpty {
                        LabeledContent("Keywords") {
                            VStack(alignment: .trailing, spacing: 2) {
                                ForEach(keywords, id: \.weight) { group in
                                    Text(group.line)
                                        .multilineTextAlignment(.trailing)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                            }
                        }
                    }
                }
                .font(.callout)
                ProvenanceLine(generation: generation)
                HStack(spacing: 8) {
                    RatingToggle(generation: generation, rating: .liked)
                    RatingToggle(generation: generation, rating: .disliked)
                }
                Divider()
                VStack(alignment: .leading, spacing: 8) {
                    if let show = all.first(where: { $0.title == "Show on Desktop" }) {
                        Button(show.title) { show.perform() }
                            .buttonStyle(.borderedProminent)
                            .disabled(!show.isEnabled)
                            .help("Return")
                    }
                    ForEach(all.filter { $0.isOn == nil && $0.title != "Show on Desktop" }) { item in
                        Button(item.title, role: item.isDestructive ? .destructive : nil) { item.perform() }
                            .disabled(!item.isEnabled)
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Details")
    }
}
