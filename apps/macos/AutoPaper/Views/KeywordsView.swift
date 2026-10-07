import AutopaperCore
import SwiftUI

/// A mood's keywords, in its detail: "Add a keyword", then the list (each editable inline, with Must / Maybe /
/// Avoid and Remove; reordered by dragging, by Edit ▸ Move Keyword Up / Down, or the row's own actions). Works on
/// any mood, current or not (the engine's keyword methods by id, and `add_mood_keyword`).
///
/// Only keywords are in the list: its rows are selected with the arrow keys, and a control inside a row is reached
/// only on the selected row, so Return renames the selected keyword, Delete removes it, and Edit ▸ Keyword Weight
/// (⌃⌘1–3) sets its weight (inside a list the arrow keys select rows rather than segments). Removing, a weight and a
/// move can be undone (Edit ▸ Undo).
struct MoodKeywords: View {
    @Environment(AppModel.self) private var model
    @Environment(\.undoManager) private var undoManager
    let mood: Mood
    @State private var newKeyword = ""
    /// Why the last keyword typed in "Add a keyword" wasn't added; cleared on the next edit.
    @State private var addProblem: String?
    @State private var selection: Keyword.ID?
    @FocusState private var addFieldFocused: Bool
    /// The keyword whose text field is being edited (Return on a selected row starts it).
    @FocusState private var editing: Keyword.ID?
    /// The list itself has the focus (back on the row after Esc ends a rename).
    @FocusState private var listFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            addBar
            Divider()
            keywordList
            // Under the list rather than its last row, so it's always whole however far the list is scrolled. The
            // keyboard's ways to move and weigh keywords are in the Edit menu (with their shortcuts) and the list's
            // VoiceOver hint. No `fixedSize`, as for the add bar's problem line.
            Text("Must: always in the scene. Maybe: some of the time. Avoid: never. Drag to reorder.")
                .font(.callout)
                .quietText()
                .frame(maxWidth: .infinity, alignment: .leading)
                .layoutPriority(1)
                .padding(.horizontal, 20)
                .padding(.vertical, 8)
        }
    }

    private var addBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                TextField("Add a keyword", text: $newKeyword)
                    .textFieldStyle(.roundedBorder)
                    .focused($addFieldFocused)
                    .onSubmit(add)
                    .onChange(of: newKeyword) { addProblem = nil }
                    .accessibilityLabel("Add a keyword to \(mood.name)")
                    .accessibilityHint(addProblem ?? "Return adds it as a Must keyword.")
                Button("Add", action: add)
                    .disabled(KeywordText.normalised(newKeyword).isEmpty || model.phase != .ready)
            }
            if let addProblem {
                // Inline, next to the field (HIG: validation where the mistake is); also announced by the model.
                // No `fixedSize` here, outside the list's scrolling: the split view would measure it at almost no
                // width and make the window's content taller than the window.
                Label(addProblem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .layoutPriority(1)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
    }

    private var keywordList: some View {
        ScrollViewReader { proxy in
            List(selection: $selection) {
                Section {
                    if mood.keywords.isEmpty {
                        Text("No keywords yet. Add a few words for what you'd like to see, like “rain”, “lighthouse” or “autumn”.")
                            .fixedSize(horizontal: false, vertical: true)
                            .selectionDisabled()
                    }
                    ForEach(mood.keywords) { keyword in
                        KeywordRow(keyword: keyword, moodID: mood.id, index: index(of: keyword), count: mood.keywords.count, editing: $editing) {
                            // Esc: the rename ends and the keyboard is back on the row, as in Finder.
                            selection = keyword.id
                            editing = nil
                            listFocused = true
                        }
                            .tag(keyword.id)
                            .id(keyword.id)
                    }
                    .onMove(perform: move)
                    if mood.active && model.narrow {
                        // `keywords_are_narrow` looks at the current mood's last wallpapers.
                        Label(Sentences.narrowKeywords, systemImage: "lightbulb")
                            .fixedSize(horizontal: false, vertical: true)
                            .selectionDisabled()
                    }
                } header: {
                    Text("Keywords")
                }
            }
            .listStyle(.inset)
            .focused($listFocused)
            .frame(minHeight: 120)
            // Named for VoiceOver, as the mood list ("Moods") and History's grid ("Wallpapers") are.
            .accessibilityLabel("Keywords of \(mood.name)")
            .accessibilityHint("Return renames the selected keyword; Delete removes it. Edit ▸ Move Keyword Up or Down (⌥⌘↑ ⌥⌘↓) moves it, and Edit ▸ Keyword Weight (⌃⌘1–3) sets its weight.")
            .onChange(of: selection) { _, id in
                if let id { withAnimation(nil) { proxy.scrollTo(id) } }
            }
            // Return on the selected row edits its text (Finder's rename); while editing, Return saves.
            .onKeyPress(.return) {
                guard editing == nil, let selection else { return .ignored }
                editing = selection
                return .handled
            }
            // The Mac's Delete key and Edit ▸ Delete remove the selected keyword (said, and easily added back).
            .onDeleteCommand {
                if let keyword = selectedKeyword { model.removeKeyword(keyword, from: mood.id, undoManager: undoManager) }
            }
            .focusedValue(\.reorderable, reorderable)
            .focusedValue(\.selectedKeyword, selectedKeywordValue)
            .onAppear(perform: selectIfAsked)
            .onChange(of: model.keywordToSelect) { selectIfAsked() }
        }
    }

    /// A keyword problem's link (`AppModel.showMood`) asks for its keyword: selected, scrolled to and focused, so
    /// Return rewords it, ⌃⌘1–3 weighs it and Delete removes it.
    private func selectIfAsked() {
        guard let text = model.keywordToSelect, model.moodSelection == mood.id else { return }
        model.keywordToSelect = nil
        guard let keyword = KeywordText.duplicate(of: text, in: mood.keywords) else { return }
        selection = keyword.id
        listFocused = true
    }

    private func add() {
        let text = KeywordText.normalised(newKeyword)
        guard !text.isEmpty else { return }
        let moodID = mood.id
        Task {
            guard let result = await model.addKeyword(text, to: moodID) else { return }
            switch result {
            case .added:
                newKeyword = ""
            case .duplicate(let existing):
                // Highlighted instead of added twice.
                newKeyword = ""
                selection = existing.id
            case .refused(let sentence):
                // What was typed stays, to fix.
                addProblem = sentence
            }
            addFieldFocused = true
        }
    }

    private func index(of keyword: Keyword) -> Int {
        mood.keywords.firstIndex(of: keyword) ?? 0
    }

    private func move(from offsets: IndexSet, to destination: Int) {
        guard let from = offsets.first else { return }
        model.moveKeyword(mood.keywords[from], in: mood.id, to: Reorder.position(from: from, droppedAt: destination), undoManager: undoManager)
    }

    private var selectedKeyword: Keyword? {
        mood.keywords.first { $0.id == selection }
    }

    /// The selected row, for Edit ▸ Move Keyword Up / Down (⌥⌘↑ / ⌥⌘↓).
    private var reorderable: ReorderableRow? {
        guard let keyword = selectedKeyword, let index = mood.keywords.firstIndex(of: keyword) else { return nil }
        let moodID = mood.id, undoManager = undoManager
        return ReorderableRow(
            noun: "Keyword",
            canMoveUp: index > 0,
            canMoveDown: index < mood.keywords.count - 1,
            moveUp: { model.moveKeyword(keyword, in: moodID, to: index - 1, undoManager: undoManager) },
            moveDown: { model.moveKeyword(keyword, in: moodID, to: index + 1, undoManager: undoManager) }
        )
    }

    /// The selected row, for Edit ▸ Keyword Weight (⌃⌘1–3).
    private var selectedKeywordValue: SelectedKeyword? {
        guard let keyword = selectedKeyword else { return nil }
        let undoManager = undoManager
        return SelectedKeyword(weight: keyword.weight, setWeight: { model.setWeight(keyword, $0, announce: true, undoManager: undoManager) })
    }
}

