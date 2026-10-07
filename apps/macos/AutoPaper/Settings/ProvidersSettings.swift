import AutopaperCore
import SwiftUI
import UniformTypeIdentifiers

/// Providers: who writes ideas and who paints. Each group has the provider, its server address (local and
/// OpenAI-compatible kinds), the model (listed by the provider, with Refresh), Test with the result in words, and
/// for Ollama and ComfyUI the Brew Browser help ("Setting up local models", systemPatterns.md). Painting adds
/// quality (for OpenAI and Gemini, the providers that use it) and, for ComfyUI, the workflow: AutoPaper's (then the
/// Model menu lists the installed models it has a workflow for) or the person's own file (then its loader decides
/// the model, said on a read-only line).
struct ProvidersSettings: View {
    var body: some View {
        SettingsPane { settings in
            ProviderSection(job: .concepts, selection: settings.textProvider)
            ProviderSection(job: .images, selection: settings.imageProvider, quality: settings.imageQuality, workflow: settings.comfyuiWorkflow)
            if settings.textProvider.kind == .google || settings.imageProvider.kind == .google {
                Section {
                    Label("A free Gemini key may let Google use what you send to improve its products; a paid project doesn't.", systemImage: "hand.raised")
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

private struct ProviderSection: View {
    @Environment(AppModel.self) private var model
    let job: ProviderJob
    let selection: ProviderSelection
    var quality: ImageQuality = .high
    var workflow: String?

    @State private var models: [ModelInfo] = []
    @State private var loadingModels = false
    @State private var testing = false
    /// The one thing this section says about the provider (spec 6a: a problem appears once per view): the outcome
    /// of the last model listing or Test, whichever came last. A missing or refused key is a link to the key.
    @State private var status: Status?
    /// How long each model takes here (seconds, by model id; "" = the default), from the engine's timings.
    @State private var estimates = AppModel.Estimates()
    /// What the model list was last loaded for: a change the person made (provider, address, key, workflow) gets its
    /// outcome announced, the pane appearing doesn't.
    @State private var loadedFor: ModelsKey?
    /// The file panel for "Your Own…" / "Choose Another…" (a sheet on the Settings window).
    @State private var choosingFile = false
    @State private var address = ""
    @FocusState private var addressFocused: Bool
    /// Why the last address typed wasn't saved, shown under the field; cleared on the next edit.
    @State private var addressProblem: String?
    /// The address the engine just refused, so leaving the field doesn't send it again.
    @State private var refusedAddress: String?
    @State private var workflowProblem: String?
    /// The last list came from the provider (not an error): a saved model missing from it isn't available.
    @State private var listed = false
    /// Switched back to AutoPaper's workflow: the saved model stays only if the next list still has it.
    @State private var checkSavedModel = false
    /// The person's own ComfyUI workflow, read: the model it loads, or why it can't be used.
    @State private var ownReading: OwnWorkflow.Reading?
    /// The person's own workflow file, remembered while AutoPaper's is in use (the engine holds only the one used).
    @AppStorage(WorkflowChoice.Keys.text) private var ownText = ""
    @AppStorage(WorkflowChoice.Keys.name) private var ownName = ""

    private enum Status: Equatable {
        /// Test passed ("It works: 6 models available.").
        case works(String)
        /// Listing models or Test failed, in words; `unavailable`: the server isn't answering at its address (the
        /// Brew Browser help then comes first).
        case problem(String, unavailable: Bool)
        /// A missing or refused key: one link to its field in Accounts.
        case key(KeyProblem)
    }

    private var keyProblem: KeyProblem? {
        if case .key(let problem) = status { return problem }
        return nil
    }

    /// A missing key: nothing can be listed or tested until it's added (the link says so), so Test and Refresh wait.
    private var waitingForKey: Bool {
        keyProblem.map { !$0.refused } ?? false
    }

    private var keyPath: WritableKeyPath<EngineSettings, ProviderSelection> {
        ProviderChoice.keyPath(job)
    }

    /// "Writing" or "Painting": names the controls for VoiceOver, where the section header isn't heard.
    private var jobName: String {
        job == .concepts ? "Writing" : "Painting"
    }

    /// The address this selection talks to, for "isn't running at …".
    private var addressInUse: [ProviderKind: String] {
        guard let address = selection.baseUrl ?? selection.kind.defaultAddress else { return [:] }
        return [selection.kind: address]
    }

    private var kinds: [ProviderKind] {
        job == .concepts ? ProviderKind.writers : ProviderKind.painters
    }

    /// ComfyUI painting with the person's own workflow file.
    private var usesOwnWorkflow: Bool {
        selection.kind == .comfyUi && workflow != nil
    }

    var body: some View {
        Section {
            LabeledContent("Provider") {
                Picker("\(jobName) provider", selection: Binding(get: { selection.kind }, set: choose)) {
                    ForEach(kinds, id: \.self) { kind in
                        Text(pickerTitle(kind)).tag(kind)
                    }
                }
                .labelsHidden()
                .fixedSize()
            }
            if selection.kind.takesAddress {
                // The placeholder is the address used when the field is blank (the core's `default_base_url`); an
                // OpenAI-compatible server has none, so its field starts empty.
                TextField("Server address", text: $address, prompt: selection.kind.defaultAddress.map { Text($0) })
                    .focused($addressFocused)
                    .onSubmit(saveAddress)
                    .onChange(of: addressFocused) { _, focused in if !focused { leaveAddress() } }
                    .onChange(of: address) { if addressFocused { addressProblem = nil; refusedAddress = nil } }
                    .accessibilityLabel("\(jobName) server address")
                    .accessibilityHint(addressProblem ?? "")
                    // An address link from the Now view ("Check the server address") lands here.
                    .onAppear(perform: takeFocusIfAsked)
                    .onChange(of: model.providersFocus) { takeFocusIfAsked() }
                if let addressProblem {
                    // Where the mistake is (HIG), rather than an alert; also announced.
                    Label(addressProblem, systemImage: "exclamationmark.triangle")
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            // ComfyUI: the workflow first, since it decides the model (a menu for AutoPaper's, a line for your own).
            if selection.kind == .comfyUi {
                workflowRow
            }
            modelRow
            // Only OpenAI and Gemini paint differently by quality; ComfyUI always paints at the largest size its model
            // supports, and Demo and OpenAI-compatible servers aren't sent it.
            if job == .images && (selection.kind == .openAi || selection.kind == .google) {
                Picker("Painting quality", selection: model.setting(\.imageQuality, .high)) {
                    Text(ImageQuality.standard.title).tag(ImageQuality.standard)
                    Text(ImageQuality.high.title).tag(ImageQuality.high)
                }
                .pickerStyle(.segmented)
            }
            LabeledContent {
                HStack(spacing: 8) {
                    if !usesOwnWorkflow {
                        Button("Refresh Models") { Task { await loadModels() } }
                            // Nothing to list until there's a key; the key link says so.
                            .disabled(loadingModels || waitingForKey)
                            .help("Ask the provider for its models again")
                            // Starts with the visible title (WCAG 2.5.3), as "Test writing provider" does.
                            .accessibilityLabel("Refresh Models for \(jobName.lowercased())")
                    }
                    Button("Test") { Task { await test() } }
                        // Nothing to test until there's a key; the key link says so.
                        .disabled(testing || waitingForKey)
                        .accessibilityLabel("Test \(jobName.lowercased()) provider")
                }
                .fixedSize()
            } label: {
                Text("Connection")
                if testing {
                    Text("Checking…")
                }
            }
            if !testing && !loadingModels, let status {
                // Said once (spec 6a), in a row of its own in primary text: something to read and act on, not the
                // label's secondary subtitle (3.9:1 in Light mode).
                switch status {
                case .key(let problem):
                    KeyLink(problem: problem)
                case .works(let text):
                    Label(text, systemImage: "checkmark.circle")
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                case .problem(let text, _):
                    Label(text, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if selection.kind == .ollama || selection.kind == .comfyUi {
                BrewBrowserHelp(kind: selection.kind, emphasised: isUnavailable)
            }
        } header: {
            Text(job == .concepts ? "Writing ideas" : "Painting")
        } footer: {
            Text(footer)
                .fixedSize(horizontal: false, vertical: true)
        }
        .onAppear { address = selection.baseUrl ?? "" }
        .onChange(of: selection.baseUrl) { _, url in if !addressFocused { address = url ?? "" } }
        .onChange(of: selection.kind) {
            status = nil
            addressProblem = nil
            refusedAddress = nil
            workflowProblem = nil
            readOwnWorkflow()
        }
        .onChange(of: workflow, initial: true) { readOwnWorkflow() }
        .task(id: ModelsKey(kind: selection.kind, address: selection.baseUrl, workflow: workflow, keys: model.keysChanged)) {
            let key = ModelsKey(kind: selection.kind, address: selection.baseUrl, workflow: workflow, keys: model.keysChanged)
            let changed = loadedFor != nil && loadedFor != key
            loadedFor = key
            status = nil
            await loadModels()
            // A provider, address or key the person just changed: what it means is said, as Test's result is
            // (WCAG 4.1.3), in the link's words for a key.
            if changed, !Task.isCancelled, let said = spokenStatus { model.announce(said) }
        }
        // Timings change after every wallpaper (and the writer's add to a painting's estimate).
        .task(id: EstimatesKey(selection: selection, writer: model.settings?.textProvider, models: models.map(\.id), revision: model.historyRevision)) {
            estimates = await model.estimates(selection, job: job, models: models.map(\.id))
        }
        .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.json], onCompletion: useFile)
        .fileDialogMessage("Choose a ComfyUI workflow saved with Export (API), with {{prompt}}, {{width}}, {{height}} and {{seed}} where AutoPaper fills them in.")
        .fileDialogConfirmationLabel("Use Workflow")
    }

    /// The status line as VoiceOver should hear it when it appears by itself.
    private var spokenStatus: String? {
        switch status {
        case .key(let problem): SettingsProblem(key: problem).spoken
        case .problem(let text, _): text
        case .works(let text): text
        case nil: nil
        }
    }

    /// An address link asked for this section's address field: focus it (once).
    private func takeFocusIfAsked() {
        guard model.providersFocus == job, selection.kind.takesAddress else { return }
        model.providersFocus = nil
        DispatchQueue.main.async { addressFocused = true }
    }

    /// The server isn't answering at its address (listing or Test said so).
    private var isUnavailable: Bool {
        if case .problem(_, true) = status { return true }
        return false
    }

    private var footer: String {
        switch (job, selection.kind) {
        case (.concepts, .demo): "No AI: Demo writes simple ideas from your keywords. Free, for trying AutoPaper."
        case (.images, .demo): "No AI: Demo paints soft gradients. Free, for trying AutoPaper."
        case (.concepts, _): "Writes a scene from your keywords; only your keywords and its own ideas are sent."
        case (.images, .comfyUi):
            workflow == nil
                ? "Paints on your own Mac with your ComfyUI, using AutoPaper's workflow for the model you choose. Free."
                : "Paints on your own Mac with your ComfyUI, using your own workflow and the model it loads. Free."
        case (.images, _): "Paints the scene. Only the written prompt is sent."
        }
    }

    private func pickerTitle(_ kind: ProviderKind) -> String {
        switch kind {
        case .demo: "Demo (no AI, gradients — for trying the app)"
        case .ollama: "Ollama (on this Mac)"
        case .comfyUi: "ComfyUI (on this Mac)"
        default: kind.name
        }
    }

    // MARK: Model

    @ViewBuilder
    private var modelRow: some View {
        if usesOwnWorkflow {
            // The file decides the model: said here, not chosen.
            LabeledContent {
                Text(OwnWorkflow.modelLine(ownReading))
                    .textSelection(.enabled)
            } label: {
                Text("Model")
                if let estimate = estimateLine {
                    Text(estimate)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("\(jobName) model")
            .accessibilityValue([OwnWorkflow.modelLine(ownReading), estimateLine].compactMap { $0 }.joined(separator: ". "))
        } else {
            // A stock picker row, so the menu stays on the label's line with the estimate under the label (Refresh is
            // in the Connection row).
            Picker(selection: Binding(get: { selection.model }, set: setModel)) {
                ForEach(ModelMenu.items(kind: selection.kind, job: job, models: models, saved: selection.model, listed: listed), id: \.tag) { item in
                    // With how long it takes here, once AutoPaper has timed it: "Qwen-Image 2.1 (about 9 minutes)".
                    Text(Formatting.withEstimate(item.title, seconds: estimates.seconds[item.tag])).tag(item.tag)
                }
            } label: {
                Text("Model")
                if loadingModels {
                    Text("Listing models…")
                } else if let estimate = estimateLine {
                    // "About 9 minutes per wallpaper on this Mac": from this Mac's own timings, never a guess.
                    Text(estimate)
                }
            }
            .accessibilityLabel("\(jobName) model")
        }
    }

    /// How long the chosen model takes here, in words; nil until AutoPaper has timed it.
    private var estimateLine: String? {
        Formatting.estimateLine(seconds: estimates.seconds[usesOwnWorkflow ? "" : selection.model], job: job, place: Formatting.Place(selection),
                                includesWriting: estimates.includesWriting)
    }

    private func loadModels() async {
        guard !usesOwnWorkflow else {
            // Its file decides the model; there's no menu to fill.
            models = []
            if case .key = status {} else { status = nil }
            listed = false
            return
        }
        loadingModels = true
        defer { loadingModels = false }
        do {
            models = try await model.listModels(selection, job: job)
            if case .works = status {} else { status = nil }
            listed = true
            if checkSavedModel {
                // Back from the person's own workflow: the model chosen here before stays if it's still there.
                checkSavedModel = false
                if !selection.model.isEmpty, !models.contains(where: { $0.id == selection.model }) { setModel("") }
            }
        } catch {
            models = []
            listed = false
            checkSavedModel = false
            log.error("Listing models failed: \(String(describing: error), privacy: .public)")
            if let problem = KeyProblem(error, selection: selection) {
                // One thing to do, said once: the key link (not a model-list error as well).
                status = .key(problem)
            } else {
                status = .problem("Couldn't list models: " + (Sentences.error(error, retrying: false, inSettings: true, addresses: addressInUse) ?? ""),
                                  unavailable: Self.isUnavailable(error))
            }
        }
    }

    // MARK: Changes

    private func choose(_ kind: ProviderKind) {
        guard kind != selection.kind else { return }
        let path = keyPath
        model.updateSettings { $0[keyPath: path] = ProviderSelection(kind: kind, model: "", baseUrl: nil) }
    }

    private func setModel(_ id: String) {
        let job = job
        model.updateSettings { ProviderChoice.choose(model: id, for: job, in: &$0) }
    }

    /// Saves the address typed (blank = the default). One the engine refuses stays in the field while it has focus,
    /// to fix, with the reason under it; the alert is only for other failures.
    private func saveAddress() {
        let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
        let value: String? = trimmed.isEmpty ? nil : trimmed
        // A blank field (nil: the default) is never refused, so only a typed address can match `refusedAddress`.
        guard value != selection.baseUrl, refusedAddress == nil || value != refusedAddress else { return }
        let path = keyPath
        addressProblem = nil
        model.updateSettings({ $0[keyPath: path].baseUrl = value }) { error in
            guard case .InvalidInput(let reason, _) = error, reason.isAboutAddress else { return false }
            let problem = Sentences.addressRefused(trimmed, reason)
            refusedAddress = value
            addressProblem = problem
            model.announce(problem)
            return true
        }
    }

    /// Leaving the field saves what's in it; an address just refused is put back to the one saved (the reason
    /// stays under the field, quoting what was typed).
    private func leaveAddress() {
        let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
        if let refusedAddress, trimmed == refusedAddress {
            address = selection.baseUrl ?? ""
            self.refusedAddress = nil
            return
        }
        saveAddress()
    }

    // MARK: Test

    private func test() async {
        testing = true
        defer { testing = false }
        do {
            let listed = try await model.listModels(selection, job: job)
            if !usesOwnWorkflow {
                models = listed
                self.listed = true
            }
            let count = listed.count
            let models = count == 1 ? "1 model available" : "\(count) models available"
            // With the person's own workflow, the count would be its loader's files, not models AutoPaper can use.
            let says = count > 0 && !usesOwnWorkflow ? "It works: \(models)." : "It works."
            status = .works(says)
            model.announce(says)
        } catch {
            log.error("Provider test failed: \(String(describing: error), privacy: .public)")
            if let problem = KeyProblem(error, selection: selection) {
                status = .key(problem)
                model.announce("\(problem.spokenContext) \(problem.linkTitle).")
                return
            }
            // Replaces a model-list error rather than adding to it: the same problem is said once.
            let sentence = Sentences.error(error, retrying: false, inSettings: true, addresses: addressInUse) ?? "The test was cancelled."
            status = .problem(sentence, unavailable: Self.isUnavailable(error))
            model.announce(sentence)
        }
    }

    private static func isUnavailable(_ error: any Error) -> Bool {
        if case .ProviderUnavailable(_, .notRunning, _) = error as? AutoPaperError { return true }
        return false
    }

    // MARK: ComfyUI workflow

    /// A file chosen before, or the one in use.
    private var hasOwnFile: Bool {
        usesOwnWorkflow || !ownText.isEmpty
    }

    @ViewBuilder
    private var workflowRow: some View {
        LabeledContent {
            Picker("\(jobName) workflow", selection: Binding(get: { usesOwnWorkflow ? WorkflowChoice.own : .autoPaper }, set: chooseWorkflow)) {
                Text(WorkflowChoice.autoPaperTitle).tag(WorkflowChoice.autoPaper)
                if hasOwnFile {
                    Text(WorkflowChoice.ownTitle(fileName: ownName)).tag(WorkflowChoice.own)
                }
                Divider()
                Text(WorkflowChoice.chooseTitle(hasFile: hasOwnFile)).tag(WorkflowChoice.chooseFile)
            }
            .labelsHidden()
            .fixedSize()
        } label: {
            Text("Workflow")
            if !usesOwnWorkflow {
                Text("AutoPaper's is made for each model below. Your own must be saved with Export (API), with {{prompt}}, {{width}}, {{height}} and {{seed}}.")
            }
        }
        if let workflowProblem {
            // The one place a problem with the file is said, with what fixes it.
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Label(workflowProblem, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.primary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(WorkflowChoice.chooseTitle(hasFile: true)) { chooseFile() }
                    .buttonStyle(.link)
                    .accessibilityLabel("Choose another painting workflow")
            }
        }
    }

    private func chooseWorkflow(_ choice: WorkflowChoice) {
        switch choice {
        case .autoPaper:
            guard usesOwnWorkflow else { return }
            workflowProblem = nil
            checkSavedModel = true
            model.updateSettings { $0.comfyuiWorkflow = nil }
        case .own:
            guard !usesOwnWorkflow else { return }
            guard !ownText.isEmpty else { DispatchQueue.main.async { chooseFile() }; return }
            let text = ownText
            model.updateSettings { $0.comfyuiWorkflow = text }
        case .chooseFile:
            // After the menu closes; cancelling the panel keeps the choice as it was.
            DispatchQueue.main.async { chooseFile() }
        }
    }

    /// Opens the file panel as a sheet on Settings (cancelling it keeps the choice as it was).
    private func chooseFile() {
        choosingFile = true
    }

    private func useFile(_ result: Result<URL, any Error>) {
        guard case .success(let url) = result else { return }
        let file = url.lastPathComponent
        // The sandbox lets AutoPaper read the file the person chose, for as long as it says it's using it.
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            let text = try String(contentsOf: url, encoding: .utf8)
            // The core's checks when it fills a workflow in, made now, so a file that can't work isn't kept only to
            // fail at the next wallpaper. The choice stays as it was.
            if case .problem(let reason) = OwnWorkflow.read(text) {
                workflowProblem = "“\(file)” wasn't used. \(Sentences.invalidInput(reason, inSettings: true))"
                model.announce(workflowProblem ?? "")
                return
            }
            workflowProblem = nil
            ownText = text
            ownName = file
            model.updateSettings { $0.comfyuiWorkflow = text }
        } catch {
            workflowProblem = "Couldn't read “\(file)”: \(error.localizedDescription)"
            model.announce(workflowProblem ?? "")
        }
    }

    /// Reads the workflow in use (its model, or why it can't be used: said once, at the workflow row), and
    /// remembers it, so switching to AutoPaper's and back is one choice.
    private func readOwnWorkflow() {
        guard selection.kind == .comfyUi, let workflow else {
            ownReading = nil
            return
        }
        if ownText != workflow {
            // Set elsewhere (an earlier version, the CLI): its file name isn't known.
            ownText = workflow
            ownName = ""
        }
        ownReading = OwnWorkflow.read(workflow)
        if case .problem(let reason) = ownReading {
            workflowProblem = Sentences.invalidInput(reason, inSettings: true)
        } else {
            workflowProblem = nil
        }
    }

    private struct ModelsKey: Equatable {
        let kind: ProviderKind
        let address: String?
        let workflow: String?
        /// A key saved or removed in Accounts reloads the list.
        let keys: Int
    }

    private struct EstimatesKey: Equatable {
        let selection: ProviderSelection
        let writer: ProviderSelection?
        let models: [String]
        let revision: Int
    }
}

/// "Setting up local models" (systemPatterns.md, user request 2026-10-05): for Ollama and ComfyUI, a footnote
/// naming the Brew Browser bundle that installs it, and a button that opens the bundle in Brew Browser
/// (`brewbrowser://bundle/<id>`, navigate-only) when something handles the scheme, else Brew Browser's website.
struct BrewBrowserHelp: View {
    let kind: ProviderKind
    var emphasised = false
    @State private var canOpenLinks = false

    static let website = URL(string: "https://brew-browser.zerologic.com")!

    private var bundle: (id: String, name: String, installs: String) {
        kind == .comfyUi ? ("image-gen", "Image Generation", "ComfyUI") : ("local-llm", "Local LLMs", "Ollama")
    }

    private var link: URL { URL(string: "brewbrowser://bundle/\(bundle.id)")! }

    var body: some View {
        LabeledContent {
            Button(canOpenLinks ? "Open in Brew Browser" : "Get Brew Browser…") {
                NSWorkspace.shared.open(canOpenLinks ? link : Self.website)
            }
            .fixedSize()
            // Two of these sit side by side in the welcome: the spoken name says which bundle, and starts with
            // the visible title so voice control still finds it.
            .accessibilityLabel(canOpenLinks
                                ? "Open in Brew Browser: \(bundle.name) bundle"
                                : "Get Brew Browser for \(bundle.installs)")
        } label: {
            if emphasised {
                // The problem line above already says it isn't answering (said once, spec 6a); this is the fix.
                Label("Not installed? Brew Browser can set it up: its \(bundle.name) bundle installs \(bundle.installs). Models are separate downloads.", systemImage: "arrow.down.circle")
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text("Not installed? Brew Browser can set it up: its \(bundle.name) bundle installs \(bundle.installs). Models are separate downloads.")
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .onAppear {
            // Older Brew Browser builds don't handle the scheme; they get the website, like no Brew Browser at all.
            canOpenLinks = NSWorkspace.shared.urlForApplication(toOpen: link) != nil
        }
    }
}
