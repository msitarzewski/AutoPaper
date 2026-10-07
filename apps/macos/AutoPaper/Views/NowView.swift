import AutopaperCore
import SwiftUI

/// Now: the wallpaper on the desktop, large; its title, summary and echo note; which models wrote and painted it;
/// Like and Dislike; then the status lines: when the next one comes, the month's spend, anything that went wrong or a
/// wallpaper brought back, and the narrow-keywords note. The toolbar's New Wallpaper Now (⌘R) becomes Stop (Esc)
/// while one is being made, like Safari's reload; the stage shows on the picture.
struct NowView: View {
    @Environment(AppModel.self) private var model
    /// The colours of the wallpaper showing, for the smoke while a new one is made (`WorkingEffect`).
    @State private var palette: [Color]?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                switch model.phase {
                case .starting:
                    ProgressView("Opening your library…")
                        .frame(maxWidth: .infinity, minHeight: 240)
                case .failed(let reason):
                    ContentUnavailableView("AutoPaper Couldn't Start", systemImage: "exclamationmark.triangle", description: Text(reason))
                case .ready:
                    if let current = model.current {
                        showing(current)
                    } else {
                        ContentUnavailableView {
                            Label("No Wallpaper Yet", systemImage: "sparkles")
                        } description: {
                            Text("Choose New Wallpaper Now to make your first one from your current mood's keywords.")
                        } actions: {
                            MakeControls()
                        }
                        .frame(minHeight: 260)
                    }
                    Divider()
                    StatusLines()
                }
            }
            .frame(maxWidth: 820, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .navigationTitle("Now")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                MakeOrStopButton()
            }
        }
    }

    @ViewBuilder
    private func showing(_ current: Generation) -> some View {
        WallpaperImage(current, maxPixelSize: 2_000)
            .working(model.isWorking, palette: palette)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(.separator)
            }
            .frame(maxHeight: 340)
            .frame(maxWidth: .infinity)
            .accessibilityElement()
            .accessibilityAddTraits(.isImage)
            .accessibilityLabel(model.currentSpokenDescription)
            .task(id: current.id) {
                let paths = [current.thumbPath, current.imagePath].compactMap { $0 }
                palette = await LoadedImage.load(paths, maxPixelSize: 96).flatMap { ImagePalette.colors(of: $0.cgImage) }
            }
            // Outside the image's accessibility element, so VoiceOver reads the stage and progress on their own.
            .overlay(alignment: .bottom) {
                if let work = model.work {
                    StageCapsule(work: work)
                        .padding(.bottom, 14)
                        .transition(.opacity)
                }
            }

        VStack(alignment: .leading, spacing: 8) {
            Text(current.concept.title)
                .font(.title2.weight(.semibold))
                .textSelection(.enabled)
                .accessibilityAddTraits(.isHeader)
            Text(current.concept.summary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if let note = current.echoNote, !note.isEmpty {
                Label(note, systemImage: "arrow.trianglehead.2.clockwise.rotate.90")
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ProvenanceLine(generation: current)
        }

        // New Wallpaper Now and Stop are the toolbar's one button (no second one here to fight it).
        HStack(spacing: 10) {
            RatingToggle(generation: current, rating: .liked)
            RatingToggle(generation: current, rating: .disliked)
            Spacer()
        }
    }
}

/// Which models made a wallpaper (user, 2026-10-06): "Written by … · Painted by … · 3840×2160 · about $0.04", the
/// painter a link to Settings → Providers (where painting is chosen). VoiceOver reads the whole line, then the link.
struct ProvenanceLine: View {
    @Environment(AppModel.self) private var model
    let generation: Generation

    var body: some View {
        let line = Provenance.line(generation, listed: model.modelNames)
        let tail = line.tail.trimmingCharacters(in: .whitespaces)
        FlowLayout(spacing: 4, lineSpacing: 2) {
            Text(line.written + " ·")
            Button(line.painted) { model.openSettings(.providers) }
                .buttonStyle(.link)
                .help("Choose who paints in Settings → Providers")
                .accessibilityHint("Opens Providers in Settings.")
            if !tail.isEmpty {
                Text(tail)
            }
        }
        .font(.callout)
        .quietText()
        .accessibilityElement(children: .contain)
        .accessibilityLabel(line.spoken)
    }
}

/// Like or Dislike as a toggle button: its state is the generation's rating, and VoiceOver hears it as on or off.
struct RatingToggle: View {
    @Environment(AppModel.self) private var model
    let generation: Generation
    let rating: Rating

    var body: some View {
        let isOn = generation.rating == rating
        Toggle(isOn: Binding(get: { isOn }, set: { _ in model.rate(generation, rating) })) {
            Label(rating == .liked ? "Like" : "Dislike", systemImage: rating == .liked ? "hand.thumbsup" : "hand.thumbsdown")
                .symbolVariant(isOn ? .fill : .none)
        }
        .toggleStyle(.button)
        .help(isOn ? "Choose again to clear the rating" : (rating == .liked ? "More like this" : "Less like this"))
    }
}

/// The status lines (each an icon and words, never colour alone).
struct StatusLines: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let settings = model.settings {
                // What the app's own timer will do: before scheduled wallpapers have started (the welcome skipped,
                // nothing made yet) the engine's due time isn't acted on, so it isn't promised.
                Label(
                    Formatting.nextLine(due: model.nextDue, paused: settings.paused, cadence: settings.cadence, armed: model.scheduleArmed),
                    systemImage: settings.paused ? "pause.circle" : "clock"
                )
            }
            if let spend = model.spend {
                Label(Formatting.budgetLine(spentMicroUSD: spend.spentMicrousd, budgetCents: spend.budgetCents), systemImage: "dollarsign.circle")
            }
            if let notice = model.notice {
                if let link = notice.link {
                    // A problem with a setting: its line and one link to the fix (spec 6a).
                    SettingsProblemLink(problem: link)
                } else {
                    Label(notice.text, systemImage: notice.kind == .error ? "exclamationmark.triangle" : "info.circle")
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if model.narrow {
                Label(Sentences.narrowKeywords, systemImage: "lightbulb")
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let own = DesktopStatus.ownPictureLine(mode: model.wallpaperMode, ownPictureShowing: model.ownPictureShowing,
                                                      paused: model.settings?.paused ?? false) {
                Label(own, systemImage: "photo")
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Status")
    }
}
