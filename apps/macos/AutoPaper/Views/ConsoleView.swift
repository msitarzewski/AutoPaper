import AppKit
import AutopaperCore
import Charts
import SwiftUI
import UniformTypeIdentifiers

/// Date → runs → outcomes, with the selected run's real requests, responses and decisions. History's image grid
/// and filmstrip cannot show attempts that never produced an image, so this uses a native selectable list instead.
struct ConsoleView: View {
    @Environment(AppModel.self) private var model
    @State private var items: [RunRecord] = []
    @State private var selection: String?
    @State private var selected: RunRecord?
    @State private var statistics: ConsoleStatistics?
    @State private var loading = false
    @State private var requested: UInt32 = page
    @State private var exhausted = false
    @State private var loadProblem: String?
    @State private var actionProblem: String?
    @State private var confirmingClear = false
    @State private var exporting = false
    @State private var report: RunReportDocument?
    @State private var reporting = false
    private static let page: UInt32 = 60

    var body: some View {
        GeometryReader { geometry in
            HSplitView {
                runList
                    .frame(minWidth: 220, idealWidth: 280, maxWidth: 360, maxHeight: .infinity)
                Group {
                    if let selected {
                        RunDetails(run: selected, statistics: statistics)
                    } else if loading {
                        ProgressView("Loading run details…")
                    } else {
                        ContentUnavailableView("Select a Run", systemImage: "terminal")
                    }
                }
                .frame(minWidth: 340, maxWidth: .infinity, maxHeight: .infinity)
            }
            // A native split view sizes empty content to its intrinsic height unless given the actual column
            // bounds. Keep its divider and both empty states the full height of the available detail.
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .navigationTitle("Console")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Copy Details", systemImage: "doc.on.doc") { makeReport(export: false) }
                    .disabled(selection == nil || reporting)
                    .help("Copy the selected run's complete JSON report")
            }
            ToolbarItem(placement: .primaryAction) {
                Button("Export JSON…", systemImage: "square.and.arrow.up") { makeReport(export: true) }
                    .disabled(selection == nil || reporting)
                    .help("Save the selected run's complete JSON report")
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("Refresh") { Task { await refresh() } }
                        .disabled(loading)
                    Divider()
                    Button("Clear Console…", role: .destructive) { confirmingClear = true }
                        .disabled(items.isEmpty || model.isWorking || items.contains { $0.status == .running })
                } label: {
                    Label("Console Actions", systemImage: "ellipsis.circle")
                }
                .accessibilityLabel("Console Actions")
                .help("Refresh or clear local run records")
            }
        }
        .task(id: model.phase == .ready) {
            guard model.phase == .ready else { return }
            await refresh()
            while !Task.isCancelled {
                // New attempts and stage details appear while this section is visible. The task cancels when the
                // person switches sections; idle Console polling is light and doesn't start the engine's schedule.
                try? await Task.sleep(for: .seconds(model.isGenerating || items.contains { $0.status == .running } ? 2 : 10))
                guard !Task.isCancelled else { return }
                await refresh()
                await model.refreshStatus()
            }
        }
        .task(id: selection) { await loadSelected() }
        .confirmationDialog("Clear Console?", isPresented: $confirmingClear) {
            Button("Clear Console", role: .destructive) {
                Task {
                    do {
                        try await model.clearRuns()
                        items = []
                        selection = nil
                        selected = nil
                        requested = Self.page
                        await refresh()
                    } catch { actionProblem = model.sentence(for: error, retrying: false) ?? error.localizedDescription }
                }
            }
        } message: {
            Text("Run records and their request and response details are deleted. Your wallpapers and budget spending stay.")
        }
        .alert("Couldn't Complete That", isPresented: Binding(
            get: { actionProblem != nil }, set: { if !$0 { actionProblem = nil } }
        )) {
            Button("OK") { actionProblem = nil }
        } message: { Text(actionProblem ?? "") }
        .fileExporter(isPresented: $exporting, document: report, contentType: .json,
                      defaultFilename: "AutoPaper-run-\(selection ?? "details")") { result in
            if case .failure(let error) = result { actionProblem = error.localizedDescription }
        }
    }

    private var runList: some View {
        VStack(spacing: 0) {
            if let loadProblem {
                Label(loadProblem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .padding(12)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if items.isEmpty {
                if loading || model.phase != .ready {
                    ProgressView("Loading runs…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView("No Runs Yet", systemImage: "terminal")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else {
                List(selection: $selection) {
                    ForEach(RunPresentation.days(items)) { day in
                        Section(day.id.formatted(date: .abbreviated, time: .omitted)) {
                            ForEach(day.runs, id: \.id) { run in
                                RunRow(run: run)
                                    .tag(run.id)
                            }
                        }
                    }
                    if !exhausted {
                        Button(loading ? "Loading…" : "Load Older Runs") {
                            requested = min(200, requested + Self.page)
                            Task { await refresh() }
                        }
                        .disabled(loading)
                    }
                }
                .accessibilityLabel("Runs by date")
            }
        }
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        defer { loading = false }
        do {
            let followingNewest = selection == nil || selection == items.first?.id
            var fresh: [RunRecord] = []
            var reachedEnd = false
            while fresh.count < Int(requested) {
                let limit = min(Self.page, requested - UInt32(fresh.count))
                let batch = try await model.runs(limit: limit, offset: UInt32(fresh.count))
                guard !Task.isCancelled else { return }
                fresh.append(contentsOf: batch)
                if batch.count < Int(limit) { reachedEnd = true; break }
            }
            guard !Task.isCancelled else { return }
            // A new run may arrive between page reads. Keep each native list row's identity unique until the next
            // refresh reads that boundary again.
            var seen = Set<String>()
            items = fresh.filter { seen.insert($0.id).inserted }
            statistics = try await model.consoleStatistics()
            exhausted = reachedEnd || requested >= 200
            loadProblem = nil
            if followingNewest || !fresh.contains(where: { $0.id == selection }) { selection = fresh.first?.id }
            await loadSelected()
        } catch {
            guard !Task.isCancelled else { return }
            loadProblem = model.sentence(for: error, retrying: false) ?? error.localizedDescription
        }
    }

    private func loadSelected() async {
        guard let id = selection else { selected = nil; return }
        if selected?.id != id { selected = items.first { $0.id == id } }
        do {
            let fresh = try await model.run(id)
            guard !Task.isCancelled, selection == id else { return }
            selected = fresh
        } catch {
            guard !Task.isCancelled, selection == id else { return }
            loadProblem = model.sentence(for: error, retrying: false) ?? error.localizedDescription
        }
    }

    private func makeReport(export: Bool) {
        guard let id = selection, !reporting else { return }
        reporting = true
        Task {
            defer { reporting = false }
            do {
                let text = try await model.runReport(id)
                if export {
                    report = RunReportDocument(text: text)
                    exporting = true
                } else {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                }
            } catch { actionProblem = model.sentence(for: error, retrying: false) ?? error.localizedDescription }
        }
    }
}

private struct RunRow: View {
    let run: RunRecord

    var body: some View {
        VStack(alignment: .leading) {
            HStack {
                Label(RunPresentation.outcome(run.status), systemImage: RunPresentation.symbol(run.status))
                Spacer()
                Text(Date(timeIntervalSince1970: TimeInterval(run.startedAt)), style: .time)
            }
            Text(run.moodName.isEmpty ? RunPresentation.trigger(run.trigger) : run.moodName)
                .lineLimit(1)
            Text(RunPresentation.trigger(run.trigger))
                .lineLimit(1)
            if !run.detail.isEmpty {
                Text(verbatim: run.detail)
                    .lineLimit(2)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct RunDetails: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings
    let run: RunRecord
    let statistics: ConsoleStatistics?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
                Label(RunPresentation.outcome(run.status), systemImage: RunPresentation.symbol(run.status))
                    .font(.title2.weight(.semibold))
                    .accessibilityAddTraits(.isHeader)
                if !run.detail.isEmpty {
                    Text(verbatim: run.detail)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if run.status == .blocked {
                    Button("Open Settings") { SettingsWindow.show(openSettings) }
                        .buttonStyle(.link)
                }
                VStack(alignment: .leading, spacing: 8) {
                    LabeledContent("Started", value: Date(timeIntervalSince1970: TimeInterval(run.startedAt))
                        .formatted(date: .abbreviated, time: .standard))
                    if let duration = RunPresentation.duration(run) { LabeledContent("Duration", value: duration) }
                    LabeledContent("Requested by", value: RunPresentation.trigger(run.trigger))
                    LabeledContent("Mood", value: run.moodName.isEmpty ? "Not recorded" : run.moodName)
                    Text(MoodText.surpriseLine(run.surprise))
                    ForEach(MoodText.keywordGroups(snapshots: run.keywords), id: \.weight) { group in
                        Text(group.line).fixedSize(horizontal: false, vertical: true)
                    }
                    LabeledContent("Writing", value: Provenance.who(run.textProvider, model: run.textModel, listed: model.modelNames))
                    LabeledContent("Writing model ID", value: run.textModel.isEmpty ? "Default; see requests below" : run.textModel)
                    LabeledContent("Painting", value: Provenance.who(run.imageProvider, model: run.imageModel, listed: model.modelNames))
                    LabeledContent("Painting model ID", value: run.imageModel.isEmpty ? "Default; see requests below" : run.imageModel)
                    LabeledContent("Estimated cost", value: Formatting.consoleDollars(microUSD: run.costMicrousd))
                    Text("Run ID: \(run.id)").font(.caption.monospaced()).textSelection(.enabled)
                    if let generation = run.generationId {
                        Text("Wallpaper ID: \(generation)").font(.caption.monospaced()).textSelection(.enabled)
                    }
                }
                .font(.callout)
                .textSelection(.enabled)
                Divider()
                if run.events.isEmpty {
                    Text(run.status == .running ? "Waiting for the next stage…" : "No provider request was recorded for this run.")
                        .font(.callout)
                } else {
                    Text("Requests and outcomes")
                        .font(.headline)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(run.events.enumerated()), id: \.offset) { index, event in
                        RunEventDetails(event: event, number: index + 1)
                    }
                }
                // The selected run comes first (it's what a click asks for); the overview of every run is below it, shut
                // until opened, and stays as the person left it.
                if let statistics, statistics.total > 0 {
                    Divider()
                    ConsoleCharts(statistics: statistics)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Run details")
    }
}

/// The engine supplies retained outcomes and timings of calls linked to those runs. No historic provider average
/// is substituted when a recorded run has no call timing.
private struct ConsoleCharts: View {
    let statistics: ConsoleStatistics
    @AppStorage("consoleOverviewExpanded") private var expanded = false

    var body: some View {
        DisclosureGroup("Run overview", isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 18) {
                HStack(alignment: .firstTextBaseline, spacing: 20) {
                    metric("Runs", "\(statistics.total)")
                    if let rate = statistics.successRate {
                        metric("Success", rate.formatted(.percent.precision(.fractionLength(0...1))))
                    }
                    if let seconds = statistics.averageRunSecs {
                        metric("Average run", RunPresentation.brief(seconds))
                    }
                }
                .font(.callout)
                .monospacedDigit()

                outcomeChart
                if !statistics.models.isEmpty { modelChart }
                if !statistics.days.isEmpty { dayChart }

                DisclosureGroup("Show as Table") {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(statistics.outcomes.enumerated()), id: \.offset) { _, outcome in
                            LabeledContent(RunPresentation.outcome(outcome.status), value: "\(outcome.count)")
                        }
                        ForEach(Array(statistics.days.enumerated()), id: \.offset) { _, day in
                            LabeledContent("\(RunPresentation.utcDay(day.dayStart)) · \(RunPresentation.outcome(day.status))",
                                           value: "\(day.count)")
                        }
                        ForEach(Array(statistics.models.enumerated()), id: \.offset) { _, timing in
                            LabeledContent {
                                Text("\(RunPresentation.seconds(timing.averageSecs)) · \(RunPresentation.calls(timing.calls))")
                            } label: {
                                Text(RunPresentation.modelTimingTitle(timing))
                                Text(timing.model)
                            }
                        }
                    }
                    .font(.callout)
                    .textSelection(.enabled)
                }
            }
            .padding(.top, 12)
        }
        .font(.headline)
    }

    private var outcomeChart: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Outcomes").accessibilityAddTraits(.isHeader)
            Chart(Array(recordedOutcomes.enumerated()), id: \.offset) { _, outcome in
                BarMark(x: .value("Runs", Int(outcome.count)), y: .value("Outcome", RunPresentation.outcome(outcome.status)),
                        height: .fixed(18))
                    .foregroundStyle(color(outcome.status))
                    .annotation(position: .top, alignment: .leading, spacing: 3) {
                        Text(RunPresentation.outcome(outcome.status)).font(.caption)
                    }
                    .annotation(position: .trailing) { Text("\(outcome.count)").font(.caption).monospacedDigit() }
                    .accessibilityLabel(RunPresentation.outcome(outcome.status))
                    .accessibilityValue("\(outcome.count) runs")
            }
            // The outcome's name sits above its bar (the axis would put long names over the bars).
            .chartYAxis(.hidden)
            .chartXScale(domain: 0...(outcomeTicks.last ?? 1))
            .chartXAxis { AxisMarks(values: outcomeTicks) { value in
                AxisGridLine()
                AxisValueLabel { if let count = value.as(Int.self) {
                    Text("\(count)").foregroundStyle(Color.primary.opacity(0.7))
                } }
            } }
            .frame(height: CGFloat(max(1, recordedOutcomes.count)) * 50 + 28)
            .accessibilityLabel("Run outcomes")
        }
    }

    private var dayChart: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Runs by day (UTC)").accessibilityAddTraits(.isHeader)
            Chart(Array(statistics.days.enumerated()), id: \.offset) { _, day in
                BarMark(x: .value("Day", RunPresentation.utcDay(day.dayStart)), y: .value("Runs", Int(day.count)))
                    .foregroundStyle(by: .value("Outcome", RunPresentation.outcome(day.status)))
                    .accessibilityLabel("\(RunPresentation.utcDay(day.dayStart)), \(RunPresentation.outcome(day.status))")
                    .accessibilityValue("\(day.count) runs")
            }
            .chartForegroundStyleScale(domain: recordedOutcomes.map { RunPresentation.outcome($0.status) },
                                       range: recordedOutcomes.map { color($0.status) })
            .chartXAxis { AxisMarks(values: .automatic(desiredCount: 4)) }
            .chartYScale(domain: 0...(dayTicks.last ?? 1))
            .chartYAxis { AxisMarks(values: dayTicks) { value in
                AxisGridLine()
                AxisValueLabel { if let count = value.as(Int.self) {
                    Text("\(count)").foregroundStyle(Color.primary.opacity(0.7))
                } }
            } }
            .frame(height: 150)
            .accessibilityLabel("Daily run outcomes")
        }
    }

    private var modelChart: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Average call time").accessibilityAddTraits(.isHeader)
            Chart(Array(statistics.models.enumerated()), id: \.offset) { _, timing in
                BarMark(x: .value("Seconds", timing.averageSecs), y: .value("Model", RunPresentation.modelTimingTitle(timing)),
                        height: .fixed(18))
                    .foregroundStyle(by: .value("Step", timing.job == .concepts ? "Writing" : "Painting"))
                    .annotation(position: .top, alignment: .leading, spacing: 3) {
                        Text(RunPresentation.modelName(timing)).font(.caption).lineLimit(1)
                    }
                    .annotation(position: .trailing) {
                        Text("\(RunPresentation.brief(timing.averageSecs)) · \(RunPresentation.calls(timing.calls))")
                            .font(.caption).monospacedDigit()
                    }
                    .accessibilityLabel(RunPresentation.modelTimingTitle(timing))
                    .accessibilityValue("\(RunPresentation.seconds(timing.averageSecs)) average, \(RunPresentation.calls(timing.calls))")
            }
            .chartForegroundStyleScale(["Writing": Color.accentColor, "Painting": Color.purple])
            .chartLegend(position: .top, alignment: .leading)
            .chartYAxis(.hidden)
            .chartXAxisLabel("Seconds")
            .frame(height: CGFloat(max(1, statistics.models.count)) * 54 + 56)
            .accessibilityLabel("Average provider call times by used model")
        }
    }

    private var recordedOutcomes: [ConsoleOutcome] { statistics.outcomes.filter { $0.count > 0 } }
    private var outcomeTicks: [Int] {
        let maximum = Int(recordedOutcomes.map(\.count).max() ?? 0)
        // Leave room for the actual count at each bar's end, even in the narrowest detail column.
        return MoodActivity.ticks(maximum: maximum + max(1, maximum / 10))
    }
    private var dayTicks: [Int] {
        let totals = Dictionary(grouping: statistics.days, by: \.dayStart).values.map { $0.reduce(UInt64(0)) { $0 + $1.count } }
        return MoodActivity.ticks(maximum: Int(totals.max() ?? 0))
    }

    private func metric(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption)
            Text(value).font(.title3.weight(.medium)).textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private func color(_ status: RunStatus) -> Color {
        switch status {
        case .succeeded: .green
        case .failed: .red
        case .blocked: .orange
        case .cancelled: .gray
        case .interrupted: .yellow
        case .running: .accentColor
        }
    }
}