/// One keyword: its text (edited in place; Return or leaving the field saves, Esc puts the saved text back), its
/// weight, and Remove. Spoken as "rain, Must"; Move Up, Move Down (where they apply) and the other two weights are
/// its accessibility actions and context menu items.
private struct KeywordRow: View {
    @Environment(AppModel.self) private var model
    @Environment(\.undoManager) private var undoManager
    let keyword: Keyword
    let moodID: Mood.ID
    let index: Int
    let count: Int
    var editing: FocusState<Keyword.ID?>.Binding
    /// Ends a cancelled rename, putting the keyboard back on the row.
    let endRename: () -> Void
    @State private var text = ""
    /// Why the last rename wasn't saved; cleared on the next edit.
    @State private var problem: String?
    /// A rename being saved (Return and leaving the field both commit; the second is the same rename).
    @State private var renaming: String?
    /// The saved text being put back (a refusal, or Esc): not an edit of the person's, so it doesn't clear `problem`.
    @State private var restoring: String?

    private var isEditing: Bool { editing.wrappedValue == keyword.id }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 12) {
                TextField("Keyword", text: $text)
                    .textFieldStyle(.plain)
                    .focused(editing, equals: keyword.id)
                    .onSubmit(commit)
                    .onChange(of: isEditing) { _, nowEditing in
                        if !nowEditing { commit() }
                    }
                    .onChange(of: text) { _, newValue in
                        if newValue == restoring { restoring = nil; return }
                        if isEditing { problem = nil }
                    }
                    // Esc cancels the rename (Finder, Notes): the saved text comes back and nothing is saved.
                    .onExitCommand {
                        put(back: keyword.text)
                        problem = nil
                        endRename()
                    }
                    .accessibilityLabel("Keyword")
                    .accessibilityHint(problem ?? "")
                WeightControl(weight: keyword.weight, label: "Weight of \(keyword.text)") { weight in
                    model.setWeight(keyword, weight, undoManager: undoManager)
                }
                .fixedSize()
                // The control's track is translucent: on the list's own background colour it reads the same in the
                // selected (blue) row as in any other (chosen segment 3.5:1 against the others).
                .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                Button {
                    model.removeKeyword(keyword, from: moodID, undoManager: undoManager)
                } label: {
                    // At least 22 × 22 points to hit (HIG's minimum for macOS controls is 20 × 20).
                    Image(systemName: "minus.circle")
                        .frame(width: 22, height: 22)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .help("Remove")
                .accessibilityLabel("Remove \(keyword.text)")
            }
            if let problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 2)
        .onAppear { text = keyword.text }
        .onChange(of: keyword.text) { _, newValue in
            if !isEditing { text = newValue }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(keyword.text), \(keyword.weight.title)")
        // Only the moves that apply, as the context menu disables the others.
        .accessibilityActions {
            if index > 0 { Button("Move Up") { model.moveKeyword(keyword, in: moodID, to: index - 1, undoManager: undoManager) } }
            if index < count - 1 { Button("Move Down") { model.moveKeyword(keyword, in: moodID, to: index + 1, undoManager: undoManager) } }
            ForEach(KeywordWeight.all.filter { $0 != keyword.weight }, id: \.self) { weight in
                Button("Make \(weight.title)") { model.setWeight(keyword, weight, announce: true, undoManager: undoManager) }
            }
        }
        .contextMenu {
            Picker("Weight", selection: Binding(
                get: { keyword.weight },
                set: { model.setWeight(keyword, $0, announce: true, undoManager: undoManager) }
            )) {
                ForEach(KeywordWeight.all, id: \.self) { Text($0.title).tag($0) }
            }
            .pickerStyle(.inline)
            Divider()
            Button("Move Up") { model.moveKeyword(keyword, in: moodID, to: index - 1, undoManager: undoManager) }
                .disabled(index == 0)
            Button("Move Down") { model.moveKeyword(keyword, in: moodID, to: index + 1, undoManager: undoManager) }
                .disabled(index >= count - 1)
            Divider()
            Button("Rename") { editing.wrappedValue = keyword.id }
            Button("Remove") { model.removeKeyword(keyword, from: moodID, undoManager: undoManager) }
        }
    }

    private func commit() {
        let normalised = KeywordText.normalised(text)
        if normalised.isEmpty {
            put(back: keyword.text)
            return
        }
        guard normalised != keyword.text, renaming != normalised else { return }
        let original = keyword.text
        renaming = normalised
        Task {
            defer { renaming = nil }
            guard let refused = await model.renameKeyword(keyword, to: normalised) else { return }
            // Put back what the engine kept, so what's shown is what's saved (and what VoiceOver says), and say
            // why next to the row, where it stays until the person edits the text again.
            put(back: original)
            problem = "“\(normalised)” wasn't saved. \(refused)"
            model.announce(problem ?? refused)
        }
    }

    /// Shows the saved text again without it counting as an edit.
    private func put(back saved: String) {
        guard text != saved else { return }
        restoring = saved
        text = saved
    }
}

