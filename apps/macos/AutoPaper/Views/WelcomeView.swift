import AutopaperCore
import SwiftUI

/// The first run (spec §4): "Tell your computer what you'd like to see." Three steps — a few keywords, who writes
/// and paints, a key if one is needed — then Make My First Wallpaper. Skippable with Not Now. The examples are
/// shown, never saved.
struct WelcomeView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismissWindow) private var dismissWindow
    @Environment(\.openWindow) private var openWindow
    @State private var keywords: [String] = []
    @State private var draft = ""
    @State private var start: AppModel.Start = .openAI
    /// A key has been entered for the chosen provider (OpenAI and Gemini need one).
    @State private var keyEntered: [AppModel.Start: Bool] = [:]
    /// Keywords the engine didn't take when Make My First Wallpaper was chosen, with why (the rest were saved).
    @State private var refusals: [AppModel.KeywordRefusal] = []
    /// Some keywords were saved when others were refused (they leave the list here).
    @State private var savedOthers = false
    /// Make My First Wallpaper is saving.
    @State private var finishing = false
    /// Why the choices couldn't be saved (rare: the library couldn't be written).
    @State private var problem: String?
    @FocusState private var keywordFocused: Bool
    /// The steps' full height, measured, so the window is as tall as they are (up to the screen; past that they
    /// scroll and the buttons stay in view).
    @State private var stepsHeight: CGFloat = 0

    private static let examples = "rain, lighthouse, autumn, night, watercolour"

    var body: some View {
        // Sized like a Settings pane (`SettingsPane`): as tall as the steps while they fit on the screen (no
        // scrolling, the window fits its content), and scrolling inside a capped height only when they don't.
        let limit = Self.maxStepsHeight
        let overflowing = stepsHeight > limit
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                steps
                    .padding([.horizontal, .top], 28)
                    .padding(.bottom, 16)
            }
            .onScrollGeometryChange(for: CGFloat.self) { geometry in
                geometry.contentSize.height + geometry.contentInsets.top + geometry.contentInsets.bottom
            } action: { _, height in
                stepsHeight = height
            }
            .scrollDisabled(!overflowing)
            .fixedSize(horizontal: false, vertical: !overflowing)
            .frame(height: overflowing ? limit : nil)
            buttons
                .padding([.horizontal, .bottom], 28)
                .padding(.top, 12)
        }
        .frame(width: 560)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { keywordFocused = true }
    }

    /// The tallest the steps may be: the screen's visible height less the title bar and the buttons (a 1280 × 800
    /// display with Local chosen would otherwise push Make My First Wallpaper off the screen).
    private static var maxStepsHeight: CGFloat {
        max(320, (NSScreen.main?.visibleFrame.height ?? 800) - 150)
    }

    private var stepTitle: String {
        switch start {
        case .demo: "Nothing else to add"
        case .local: "Set up Ollama and ComfyUI"
        case .openAI, .gemini: "Add your key"
        }
    }

    private var steps: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(alignment: .center, spacing: 16) {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 64, height: 64)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Welcome to AutoPaper")
                        .font(.largeTitle.weight(.semibold))
                        .accessibilityAddTraits(.isHeader)
                    Text("Tell your computer what you'd like to see.")
                        .font(.title3)
                }
            }

            step(1, "Add a few keywords") {
                HStack(spacing: 8) {
                    TextField("Keyword", text: $draft, prompt: Text("For example: \(Self.examples)"))
                        .textFieldStyle(.roundedBorder)
                        .focused($keywordFocused)
                        .onSubmit(addDraft)
                        .accessibilityLabel("Keyword")
                        .accessibilityHint("Return adds it. For example: \(Self.examples).")
                    Button("Add", action: addDraft)
                        .disabled(KeywordText.normalised(draft).isEmpty)
                }
                if keywords.isEmpty {
                    Text("Words for what you'd like to see: places, weather, moods, colours. You can mark them Must, Maybe or Avoid later.")
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    FlowRow(items: keywords) { keyword in
                        Button {
                            keywords.removeAll { $0 == keyword }
                            refusals.removeAll { $0.keyword == keyword }
                        } label: {
                            Label(keyword, systemImage: "xmark.circle.fill")
                                .labelStyle(TrailingIconLabelStyle())
                        }
                        .buttonStyle(.bordered)
                        .help("Remove \(keyword)")
                        .accessibilityLabel("\(keyword), remove")
                    }
                }
                ForEach(refusals, id: \.keyword) { refusal in
                    // Next to the keywords, where the mistake is; also announced.
                    Label("“\(refusal.keyword)” wasn't added. \(refusal.reason) Remove it to continue.", systemImage: "exclamationmark.triangle")
                        .fixedSize(horizontal: false, vertical: true)
                }
                if savedOthers && !refusals.isEmpty {
                    Text("Your other keywords are saved.")
                }
            }

            step(2, "Choose who writes and paints") {
                Picker("Who writes and paints", selection: $start) {
                    ForEach(AppModel.Start.allCases) { choice in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(choice.title)
                            Text(choice.detail).font(.callout)
                        }
                        .tag(choice)
                    }
                }
                .pickerStyle(.radioGroup)
                .labelsHidden()
            }

            step(3, stepTitle) {
                if needsKey {
                    // Why Make My First Wallpaper is dimmed, in words (a disabled button's help tag isn't reliable).
                    Label("Paste your key to continue.", systemImage: "key")
                }
                switch start {
                case .openAI:
                    KeyField(account: ProviderKind.openAi.keyAccount ?? "openai.api_key", label: "OpenAI API key", spokenLabel: "OpenAI API key") {
                        keyEntered[.openAI] = !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }
                    Link("Get a key", destination: URL(string: "https://platform.openai.com/api-keys")!)
                        .accessibilityLabel("Get a key for OpenAI")
                case .gemini:
                    KeyField(account: ProviderKind.google.keyAccount ?? "google.api_key", label: "Gemini API key", spokenLabel: "Google Gemini API key") {
                        keyEntered[.gemini] = !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    }
                    Link("Get a key", destination: URL(string: "https://aistudio.google.com/apikey")!)
                        .accessibilityLabel("Get a key for Google Gemini")
                    Text("A free Gemini key may let Google use what you send to improve its products; a paid project doesn't.")
                        .fixedSize(horizontal: false, vertical: true)
                case .local:
                    Text("Ollama and ComfyUI need to be running on this Mac, each with a model downloaded.")
                        .fixedSize(horizontal: false, vertical: true)
                    GroupBox {
                        VStack(alignment: .leading, spacing: 10) {
                            BrewBrowserHelp(kind: .ollama)
                            Divider()
                            BrewBrowserHelp(kind: .comfyUi)
                        }
                        .padding(6)
                    }
                case .demo:
                    Text("The Demo needs no key and costs nothing. Choose real providers any time in Settings → Providers.")
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            if let problem {
                // Next to the button that was chosen; also announced.
                Label(problem, systemImage: "exclamationmark.triangle")
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var buttons: some View {
            HStack {
                Button("Not Now") {
                    model.skipWelcome()
                    dismissWindow(id: WindowID.welcome)
                }
                .keyboardShortcut(.cancelAction)
                Spacer()
                Button("Make My First Wallpaper") {
                    addDraft()
                    let chosen = keywords, choice = start
                    finishing = true
                    problem = nil
                    Task {
                        let outcome = await model.finishWelcome(keywords: chosen, start: choice)
                        finishing = false
                        switch outcome {
                        case .started:
                            model.section = .now
                            openWindow(id: WindowID.main)
                            dismissWindow(id: WindowID.welcome)
                        case .refused(let refused):
                            // The others are saved; only the refused ones stay here, with why.
                            savedOthers = savedOthers || chosen.count > refused.count
                            keywords = refused.map(\.keyword)
                            refusals = refused
                            model.announce(refused.map { "“\($0.keyword)” wasn't added. \($0.reason)" }.joined(separator: " "))
                            keywordFocused = true
                        case .failed(let sentence):
                            problem = sentence
                            model.announce(sentence)
                        }
                    }
                }
                // Return belongs to the keyword field while it has focus ("Return adds it"), so it never finishes the
                // welcome with a keyword half typed; from anywhere else Return chooses this button.
                .keyboardShortcut(keywordFocused ? nil : .defaultAction)
                .buttonStyle(.borderedProminent)
                .disabled(model.phase != .ready || needsKey || finishing || !refusals.isEmpty)
                .help(needsKey ? "Add your key first"
                      : !refusals.isEmpty ? "Remove the keywords that weren't added first"
                      : "Saves your keywords and makes a wallpaper now")
            }
    }

    /// The chosen provider needs a key and none has been entered yet.
    private var needsKey: Bool {
        start.keyAccount != nil && keyEntered[start] != true
    }

    private func step<Content: View>(_ number: Int, _ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("\(number). \(title)")
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    private func addDraft() {
        let text = KeywordText.normalised(draft)
        guard !text.isEmpty else { return }
        if !keywords.contains(where: { $0.caseInsensitiveCompare(text) == .orderedSame }) {
            keywords.append(text)
        }
        draft = ""
    }
}

/// Lays out items left to right, wrapping onto new lines.
private struct FlowRow<Item: Hashable, ItemView: View>: View {
    let items: [Item]
    @ViewBuilder let content: (Item) -> ItemView

    var body: some View {
        FlowLayout(spacing: 6, lineSpacing: 6) {
            ForEach(items, id: \.self) { content($0) }
        }
    }
}

private struct TrailingIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.title
            configuration.icon
        }
    }
}