private struct RunEventDetails: View {
    let event: RunEvent
    let number: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text("\(number). \(RunPresentation.label(event.stage)) · \(RunPresentation.label(event.kind))")
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 8)
                Text(Date(timeIntervalSince1970: TimeInterval(event.at)), style: .time)
                    .font(.caption)
            }
            if let provider = event.provider {
                Text(Provenance.who(provider, model: event.model))
                    .font(.callout)
                    .textSelection(.enabled)
                if !event.model.isEmpty {
                    Text("Model ID: \(event.model)")
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                }
            }
            if let instructions = InstructionsDetail(detail: event.detail) {
                InstructionsDisclosures(instructions: instructions)
            } else if let body = EventBody(detail: event.detail, kind: event.kind) {
                if !body.head.isEmpty {
                    Text(verbatim: body.head)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                JSONDisclosure(body: body)
            } else if !event.detail.isEmpty {
                Text(verbatim: event.detail)
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(12)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .contain)
    }
}

/// What a writing call was sent, each part shut until opened: the instructions and the prompt as formatted text, the
/// schema as JSON.
private struct InstructionsDisclosures: View {
    let instructions: InstructionsDetail

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            MarkdownDisclosure(title: "System instructions", text: instructions.system)
            MarkdownDisclosure(title: "User prompt", text: instructions.user)
            JSONDisclosure(body: EventBody(json: instructions.schema, title: "Schema"))
            if let temperature = instructions.temperature {
                Text("Temperature (where supported): \(temperature)").font(.caption)
            }
        }
    }
}

