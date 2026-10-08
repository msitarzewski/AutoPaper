import AutopaperCore
import SwiftUI

/// Memory: what AutoPaper has learned from ratings (shared by every mood) with Reset What It Learned…, the quiet
/// period, echoes, whether memory is full or reduced (and why), images on disk and the storage limit, and Clear
/// History… (offering to keep memory, so AutoPaper still avoids repeats).
struct MemorySettings: View {
    @Environment(AppModel.self) private var model
    @State private var usage: StorageUsage?
    @State private var confirmingClear = false
    @State private var confirmingReset = false
    @State private var problem: String?

    var body: some View {
        SettingsPane { settings in
            Section {
                learned
            } header: {
                Text("What AutoPaper has learned")
            } footer: {
                Text("From your likes and dislikes, across every mood.")
            }

            Section {
                Picker(selection: model.setting(\.quietPeriod, .sixMonths)) {
                    ForEach(QuietPeriod.all, id: \.self) { Text($0.title).tag($0) }
                } label: {
                    Text("Quiet period")
                    Text("How long before AutoPaper may make something similar to a wallpaper again.")
                }
                Picker(selection: model.setting(\.echoes, .sometimes)) {
                    ForEach(EchoFrequency.all, id: \.self) { Text($0.title).tag($0) }
                } label: {
                    Text("Echoes")
                    Text("Once an idea's quiet period has passed, it may come back as a new take on the old one.")
                }
                if let memory = model.memory {
                    // The reason is in words below; the engine's `problem` is English for logs and support, never shown.
                    LabeledContent {
                        // A reading the person needs, so in primary text (secondary measures 3.9:1 in Light mode).
                        Text(memory.reduced ? "Reduced" : "Full")
                            .foregroundStyle(.primary)
                    } label: {
                        Text("Memory")
                        Text(memoryExplanation(memory))
                    }
                }
            }

            Section("Storage") {
                LabeledContent("Images on disk") {
                    if let usage {
                        Text("\(Formatting.bytes(usage.imageBytes)) · \(usage.imagesOnDisk == 1 ? "1 image" : "\(usage.imagesOnDisk) images")")
                            .monospacedDigit()
                            .foregroundStyle(.primary)
                    } else {
                        ProgressView().controlSize(.small)
                    }
                }
                // Saved and pruned in one write; the size is measured again after it (`historyRevision`).
                Picker(selection: Binding(get: { settings.storageLimitMb }, set: { limit in
                    Task { await model.setStorageLimit(limit) }
                })) {
                    ForEach(Choices.storageLimitsMB, id: \.self) { Text(Choices.storageTitle($0)).tag($0) }
                    if !Choices.storageLimitsMB.contains(settings.storageLimitMb) {
                        Text(Choices.storageTitle(settings.storageLimitMb)).tag(settings.storageLimitMb)
                    }
                } label: {
                    Text("Keep up to")
                    Text("The oldest images are removed first; liked ones and what's on your desktop are kept. Memory of every wallpaper stays.")
                }
            }

            Section("History") {
                LabeledContent {
                    Button("Clear History…") { confirmingClear = true }
                } label: {
                    Text("History")
                    Text(historyCount)
                }
                LabeledContent {
                    Text("Up to 200 runs, kept for 30 days on this Mac. Requests made before Console was added were not recorded.")
                        .fixedSize(horizontal: false, vertical: true)
                } label: { Text("Console") }
                LabeledContent {
                    Text("Copy Details and Export JSON include a run's prompts, models, responses and outcomes. Clear Console deletes these records while keeping wallpapers and budget spending.")
                        .fixedSize(horizontal: false, vertical: true)
                } label: { Text("Run reports") }
                LabeledContent {
                    Text("Outcomes cover retained runs. Average call time uses recorded model calls; older runs may lack model timings.")
                        .fixedSize(horizontal: false, vertical: true)
                } label: { Text("Charts") }
                LabeledContent {
                    Text("Run records stay on this Mac. Reports can contain your keywords and private content; review them before sharing.")
                        .fixedSize(horizontal: false, vertical: true)
                } label: { Text("Privacy") }
                Link("Console help", destination: Website.console)
            }
        }
        .task(id: model.historyRevision) { await measure() }
        // Both choices delete every image and can't be undone, so both are destructive: neither becomes the default
        // button, and Return can't clear anything (HIG, Alerts). Cancel stays the safe way out (Esc).
        .confirmationDialog("Clear your history?", isPresented: $confirmingClear) {
            Button("Clear History, Keep Memory", role: .destructive) { clear(keepMemory: true) }
            Button("Clear History and Memory", role: .destructive) { clear(keepMemory: false) }
        } message: {
            Text("Every wallpaper's image is deleted. Keep memory, and AutoPaper still remembers what it made, so it won't repeat itself. Clearing memory too also forgets what you like.")
        }
        .confirmationDialog("Reset what AutoPaper has learned?", isPresented: $confirmingReset) {
            Button("Reset", role: .destructive) { model.resetTaste() }
        } message: {
            Text("AutoPaper forgets which features you like and dislike. Your ratings stay on each wallpaper.")
        }
        .alert("Couldn't Clear History", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            Button("OK") { problem = nil }
        } message: {
            Text(problem ?? "")
        }
    }