/// A keyword's weight: Must · Maybe · Avoid as the Mac's own segmented control (`NSSegmentedControl`), the same
/// control SwiftUI's segmented picker shows, with one difference: in the selected list row AppKit asks controls to
/// draw for an emphasized (accent-coloured) background, which turns all three segments the same blue (about 1.05:1
/// between the chosen one and the others). This control keeps its usual look there, so the chosen weight can still be
/// seen in the row the person is working on (WCAG 1.4.11, 1.4.1), on the list's background colour. VoiceOver hears it as SwiftUI's picker is heard:
/// "Weight of rain", its segments Must, Maybe and Avoid, the chosen one selected.
private struct WeightControl: NSViewRepresentable {
    let weight: KeywordWeight
    let label: String
    let choose: (KeywordWeight) -> Void

    func makeNSView(context: Context) -> NSSegmentedControl {
        let control = SteadySegmentedControl()
        control.segmentCount = KeywordWeight.all.count
        control.trackingMode = .selectOne
        for (index, weight) in KeywordWeight.all.enumerated() {
            control.setLabel(weight.title, forSegment: index)
            control.setToolTip(weight.explanation, forSegment: index)
        }
        control.target = context.coordinator
        control.action = #selector(Coordinator.changed(_:))
        return control
    }

    func updateNSView(_ control: NSSegmentedControl, context: Context) {
        context.coordinator.choose = choose
        control.selectedSegment = KeywordWeight.all.firstIndex(of: weight) ?? -1
        control.setAccessibilityLabel(label)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(choose: choose)
    }