private struct MarkdownDisclosure: View {
    let title: String
    let text: String
    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(MarkdownBlock.parse(text).enumerated()), id: \.offset) { _, block in
                    MarkdownBlockView(block: block)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .textSelection(.enabled)
            .padding(.top, 6)
        } label: {
            Text("\(title) · \(ByteCountFormatter.string(fromByteCount: Int64(text.utf8.count), countStyle: .file))")
                .font(.callout.weight(.medium))
        }
    }
}

private struct MarkdownBlockView: View {
    let block: MarkdownBlock

    var body: some View {
        switch block {
        case .heading(let level, let text):
            Text(inline(text))
                .font(level <= 1 ? .title3.weight(.semibold) : .headline)
                .padding(.top, 4)
                .accessibilityAddTraits(.isHeader)
        case .bullet(let text):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("•")
                Text(inline(text)).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, 8)
        case .numbered(let marker, let text):
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(marker).monospacedDigit()
                Text(inline(text)).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, 8)
        case .paragraph(let text):
            Text(inline(text)).fixedSize(horizontal: false, vertical: true)
        case .space:
            Spacer().frame(height: 4)
        }
    }

    /// `**bold**`, `*italic*` and `code` inside a line.
    private func inline(_ text: String) -> AttributedString {
        (try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text)
    }
}

