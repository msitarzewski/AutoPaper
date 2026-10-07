import AutopaperCore
import SwiftUI

/// The main window: a sidebar with Now, Moods and History (View ▸ ⌘1–⌘3), the section on the right. Moods is a
/// disclosure group listing every mood (the current one checkmarked); choosing the Moods row shows the summary of all
/// moods, choosing a mood shows its detail. A stationary footer under the sidebar holds Settings and a one-line status.
///
/// Moods is a first-party three-column layout (the user's layout, after their own app's library): sidebar › mood
/// list, titled "Moods" with its + in the list's part of the toolbar › the selected mood, its name the title at the
/// leading edge of the detail's part of the toolbar (renamed in place) and Use This Mood at its trailing end. Now and
/// History have no middle column, so, as the user's app does for its sections of a different shape, each shape is a
/// split view of its own: three columns for Moods, two for Now and History, sharing the sidebar, its width and whether
/// it's shown. Changing shape rebuilds the sidebar, so when the sidebar had the keyboard the new one takes it back
/// (arrowing through Now, Moods, the moods and History, or View ▸ ⌘1–⌘3, keeps the person's place). The mood list
/// sits beside the sidebar rather than under it (`SplitViewSetup`), so Tab goes sidebar › list › detail › toolbar.
struct MainWindow: View {
    @Environment(AppModel.self) private var model
    /// Each shape's column visibility: the sidebar shown or hidden (the toolbar's sidebar button, View ▸ Hide
    /// Sidebar), the same in both.
    @State private var twoColumns: NavigationSplitViewVisibility = .all
    @State private var threeColumns: NavigationSplitViewVisibility = .all
    @State private var memory = SplitViewMemory()
    @FocusState private var sidebarFocused: Bool

    var body: some View {
        let ideal = columnWidths()
        Group {
            if model.section == .moods {
                NavigationSplitView(columnVisibility: $threeColumns) {
                    sidebar(width: ideal.sidebar)
                } content: {
                    MoodList()
                        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { memory.measured(list: $0) }
                        // Last, on the column's outermost view, where the split view reads it.
                        .navigationSplitViewColumnWidth(min: SplitViewMemory.listRange.lowerBound, ideal: ideal.list,
                                                        max: SplitViewMemory.listRange.upperBound)
                } detail: {
                    MoodDetailColumn()
                        .frame(minWidth: 440)
                        .modifier(WallpaperDialogs())
                }
            } else {
                NavigationSplitView(columnVisibility: $twoColumns) {
                    sidebar(width: ideal.sidebar)
                } detail: {
                    Group {
                        if model.section == .history { HistoryView() } else { NowView() }
                    }
                    .modifier(WallpaperDialogs())
                }
            }
        }
        .frame(minWidth: 860, minHeight: 540)
        .modifier(MoodDialogs())
        // Both kept in step, so the next shape is built with the sidebar as the person left it (a hidden sidebar is
        // `.detailOnly` with two columns, `.doubleColumn`, the list and the mood, with three).
        .onChange(of: twoColumns) { _, visibility in
            let hidden = visibility == .detailOnly
            if hidden != Self.sidebarHidden(threeColumns) { threeColumns = hidden ? .doubleColumn : .all }
        }
        .onChange(of: threeColumns) { _, visibility in
            let hidden = Self.sidebarHidden(visibility)
            if hidden != (twoColumns == .detailOnly) { twoColumns = hidden ? .detailOnly : .all }
        }
    }

    /// The widths to build the split view with. When the shape changes, the old sidebar still has the keyboard as this
    /// is worked out, so the new sidebar is told to take it back, unless a new or renamed mood is asking for its name
    /// to be typed, or a keyword problem's link for its keyword to be selected.
    private func columnWidths() -> (sidebar: CGFloat, list: CGFloat) {
        let ideal = memory.ideal(forMoods: model.section == .moods)
        if ideal.rebuilt, sidebarFocused, model.moodToName == nil, model.keywordToSelect == nil { memory.refocusSidebar = true }
        return (ideal.sidebar, ideal.list)
    }

    private static func sidebarHidden(_ threeColumns: NavigationSplitViewVisibility) -> Bool {
        threeColumns == .doubleColumn || threeColumns == .detailOnly
    }