    final class Coordinator: NSObject {
        var choose: (KeywordWeight) -> Void

        init(choose: @escaping (KeywordWeight) -> Void) {
            self.choose = choose
        }

        @MainActor @objc func changed(_ sender: NSSegmentedControl) {
            guard KeywordWeight.all.indices.contains(sender.selectedSegment) else { return }
            choose(KeywordWeight.all[sender.selectedSegment])
        }
    }
}

/// A segmented control that draws with its normal background style even in a selected (emphasized) table row.
private final class SteadySegmentedControl: NSSegmentedControl {
    override class var cellClass: AnyClass? {
        get { SteadySegmentedCell.self }
        set {}
    }
}

private final class SteadySegmentedCell: NSSegmentedCell {
    override var backgroundStyle: NSView.BackgroundStyle {
        get { .normal }
        set {}
    }
}

/// A mood's Surprise: 0–100 from Faithful to Wild, with the composer's band in words below. Spoken as "35 percent,
/// fresh", followed by the band's explanation. Saved to that mood shortly after it stops changing (dragging, arrow
/// keys or VoiceOver, which steps by 5).
struct SurpriseControl: View {
    @Environment(AppModel.self) private var model
    let mood: Mood
    @State private var percent: Double = 35
    @State private var loaded = false

    var body: some View {
        let value = Int(percent.rounded())
        let band = SurpriseBand(percent: value)
        VStack(alignment: .leading, spacing: 8) {
            VStack(spacing: 2) {
                // No `step`: a stepped slider draws a tick for every value. Whole numbers come from the binding.
                Slider(value: Binding(get: { percent }, set: { percent = $0.rounded() }), in: 0...100) {
                    Text("Surprise")
                }
                .labelsHidden()
                .accessibilityValue(SurpriseBand.spokenValue(percent: value))
                .accessibilityAdjustableAction { direction in
                    switch direction {
                    case .increment: percent = min(100, (percent / 5).rounded(.down) * 5 + 5)
                    case .decrement: percent = max(0, (percent / 5).rounded(.up) * 5 - 5)
                    @unknown default: break
                    }
                }
                // The ends' names, drawn here rather than as the slider's value labels, which macOS draws in its
                // own faint secondary style whatever is asked (3.6–3.9:1 in Light mode): these are primary text.
                // The slider's value already says the band ("35 percent, fresh"), so VoiceOver doesn't hear them
                // again.
                HStack {
                    Text("Faithful")
                    Spacer()
                    Text("Wild")
                }
                .font(.callout)
                .foregroundStyle(.primary)
                .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: 2) {
                // The slider's value says this already ("35 percent, fresh").
                Text("\(value) — \(band.title)")
                    .fontWeight(.medium)
                    .accessibilityHidden(true)
                // What the band does, read after the slider. (Not the slider's hint or help: on a macOS slider
                // SwiftUI puts either in place of its role, so VoiceOver would no longer say "slider".)
                Text(band.explanation)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 4)
        .onAppear(perform: load)
        .onChange(of: mood.surprise) { load() }
        .task(id: percent) {
            guard loaded else { return }
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            let value = Float(percent.rounded()) / 100
            if abs(mood.surprise - value) < 0.001 { return }
            model.setSurprise(value, of: mood.id)
        }
    }

    private func load() {
        let value = Double((mood.surprise * 100).rounded())
        if abs(value - percent) >= 1 || !loaded { percent = value }
        loaded = true
    }
}