private struct JSONDisclosure: View {
    let content: EventBody
    @State private var expanded = false
    @State private var highlighted: AttributedString?

    init(body: EventBody) { content = body }

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            VStack(alignment: .leading, spacing: 8) {
                if !content.valid {
                    Label(content.truncated ? "Cut off at 65,536 bytes, so this isn't complete JSON."
                                            : "This isn't valid JSON; shown as it was recorded.",
                          systemImage: "exclamationmark.triangle")
                        .font(.caption)
                }
                Text(highlighted ?? AttributedString(content.json))
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button("Copy \(content.title)", systemImage: "doc.on.doc") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(content.json, forType: .string)
                }
                .controlSize(.small)
            }
            .padding(.top, 6)
        } label: {
            Text("\(content.title) · \(content.size)")
                .font(.callout.weight(.medium))
        }
        // Coloured the first time it's opened: a long body costs nothing while it's shut.
        .onChange(of: expanded) { _, open in
            if open, highlighted == nil { highlighted = JSONColors.highlight(content.json) }
        }
    }
}

private struct RunReportDocument: FileDocument {
    static let readableContentTypes: [UTType] = [.json]
    let text: String

    init(text: String) { self.text = text }

    init(configuration: ReadConfiguration) throws {
        guard let data = configuration.file.regularFileContents, let text = String(data: data, encoding: .utf8) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        self.text = text
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: Data(text.utf8))
    }
}