    /// The liked and disliked features (`taste_summary`), or why there are none yet, and Reset What It Learned….
    @ViewBuilder
    private var learned: some View {
        if let taste = model.taste, !(taste.liked.isEmpty && taste.disliked.isEmpty) {
            // The lists are what this section is for, so they're in primary text (a form's values are secondary,
            // 3.9:1 in Light mode).
            LabeledContent("Liked") {
                Text(taste.liked.isEmpty ? "Nothing yet" : taste.liked.joined(separator: ", "))
                    .multilineTextAlignment(.trailing)
                    .textSelection(.enabled)
                    .foregroundStyle(.primary)
            }
            LabeledContent("Disliked") {
                Text(taste.disliked.isEmpty ? "Nothing yet" : taste.disliked.joined(separator: ", "))
                    .multilineTextAlignment(.trailing)
                    .textSelection(.enabled)
                    .foregroundStyle(.primary)
            }
        } else if (model.taste?.ratings ?? 0) == 0 {
            Text("Nothing yet. Like or dislike a few wallpapers and AutoPaper learns what you enjoy.")
                .fixedSize(horizontal: false, vertical: true)
        } else {
            Text("Nothing clear yet. A feature needs a few ratings before AutoPaper counts it as something you like or dislike.")
                .fixedSize(horizontal: false, vertical: true)
        }
        LabeledContent {
            Button("Reset What It Learned…") { confirmingReset = true }
                .disabled(nothingLearned)
        } label: {
            Text(ratingCount)
        }
    }

    private var nothingLearned: Bool {
        guard let taste = model.taste else { return true }
        return taste.ratings == 0 && taste.liked.isEmpty && taste.disliked.isEmpty
    }

    private var ratingCount: String {
        switch model.taste?.ratings ?? 0 {
        case 0: "No ratings yet"
        case 1: "From 1 rating"
        case let count: "From \(count) ratings"
        }
    }

    private var historyCount: String {
        guard let usage else { return "Wallpapers AutoPaper has made." }
        return usage.generations == 1 ? "1 wallpaper remembered." : "\(usage.generations) wallpapers remembered."
    }

    private func memoryExplanation(_ memory: MemoryStatus) -> String {
        if memory.reduced {
            return "The language model AutoPaper uses to recognise similar ideas isn't available, so memory only catches near-identical repeats. Reinstalling AutoPaper restores it."
        }
        return "Recognises similar ideas, even in different words (\(memory.embeddingModel), on this Mac)."
    }

    private func measure() async {
        usage = await model.storageUsage()
    }

    private func clear(keepMemory: Bool) {
        Task {
            problem = await model.clearHistory(keepMemory: keepMemory)
            await measure()
        }
    }
}