    private func sidebar(width: CGFloat) -> some View {
        Sidebar(focused: $sidebarFocused)
            .onAppear {
                guard memory.refocusSidebar else { return }
                memory.refocusSidebar = false
                // Once the new split view is in the window.
                DispatchQueue.main.async { sidebarFocused = true }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { memory.measured(sidebar: $0) }
            .background(SplitViewSetup())
            // Last, on the column's outermost view, where the split view reads it.
            .navigationSplitViewColumnWidth(min: SplitViewMemory.sidebarRange.lowerBound, ideal: width,
                                            max: SplitViewMemory.sidebarRange.upperBound)
    }
}

/// What the window carries from one split view to the next: the sidebar's and the mood list's widths as the person
/// leaves them (each split view would open its columns at their ideal widths; kept across launches too), and whether
/// the new sidebar takes the keyboard. Not observed, so a drag doesn't redraw the window; and a split view is given the
/// widths of the moment it's built (`ideal(forMoods:)`), never one measured while it's on screen, which it would take
/// as a new ideal and apply.
@MainActor
private final class SplitViewMemory {
    static let sidebarRange: ClosedRange<CGFloat> = 170...280
    static let listRange: ClosedRange<CGFloat> = 220...360

    /// The new split view's sidebar takes the keyboard when it appears (see `MainWindow.columnWidths()`).
    var refocusSidebar = false

    /// What the columns measure now. Only widths in their range: a column being shown, hidden or taken down passes
    /// through others, which aren't the person's choice.
    private var sidebar: CGFloat
    private var list: CGFloat
    /// The shape of the split view on screen and the widths it was built with.
    private var built: (moods: Bool, sidebar: CGFloat, list: CGFloat)?

    init() {
        let defaults = UserDefaults.standard
        sidebar = Self.saved(defaults.double(forKey: Self.sidebarKey), in: Self.sidebarRange) ?? 210
        list = Self.saved(defaults.double(forKey: Self.listKey), in: Self.listRange) ?? 270
    }

    /// The ideal widths for a split view of this shape: the current ones when the shape changes (`rebuilt`), then
    /// the same for as long as it's on screen.
    func ideal(forMoods moods: Bool) -> (sidebar: CGFloat, list: CGFloat, rebuilt: Bool) {
        if let built, built.moods == moods { return (built.sidebar, built.list, false) }
        let first = built == nil
        built = (moods, sidebar, list)
        return (sidebar, list, !first)
    }

    func measured(sidebar width: CGFloat) {
        guard Self.sidebarRange.contains(width), width != sidebar else { return }
        sidebar = width
        UserDefaults.standard.set(Double(width), forKey: Self.sidebarKey)
    }

    func measured(list width: CGFloat) {
        guard Self.listRange.contains(width), width != list else { return }
        list = width
        UserDefaults.standard.set(Double(width), forKey: Self.listKey)
    }

    private static let sidebarKey = "mainWindowSidebarWidth"
    private static let listKey = "mainWindowMoodListWidth"

    private static func saved(_ value: Double, in range: ClosedRange<CGFloat>) -> CGFloat? {
        range.contains(CGFloat(value)) ? CGFloat(value) : nil
    }
}

/// Sets up the split view AppKit-side, once SwiftUI has built it (found the way the user's app finds its split view to
/// autosave its columns: up the superviews from a view in a column; placed in the sidebar, which both shapes have):
/// - SwiftUI autosaves both shapes' split views under one name, so each restored the other's columns (a sidebar
///   narrower than its minimum, a list the wrong width; seen in the test VM, 2026-10-06). That's turned off, and what
///   it saved is cleared; `SplitViewMemory` keeps the widths instead.
/// - SwiftUI turns on `automaticallyAdjustsSafeAreaInsets` for the middle column, which lays it out from the window's
///   left edge under the floating sidebar. The window orders Tab by the columns' left edges, so Tab went sidebar ›
///   detail › toolbar › list; with the mood list beside the sidebar it goes sidebar › list › detail › toolbar.
private struct SplitViewSetup: NSViewRepresentable {
    func makeNSView(context: Context) -> Probe { Probe() }
    func updateNSView(_ view: Probe, context: Context) {}

    final class Probe: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            DispatchQueue.main.async { [weak self] in self?.setUp() }
        }

        private func setUp() {
            guard window != nil else { return }
            var candidate = superview
            while let view = candidate, !(view is NSSplitView) { candidate = view.superview }
            guard let split = candidate as? NSSplitView else { return }
            if let name = split.autosaveName {
                split.autosaveName = nil
                UserDefaults.standard.removeObject(forKey: "NSSplitView Subview Frames \(name)")
            }
            if let controller = split.delegate as? NSSplitViewController, controller.splitViewItems.count == 3 {
                controller.splitViewItems[1].automaticallyAdjustsSafeAreaInsets = false
            }
        }
    }
}

/// The sidebar: Now, Moods (a disclosure group of every mood, expanded until the person closes it, remembered), and
/// History; the footer stays at the bottom whatever the list scrolls.
private struct Sidebar: View {
    @Environment(AppModel.self) private var model
    @AppStorage("sidebarMoodsExpanded") private var moodsExpanded = true
    var focused: FocusState<Bool>.Binding

    var body: some View {
        List(selection: selection) {
            ForEach(MainSection.allCases) { section in
                if section == .moods {
                    DisclosureGroup(isExpanded: $moodsExpanded) {
                        ForEach(model.moods) { mood in
                            MoodSidebarRow(mood: mood)
                                .tag(SidebarItem.mood(mood.id))
                        }
                    } label: {
                        Label(section.title, systemImage: section.symbol)
                            .tag(SidebarItem.section(section))
                    }
                } else {
                    Label(section.title, systemImage: section.symbol)
                        .tag(SidebarItem.section(section))
                }
            }
        }
        .focused(focused)
        .contextMenu(forSelectionType: SidebarItem.self) { items in
            // A mood's row has the mood list's own menu; the sections have none.
            if case .mood(let id)? = items.first, let mood = model.mood(id) {
                MoodMenuItems(mood: mood)
            }
        }
        .onDeleteCommand {
            if case .mood(let id) = showing, model.moods.count > 1 { model.moodToDelete = id }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            SidebarFooter()
        }
    }

    /// The row marking where the window is (`SidebarItem.showing`).
    private var showing: SidebarItem {
        SidebarItem.showing(section: model.section, mood: model.mood(model.moodSelection)?.id, moodsExpanded: moodsExpanded)
    }

    private var selection: Binding<SidebarItem?> {
        Binding(
            get: { showing },
            set: { item in
                // Only a choice the person made moves the window (the list may hand back the row it shows).
                guard let item, item != showing else { return }
                let destination = item.destination(keeping: model.moodSelection)
                model.section = destination.section
                model.moodSelection = destination.mood
            }
        )
    }
}

/// A mood in the sidebar: its name, and a checkmark on the current one (spoken "Rainy beach, current mood").
private struct MoodSidebarRow: View {
    let mood: Mood

    var body: some View {
        HStack(spacing: 6) {
            Label(mood.name, systemImage: "swatchpalette")
                .lineLimit(1)
            Spacer(minLength: 4)
            if mood.active {
                Image(systemName: "checkmark")
                    .font(.callout.weight(.semibold))
            }
        }
        // VoiceOver reads a text element's value, not a label put on it, so the spoken line is a text of its own.
        .accessibilityRepresentation {
            Text(MoodText.spokenName(mood))
        }
        .help(mood.active ? "\(mood.name), the current mood" : mood.name)
    }
}

/// The sidebar's footer (user, 2026-10-06), like the "Synced with iCloud" footer of first-party Mac apps: a hairline
/// above, Settings (⌘,) as a gear at the left and a quiet one-line status beside it ("Next wallpaper at 3:00 PM",
/// "Painting…", "Paused"). The status is plain text, spoken whole even when the sidebar is too narrow to show it all.
private struct SidebarFooter: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 5) {
                Button {
                    SettingsWindow.show(openSettings)
                } label: {
                    Label("Settings", systemImage: "gearshape")
                        .labelStyle(.iconOnly)
                        .font(.body)
                }
                .buttonStyle(.borderless)
                .help("Settings (⌘,)")
                if let (full, short) = status {
                    // The whole sentence when the sidebar is wide enough, else its short form; VoiceOver and the help
                    // tag always have the whole sentence.
                    ViewThatFits(in: .horizontal) {
                        Text(full)
                        Text(short)
                        Text(short)
                            .font(.footnote)
                            .truncationMode(.tail)
                    }
                    .font(.subheadline)
                    .quietText()
                    .lineLimit(1)
                    .help(full)
                    .accessibilityRepresentation { Text(full) }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
        }
    }

    private var status: (String, String)? {
        guard model.phase == .ready, let settings = model.settings else { return nil }
        let stage = model.work?.stage
        return (
            Formatting.footerLine(stage: stage, due: model.nextDue, paused: settings.paused, cadence: settings.cadence, armed: model.scheduleArmed),
            Formatting.footerShortLine(stage: stage, due: model.nextDue, paused: settings.paused, cadence: settings.cadence, armed: model.scheduleArmed)
        )
    }
}

/// A mood's menu, the same wherever a mood is right-clicked (the sidebar, the mood list, the summary's cards): Use,
/// Duplicate, Rename…, Delete… (disabled for the last mood).
struct MoodMenuItems: View {
    @Environment(AppModel.self) private var model
    let mood: Mood

    var body: some View {
        Button("Use") { model.useMood(mood.id) }
            .disabled(mood.active)
        Button("Duplicate") { model.duplicateMood(mood.id) }
        Button("Rename…") { model.startRenaming(mood.id) }
        Divider()
        Button("Delete…") { model.moodToDelete = mood.id }
            .disabled(model.moods.count <= 1)
    }
}

/// Deleting a mood asks first, and a mood change that failed is said in an alert. Hosted by the window, so the
/// sidebar's menu works whatever section is showing.
private struct MoodDialogs: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                "Delete “\(model.mood(model.moodToDelete)?.name ?? "")”?",
                isPresented: Binding(get: { model.moodToDelete != nil }, set: { if !$0 { model.moodToDelete = nil } }),
                presenting: model.moodToDelete
            ) { id in
                Button("Delete Mood", role: .destructive) { model.deleteMood(id) }
            } message: { id in
                Text(model.mood(id)?.active == true
                     ? "Its keywords are deleted, and the next mood becomes current. Wallpapers made in it stay in History."
                     : "Its keywords are deleted. Wallpapers made in it stay in History.")
            }
            // Failures that aren't about what was typed (moving, removing, a weight, switching): rare, so an alert.
            .alert("Couldn't Change Moods", isPresented: Binding(
                get: { model.keywordError != nil },
                set: { if !$0 { model.keywordError = nil } }
            )) {
                Button("OK") { model.keywordError = nil }
            } message: {
                Text(model.keywordError ?? "")
            }
    }
}

/// A wallpaper's Delete… (asked first), Show Original and Echoes (a sheet) and what an action couldn't do (an alert),
/// for every place that offers a wallpaper's actions (History, the Gallery, a mood's wallpapers, the summary).
private struct WallpaperDialogs: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        @Bindable var model = model
        content
            .sheet(item: $model.lineageOf) { generation in
                LineageSheet(generation: generation)
            }
            .confirmationDialog(
                "Delete “\(model.wallpaperToDelete?.concept.title ?? "")”?",
                isPresented: Binding(get: { model.wallpaperToDelete != nil }, set: { if !$0 { model.wallpaperToDelete = nil } }),
                presenting: model.wallpaperToDelete
            ) { generation in
                Button("Delete", role: .destructive) { model.deleteConfirmed(generation) }
            } message: { _ in
                Text("Its image is deleted and AutoPaper forgets it, so a similar idea may come back. Echoes of it stay.")
            }
            .alert("Couldn't Do That", isPresented: Binding(
                get: { model.wallpaperProblem != nil },
                set: { if !$0 { model.wallpaperProblem = nil } }
            )) {
                Button("OK") { model.wallpaperProblem = nil }
            } message: {
                Text(model.wallpaperProblem ?? "")
            }
    }
}

/// The empty Now's call to action: New Wallpaper Now, or the stage (with a ring and time left when the engine knows
/// them) while one is being made. Stopping is the toolbar's Stop (Esc), the one control for it.
struct MakeControls: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let work = model.work {
            HStack(spacing: 8) {
                WorkIndicator(work: work)
                VStack(alignment: .leading, spacing: 1) {
                    Text(work.stage)
                        .accessibilityAddTraits(.updatesFrequently)
                    if let timeLeft = work.timeLeft {
                        Text(timeLeft)
                            .font(.callout)
                            .monospacedDigit()
                            .accessibilityHidden(true)
                    }
                }
                // Whole words, never "About 15 seco…" (the empty state's actions are laid out narrow).
                .fixedSize()
            }
        } else {
            Button {
                model.newWallpaperNow()
            } label: {
                Label("New Wallpaper Now", systemImage: "sparkles")
            }
            .buttonStyle(.borderedProminent)
            .disabled(model.phase != .ready)
            .help("Make a new wallpaper now (⌘R)")
        }
    }
}

/// Now's toolbar button, like Safari's reload/stop: New Wallpaper Now (⌘R), which becomes Stop (Esc) while a
/// wallpaper is being made. One button that changes, so the keyboard focus stays on it. A mood's detail has it too
/// (`mood`): the same for the current mood; any other is made current first (Use This Mood and Make a New Wallpaper).
/// Only one of them is ever on screen, so ⌘R and Esc each do one thing. Esc is left to a text field while the keyboard
/// is in one (a mood's name or keywords): there it puts the text back, and never stops a wallpaper (tried in the test
/// VM, 2026-10-06: the button's Esc came first).
struct MakeOrStopButton: View {
    @Environment(AppModel.self) private var model
    /// The mood whose detail shows the button; nil in Now.
    var mood: Mood?
    @State private var editingText = false

    var body: some View {
        let otherMood = mood.map { !$0.active } ?? false
        let state = MakeOrStop(ready: model.phase == .ready, working: model.isWorking, cancellable: model.work?.cancellable ?? false,
                               otherMood: otherMood)
        let stopping = if case .stop = state { true } else { false }
        Button {
            if stopping {
                model.cancel()
            } else if let mood, otherMood {
                model.newWallpaper(from: mood.id)
            } else {
                model.newWallpaperNow()
            }
        } label: {
            Label(state.title, systemImage: state.symbol)
                .contentTransition(.symbolEffect(.replace))
        }
        .help(state.help)
        .keyboardShortcut(stopping ? (editingText ? nil : .cancelAction) : KeyboardShortcut("r", modifiers: .command))
        .disabled(!state.isEnabled)
        .background(TextEditingReader(editing: $editingText))
    }
}

/// Whether the window's keyboard focus is in a text field (its field editor, or another text view, is the first
/// responder), read from AppKit: SwiftUI has no window-wide answer, and the fields include AppKit ones.
private struct TextEditingReader: NSViewRepresentable {
    @Binding var editing: Bool

    func makeNSView(context: Context) -> Probe { Probe() }

    func updateNSView(_ view: Probe, context: Context) {
        view.onChange = { editing = $0 }
    }

    final class Probe: NSView {
        var onChange: ((Bool) -> Void)?
        private var observation: NSKeyValueObservation?

        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observation = window?.observe(\.firstResponder, options: [.initial, .new]) { [weak self] window, _ in
                MainActor.assumeIsolated {
                    let editing = window.firstResponder is NSText
                    // Not while SwiftUI is updating the view that changed the focus.
                    DispatchQueue.main.async { self?.onChange?(editing) }
                }
            }
        }
    }
}

/// Text that stays in the background of a view: the primary colour, a little lighter (still 5:1 or more on the
/// window's backgrounds, unlike the system's secondary label colour at small sizes), and fully primary with Increase
/// Contrast.
struct QuietText: ViewModifier {
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        content.foregroundStyle(contrast == .increased ? Color.primary : Color.primary.opacity(0.7))
    }
}

extension View {
    /// See `QuietText`.
    func quietText() -> some View {
        modifier(QuietText())
    }
}

/// Lays its pieces out left to right, starting a new line when the next one doesn't fit (the welcome's suggested
/// keywords, a sentence with a link in it, a chart's legend). A piece wider than a line wraps within it. Takes the
/// width it's offered.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    var lineSpacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let frames = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let width = frames.map(\.maxX).max() ?? 0
        let height = frames.map(\.maxY).max() ?? 0
        return CGSize(width: proposal.width.map { $0.isFinite ? $0 : width } ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (subview, frame) in zip(subviews, arrange(width: bounds.width, subviews: subviews)) {
            subview.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                          proposal: ProposedViewSize(width: frame.width, height: frame.height))
        }
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [CGRect] {
        var frames: [CGRect] = []
        var x: CGFloat = 0, y: CGFloat = 0, lineHeight: CGFloat = 0
        for subview in subviews {
            let ideal = subview.sizeThatFits(.unspecified)
            let fitted = ideal.width > width ? subview.sizeThatFits(ProposedViewSize(width: width, height: nil)) : ideal
            if x > 0, x + fitted.width > width {
                x = 0
                y += lineHeight + lineSpacing
                lineHeight = 0
            }
            frames.append(CGRect(x: x, y: y, width: min(fitted.width, width), height: fitted.height))
            x += fitted.width + spacing
            lineHeight = max(lineHeight, fitted.height)
        }
        return frames
    }
}
