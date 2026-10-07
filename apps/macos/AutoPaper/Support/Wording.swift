import AutopaperCore
import Foundation

// Everything AutoPaper says, in AudioPaper's voice: plain, specific, short. macOS menu items and buttons are in
// title case (HIG); labels and sentences are in sentence case. The engine's enums get their words here, so the
// menus, the window, Settings and App Intents all say the same thing.

extension ProviderKind {
    /// The provider's name in pickers and sentences.
    var name: String {
        switch self {
        case .openAi: "OpenAI"
        case .google: "Google Gemini"
        case .ollama: "Ollama"
        case .openAiCompatible: "OpenAI-compatible"
        case .comfyUi: "ComfyUI"
        case .demo: "Demo"
        }
    }

    /// The provider as the subject of a sentence ("ComfyUI isn't running…", "Your server didn't accept…").
    var subject: String {
        switch self {
        case .openAiCompatible: "Your OpenAI-compatible server"
        case .demo: "The Demo provider"
        default: name
        }
    }

    /// Runs on the person's own Mac or network, at an address they can change.
    var takesAddress: Bool {
        switch self {
        case .ollama, .comfyUi, .openAiCompatible: true
        default: false
        }
    }

    /// The address used when the field is left blank (the core's `default_base_url`); nil when the person must
    /// enter one (OpenAI-compatible) or the provider is hosted.
    var defaultAddress: String? {
        defaultBaseUrl(kind: self)
    }

    /// The model picker's blank choice: "Default (gpt-6-luna)", from the core's `default_model`. Ollama and
    /// OpenAI-compatible servers have no fixed default (they use the first model they list); a ComfyUI workflow of
    /// the person's own always loads its own model. ComfyUI's own workflows name their models, so its default reads
    /// "Z-Image Turbo (default)" (the name from `models`, the list `list_models` gave; else AutoPaper's own name for
    /// it, `BundledWorkflows`).
    func defaultModelTitle(job: ProviderJob, ownWorkflow: Bool = false, models: [ModelInfo] = []) -> String {
        if self == .comfyUi && ownWorkflow { return "Default (your workflow's model)" }
        let model = defaultModel(kind: self, job: job)
        if self == .comfyUi && job == .images && !model.isEmpty {
            return "\(BundledWorkflows.name(of: model, listed: models)) (default)"
        }
        if !model.isEmpty { return "Default (\(model))" }
        switch self {
        case .ollama, .openAiCompatible: return "Default (the server's first model)"
        default: return "Default"
        }
    }

    /// Can write ideas (`ProviderKind.writers`) / can paint (`ProviderKind.painters`), for wording `Unsupported`.
    var writes: Bool { Self.writers.contains(self) }
    var paints: Bool { Self.painters.contains(self) }

    /// Providers for writing ideas, in the order Settings lists them (spec: OpenAI · Google Gemini · Ollama ·
    /// OpenAI-compatible · Demo).
    static let writers: [ProviderKind] = [.openAi, .google, .ollama, .openAiCompatible, .demo]
    /// Providers for painting (OpenAI · Google Gemini · OpenAI-compatible · ComfyUI · Demo).
    static let painters: [ProviderKind] = [.openAi, .google, .openAiCompatible, .comfyUi, .demo]
}

/// What a choice in Settings → Providers changes in the engine's settings: Writing's pickers the writing provider,
/// Painting's the painting provider (`update_settings` saves it; the next wallpaper asks that model and records it).
enum ProviderChoice {
    static func keyPath(_ job: ProviderJob) -> WritableKeyPath<AutopaperCore.Settings, ProviderSelection> {
        job == .concepts ? \.textProvider : \.imageProvider
    }

    /// The Model menu's choice ("" = the provider's default), kept with the provider and address chosen.
    static func choose(model id: String, for job: ProviderJob, in settings: inout AutopaperCore.Settings) {
        settings[keyPath: keyPath(job)].model = id.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// The Model menu in Providers: its items (tag = the model id saved in settings, "" = the default) and their titles.
enum ModelMenu {
    struct Item: Equatable {
        let tag: String
        let title: String
    }

    /// The blank choice (the default) first, then the listed models. ComfyUI's are named in words ("Krea 2 Turbo"),
    /// the default's own entry being the blank choice and the rest by name; other providers' show the name and id
    /// when they differ. A saved model the list doesn't have stays in the menu, so the menu shows what's saved: as
    /// is while the list couldn't be read (offline, no key yet), and marked "(not available)" for ComfyUI once the
    /// server has answered without it (no workflow for it, or its files are gone).
    static func items(kind: ProviderKind, job: ProviderJob, models: [ModelInfo], saved: String, listed: Bool) -> [Item] {
        var items = [Item(tag: "", title: kind.defaultModelTitle(job: job, models: models))]
        var others = models
        if kind == .comfyUi {
            let defaultID = defaultModel(kind: kind, job: job)
            others = models.filter { $0.id != defaultID || $0.id == saved }
                .sorted { $0.displayName.localizedStandardCompare($1.displayName) == .orderedAscending }
        }
        if !saved.isEmpty, !models.contains(where: { $0.id == saved }) {
            // ComfyUI's are named in words when AutoPaper knows the model; marked once the server has answered.
            let title = kind == .comfyUi ? BundledWorkflows.name(of: saved) : saved
            items.append(Item(tag: saved, title: kind == .comfyUi && listed ? "\(title) (not available)" : title))
        }
        items += others.map { Item(tag: $0.id, title: title(of: $0, kind: kind)) }
        return items
    }

    static func title(of info: ModelInfo, kind: ProviderKind) -> String {
        if kind == .comfyUi && !info.displayName.isEmpty { return info.displayName }
        return info.displayName.isEmpty || info.displayName == info.id ? info.id : "\(info.displayName) (\(info.id))"
    }

    /// Names by id, for wording ComfyUI's models elsewhere.
    static func names(_ models: [ModelInfo]) -> [String: String] {
        Dictionary(models.filter { !$0.displayName.isEmpty }.map { ($0.id, $0.displayName) }, uniquingKeysWith: { first, _ in first })
    }
}

extension Cadence {
    static let all: [Cadence] = [.hourly, .every3Hours, .every6Hours, .every12Hours, .daily, .weekly, .manual]

    var title: String {
        switch self {
        case .hourly: "Every hour"
        case .every3Hours: "Every 3 hours"
        case .every6Hours: "Every 6 hours"
        case .every12Hours: "Every 12 hours"
        case .daily: "Every day"
        case .weekly: "Every week"
        case .manual: "Only when I ask"
        }
    }
}

extension QuietPeriod {
    static let all: [QuietPeriod] = [.oneMonth, .threeMonths, .sixMonths, .oneYear, .twoYears]

    var title: String {
        switch self {
        case .oneMonth: "1 month"
        case .threeMonths: "3 months"
        case .sixMonths: "6 months"
        case .oneYear: "1 year"
        case .twoYears: "2 years"
        }
    }
}

extension EchoFrequency {
    static let all: [EchoFrequency] = [.off, .rarely, .sometimes, .often]

    var title: String {
        switch self {
        case .off: "Off"
        case .rarely: "Rarely"
        case .sometimes: "Sometimes"
        case .often: "Often"
        }
    }
}

extension Fallback {
    var title: String {
        switch self {
        case .revisitLiked: "Bring back one I liked"
        case .keepCurrent: "Keep the current one"
        }
    }
}

extension ImageQuality {
    var title: String {
        switch self {
        case .standard: "Standard"
        case .high: "High"
        }
    }
}

extension KeywordWeight {
    static let all: [KeywordWeight] = [.must, .maybe, .avoid]

    var title: String {
        switch self {
        case .must: "Must"
        case .maybe: "Maybe"
        case .avoid: "Avoid"
        }
    }

    /// What the weight does, for help tags.
    var explanation: String {
        switch self {
        case .must: "Always part of the scene."
        case .maybe: "Used some of the time."
        case .avoid: "Never part of the scene."
        }
    }
}

extension Rating {
    /// The rating in AutoPaper's own words, for spoken descriptions (`describe` leaves it out). Nil when unrated.
    var spoken: String? {
        switch self {
        case .liked: "Liked"
        case .disliked: "Disliked"
        case .unrated: nil
        }
    }
}

extension ProgressStage {
    /// What AutoPaper is doing now; nil once it's done.
    var title: String? {
        switch self {
        case .composing: "Composing an idea…"
        case .checkingMemory: "Checking memory…"
        case .generating: "Painting…"
        case .downloading: "Downloading…"
        case .rendering: "Preparing for your displays…"
        case .done: nil
        }
    }
}

/// Surprise in words. The band itself is the core's (`surprise_band`, the composer's own thresholds); the app only
/// names and explains it.
extension SurpriseBand {
    /// The band for a slider position from 0 to 100.
    init(percent: Int) {
        self = surpriseBand(surprise: Float(percent) / 100)
    }

    var title: String {
        switch self {
        case .faithful: "Faithful"
        case .fresh: "Fresh"
        case .adventurous: "Adventurous"
        case .wild: "Wild"
        }
    }

    /// One line under the slider, from the composer's own description of each band.
    var explanation: String {
        switch self {
        case .faithful: "Each keyword as anyone would picture it, in a natural, believable scene."
        case .fresh: "Natural scenes, each with one unexpected choice of viewpoint, light, season, era or medium."
        case .adventurous: "Keywords read freely, with unexpected settings, scales, eras and styles."
        case .wild: "Keywords are loose inspiration for surprising, dreamlike scenes. Musts still appear."
        }
    }

    /// What VoiceOver says for the slider's value: "35 percent, fresh".
    static func spokenValue(percent: Int) -> String {
        "\(percent) percent, \(SurpriseBand(percent: percent).title.lowercased())"
    }
}

/// Plain sentences for everything that can go wrong, and for a wallpaper brought back instead of a new one.
/// Worded from the error's variant and typed reason only: the core's `detail` is English for logs, never shown.
enum Sentences {
    /// What to tell the person about an engine error; nil for a cancel (they asked for it). `retrying` adds that
    /// AutoPaper tries again later, for errors of making a wallpaper (not for a Test in Settings). `inSettings`:
    /// the sentence is shown in Settings, next to what it's about; elsewhere (Now, the menu bar, a notification) a
    /// problem with a setting also says where to fix it. `addresses` gives the server address in use for local
    /// providers, for "isn't running at …" (default: the core's).
    static func error(_ error: AutoPaperError, retrying: Bool = true, inSettings: Bool = false, addresses: [ProviderKind: String] = [:]) -> String? {
        let later = retrying ? " AutoPaper will try again later." : ""
        switch error {
        case .MissingKey(let provider):
            switch provider {
            case .openAiCompatible: return "Add the key for your OpenAI-compatible server in Settings → Accounts to start."
            default: return "Add your \(provider.name) key in Settings → Accounts to start."
            }
        case .InvalidKey(let provider):
            return "\(provider.subject) didn't accept your key. Check it in Settings → Accounts."
        case .ProviderUnavailable(let provider, let reason, _):
            switch reason {
            case .notRunning:
                if let address = addresses[provider] ?? provider.defaultAddress {
                    return "\(provider.subject) isn't running at \(address)." + later
                }
                return "\(provider.subject) isn't answering. Check that it's running and its address is right." + later
            case .timedOut:
                return "\(provider.subject) took too long to answer." + later
            case .serverError:
                return "\(provider.subject) had a problem on its side." + later
            case .stopped:
                return "The job was stopped in \(provider.name)." + later
            case .other:
                return "\(provider.subject) isn't available right now." + later
            }
        case .RateLimited(let provider, let retryAfterSecs):
            let minutes = max(1, Int((Double(retryAfterSecs) / 60).rounded(.up)))
            return "\(provider.name) asked AutoPaper to wait; it'll try again in \(minutes == 1 ? "1 minute" : "\(minutes) minutes")."
        case .Refused(let provider):
            return "\(provider.subject) declined to paint this idea; AutoPaper tried a gentler one, and it declined that too."
        case .Unsupported(let provider, _):
            // Worded from what the provider can do, not the core's English `job`.
            if provider.writes && !provider.paints {
                return "\(provider.subject) can't paint. Choose a painting provider in Settings → Providers."
            }
            if provider.paints && !provider.writes {
                return "\(provider.subject) can't write ideas. Choose a writing provider in Settings → Providers."
            }
            return "\(provider.subject) can't do that. Choose another provider in Settings → Providers."
        case .BudgetReached:
            return "This month's budget is spent. New wallpapers start again next month, or raise the budget in Settings → Budget."
        case .Offline:
            return "AutoPaper can't reach the internet." + (retrying ? " It'll try again soon." : "")
        case .InvalidResponse:
            return "The answer from your provider couldn't be used." + later
        case .PaintingFailed(let provider, let model, _):
            // Not something waiting fixes, so no "try again later". Away from Settings it says where to look; the
            // Now view shows `PaintingProblem` instead (the same line with one link to Providers).
            let line = PaintingProblem.sentence(provider: provider, model: model)
            return inSettings ? line : line + " Check it in Settings → Providers."
        case .KeywordNotFollowed(let keyword, let weight, _):
            // Asking again rarely helps, so no "try again later". Where there's no link (Siri, Shortcuts) it says
            // where to change it; the Now view shows `SettingsProblem` instead (the line with a link to the mood).
            return keywordNotFollowed(keyword, weight) + " Change it in Moods."
        case .InvalidInput(let reason, _):
            return invalidInput(reason, inSettings: inSettings)
        case .NotFound:
            return "That wallpaper's image isn't there any more."
        case .NothingToRevisit:
            return "There's no liked wallpaper to bring back yet."
        case .Storage:
            return "AutoPaper couldn't read or write its files. Check that your disk isn't full."
        case .Cancelled:
            return nil
        case .Internal:
            return internalProblem
        }
    }

    /// What was wrong with something the person entered (or, for `displaySizeInvalid` and `other`, a mistake of
    /// AutoPaper's own, said generically). `inSettings`: said next to the field it's about (the server address, the
    /// workflow, the model list); otherwise a problem with a setting names where to fix it. Keyword problems are
    /// always shown next to the keyword.
    static func invalidInput(_ reason: InvalidInputReason, inSettings: Bool = true) -> String {
        switch reason {
        case .keywordEmpty: "A keyword needs at least one letter or number."
        case .keywordTooLong: "Keywords can be up to 40 characters."
        case .tooManyKeywords: "You can have up to 64 keywords. Remove one to add another."
        case .duplicateKeyword: "Another keyword already has this text."
        case .addressMissing:
            inSettings
                ? "Enter the server's address."
                : "Enter your server's address in Settings → Providers."
        case .addressNotAllowed:
            inSettings
                ? "AutoPaper can't use that address. Use https, or http only for this Mac or your local network, with no user name or password in it."
                : "AutoPaper can't use the server address in Settings → Providers. Use https, or http only for this Mac or your local network, with no user name or password in it."
        case .addressInvalid:
            inSettings
                ? "That isn't a complete web address. Include http:// or https:// and the server's name."
                : "The server address in Settings → Providers isn't complete. Include http:// or https:// and the server's name."
        case .noModels:
            inSettings
                ? "The server has no models to use yet. Download one there, then choose Refresh."
                : "Your server has no models to use yet. Download one there and AutoPaper will use it."
        case .workflowNeedsPrompt:
            inSettings
                ? "That workflow needs a {{prompt}} placeholder where AutoPaper puts the scene."
                : "Your ComfyUI workflow needs a {{prompt}} placeholder where AutoPaper puts the scene. Fix it, or use the bundled one in Settings → Providers."
        case .workflowNotApiFormat:
            inSettings
                ? "That workflow is in ComfyUI's editor format. In ComfyUI, choose Export (API) and use that file."
                : "Your ComfyUI workflow is in ComfyUI's editor format. In ComfyUI, choose Export (API), then use that file in Settings → Providers."
        case .workflowInvalid:
            inSettings
                ? "That workflow isn't valid once AutoPaper fills in {{prompt}}, {{width}}, {{height}} and {{seed}}. Check it in ComfyUI and export it again."
                : "Your ComfyUI workflow isn't valid once AutoPaper fills in {{prompt}}, {{width}}, {{height}} and {{seed}}. Export it again, or use the bundled one in Settings → Providers."
        case .nothingToEcho: "Only a finished wallpaper can have an echo."
        case .moodNameEmpty: "A mood needs a name."
        case .moodNameTooLong: "Mood names can be up to \(MoodText.maxNameLength) characters."
        case .duplicateMoodName: "Another mood already has this name."
        case .lastMood: "There's always at least one mood, so the last one can't be deleted."
        case .displaySizeInvalid, .other: internalProblem
        }
    }

    static let internalProblem = "Something went wrong inside AutoPaper. Please try again."

    /// The writing model kept breaking one of the mood's keywords (`KeywordNotFollowed`), in one line that names
    /// it: a Must it left out, or an Avoid it brought in.
    static func keywordNotFollowed(_ keyword: String, _ weight: KeywordWeight) -> String {
        let quoted = "“\(KeywordText.normalised(keyword))”"
        return switch weight {
        case .must: "The writing model kept leaving out \(quoted)."
        case .avoid: "The writing model kept including \(quoted), which this mood avoids."
        // Not reported for a Maybe (the core names a Must or an Avoid); said plainly should that change.
        case .maybe: "The writing model kept getting \(quoted) wrong."
        }
    }

    /// A server address the engine refused, said under the address field: what was typed, then why.
    static func addressRefused(_ typed: String, _ reason: InvalidInputReason) -> String {
        "“\(typed)” wasn't saved. \(invalidInput(reason, inSettings: true))"
    }

    /// Any error thrown by an engine call or by AutoPaper itself.
    static func error(_ error: any Error, retrying: Bool = true, inSettings: Bool = false, addresses: [ProviderKind: String] = [:]) -> String? {
        if let engine = error as? AutoPaperError {
            return self.error(engine, retrying: retrying, inSettings: inSettings, addresses: addresses)
        }
        if let cancel = error as? CancellationError { _ = cancel; return nil }
        return (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
    }

    /// The line shown when a past wallpaper came back instead of a new one.
    static func revisit(_ reason: RevisitReason) -> String {
        switch reason {
        case .requested: "Brought back a wallpaper you liked."
        case .overBudget: "This month's budget is spent. AutoPaper is bringing back wallpapers you liked."
        case .offline: "AutoPaper couldn't reach the internet, so it brought back one you liked. It'll try again soon."
        case .providerFailed: "A new wallpaper couldn't be made, so AutoPaper brought back one you liked. It'll try again soon."
        }
    }

    static let narrowKeywords =
        "Your keywords are narrow, so new ideas are getting hard to find. Add some Maybes or raise Surprise for more variety."
}

extension InvalidInputReason {
    /// A problem with the server address (Settings → Providers), shown under that field rather than in an alert.
    var isAboutAddress: Bool {
        switch self {
        case .addressMissing, .addressNotAllowed, .addressInvalid: true
        default: false
        }
    }
}

/// Money and schedule lines.
enum Formatting {
    /// Micro-US-dollars as dollars and cents ("$1.20"); sub-cent amounts keep a third decimal ("$0.004").
    static func dollars(microUSD: UInt64) -> String {
        let dollars = Double(microUSD) / 1_000_000
        let digits = dollars > 0 && dollars < 0.01 ? 3 : 2
        return dollars.formatted(.currency(code: "USD").precision(.fractionLength(digits)).locale(Locale(identifier: "en_US")))
    }

    static func dollars(cents: UInt32) -> String {
        dollars(microUSD: UInt64(cents) * 10_000)
    }

    /// "$1.20 of $5.00 this month (estimated)", or without a cap "$1.20 this month (estimated)".
    static func budgetLine(spentMicroUSD: UInt64, budgetCents: UInt32?) -> String {
        let spent = dollars(microUSD: spentMicroUSD)
        guard let budgetCents else { return "\(spent) this month (estimated)" }
        return "\(spent) of \(dollars(cents: budgetCents)) this month (estimated)"
    }

    /// "Next new wallpaper at 9:00", "… tomorrow at 9:00", "… on Friday at 9:00", or "Paused" / "Only when you ask".
    /// `armed` is false until scheduled wallpapers have started (the person made one, finished the welcome or chose
    /// a real provider): the engine's due time is then only what *would* happen, so the line says what will.
    /// `short` drops "new" and shortens the not-started line, for the sidebar's one-line footer ("Next wallpaper at
    /// 3:00 PM").
    static func nextLine(due: Date?, paused: Bool, cadence: Cadence, armed: Bool = true, short: Bool = false, now: Date = .now,
                         calendar: Calendar = .current, locale: Locale = .current) -> String {
        if paused { return "Paused" }
        if cadence == .manual { return "Only when you ask" }
        if !armed { return short ? "Starts after your first wallpaper" : notStartedLine }
        guard let due else { return "Only when you ask" }
        let next = short ? "Next wallpaper" : "Next new wallpaper"
        if due <= now.addingTimeInterval(30) { return "\(next) soon" }
        var time = Date.FormatStyle(date: .omitted, time: .shortened)
        time.calendar = calendar
        time.locale = locale
        time.timeZone = calendar.timeZone
        let clock = due.formatted(time)
        if calendar.isDate(due, inSameDayAs: now) { return "\(next) at \(clock)" }
        if let tomorrow = calendar.date(byAdding: .day, value: 1, to: now), calendar.isDate(due, inSameDayAs: tomorrow) {
            return "\(next) tomorrow at \(clock)"
        }
        var day = Date.FormatStyle().locale(locale)
        day.calendar = calendar
        day.timeZone = calendar.timeZone
        if due < now.addingTimeInterval(6 * 86_400) {
            return "\(next) on \(due.formatted(day.weekday(.wide))) at \(clock)"
        }
        return "\(next) on \(due.formatted(day.month(.abbreviated).day())) at \(clock)"
    }

    /// The footer's status where the sidebar is too narrow for `footerLine`, in the style of Time Machine's "Next
    /// backup: Tomorrow, 9:00 AM": "Next: Today, 3:00 PM", "Next: Tomorrow, 6:33 PM", "Next: Friday, 9:00 AM".
    static func footerShortLine(stage: String?, due: Date?, paused: Bool, cadence: Cadence, armed: Bool, now: Date = .now,
                                calendar: Calendar = .current, locale: Locale = .current) -> String {
        if let stage { return stage }
        if paused { return "Paused" }
        if cadence == .manual { return "Only when you ask" }
        if !armed { return "Not started yet" }
        guard let due else { return "Only when you ask" }
        if due <= now.addingTimeInterval(30) { return "Next wallpaper soon" }
        var time = Date.FormatStyle(date: .omitted, time: .shortened)
        time.calendar = calendar
        time.locale = locale
        time.timeZone = calendar.timeZone
        let clock = due.formatted(time)
        if calendar.isDate(due, inSameDayAs: now) { return "Next: Today, \(clock)" }
        if let tomorrow = calendar.date(byAdding: .day, value: 1, to: now), calendar.isDate(due, inSameDayAs: tomorrow) {
            return "Next: Tomorrow, \(clock)"
        }
        var day = Date.FormatStyle().locale(locale)
        day.calendar = calendar
        day.timeZone = calendar.timeZone
        let when = due < now.addingTimeInterval(6 * 86_400) ? due.formatted(day.weekday(.wide)) : due.formatted(day.month(.abbreviated).day())
        return "Next: \(when), \(clock)"
    }

    /// The sidebar footer's status: what's being made ("Painting…") while AutoPaper works, else when the next one
    /// comes ("Next wallpaper at 3:00 PM", "Paused"). Also what VoiceOver says, whichever form fits on screen.
    static func footerLine(stage: String?, due: Date?, paused: Bool, cadence: Cadence, armed: Bool, now: Date = .now,
                           calendar: Calendar = .current, locale: Locale = .current) -> String {
        if let stage { return stage }
        return nextLine(due: due, paused: paused, cadence: cadence, armed: armed, short: true, now: now, calendar: calendar, locale: locale)
    }

    /// Scheduled wallpapers haven't started: nothing changes the desktop until the person makes the first one.
    static let notStartedLine = "New wallpapers start after you make your first one"

    /// A rough duration in words, for estimates and time left: "a few seconds", "about 40 seconds", "about 1
    /// minute", "about 9 minutes", "about 2 hours". Rounded so it doesn't flicker while counting down (seconds to
    /// the nearest 5, minutes to the nearest whole one).
    static func roughly(seconds: UInt32) -> String {
        switch seconds {
        case 0..<10:
            return "a few seconds"
        case 10..<58:
            return "about \(Int((Double(seconds) / 5).rounded()) * 5) seconds"
        case 58..<(90 * 60):
            let minutes = max(1, Int((Double(seconds) / 60).rounded()))
            return minutes == 1 ? "about 1 minute" : "about \(minutes) minutes"
        default:
            let hours = Int((Double(seconds) / 3_600).rounded())
            return hours == 1 ? "about 1 hour" : "about \(hours) hours"
        }
    }

    /// What's left of the painting, under the stage: "About 6 minutes left"; nil when nothing backs a number (the
    /// engine gives none) — never a made-up one. 0 means only saving is left.
    static func timeLeft(seconds: UInt32?) -> String? {
        guard let seconds else { return nil }
        if seconds == 0 { return "Almost done" }
        let words = roughly(seconds: seconds)
        return words.prefix(1).uppercased() + words.dropFirst() + " left"
    }

    /// How far along the painting is, spoken with the ring: "40 percent".
    static func percent(_ fraction: Float) -> String {
        "\(Int((min(max(fraction, 0), 1) * 100).rounded(.down))) percent"
    }

    /// Where a provider's recorded timings come from, for "… on this Mac".
    enum Place: Equatable {
        /// A hosted service: its speed isn't this computer's.
        case hosted
        /// This Mac (Demo, or a local server at a loopback address).
        case thisMac
        /// Another computer on the network (a local server's host).
        case host(String)

        /// Where `selection` runs: a local or compatible server's host from its address (blank = the core's
        /// default), else this Mac for Demo, else hosted.
        init(_ selection: ProviderSelection) {
            switch selection.kind {
            case .demo:
                self = .thisMac
            case .ollama, .comfyUi, .openAiCompatible:
                let address = selection.baseUrl ?? selection.kind.defaultAddress ?? ""
                let host = URL(string: address)?.host(percentEncoded: false)?.lowercased() ?? ""
                if host.isEmpty { self = .hosted; return }
                let loopback = ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host) || host.hasPrefix("127.")
                self = loopback ? .thisMac : (selection.kind == .openAiCompatible && !Self.isLocal(host) ? .hosted : .host(host))
            default:
                self = .hosted
            }
        }

        private static func isLocal(_ host: String) -> Bool {
            host.hasSuffix(".local") || host.hasPrefix("10.") || host.hasPrefix("192.168.")
                || (host.hasPrefix("172.") && (16...31).contains(Int(host.split(separator: ".").dropFirst().first ?? "") ?? 0))
        }

        var suffix: String {
            switch self {
            case .hosted: ""
            case .thisMac: " on this Mac"
            case .host(let name): " on \(name)"
            }
        }
    }

    /// The estimate line under a model picker, from the engine's `estimate` (rounded seconds; nil until a call
    /// like it has finished here — then nothing is shown). Painting: "About 9 minutes per wallpaper on this Mac"
    /// (writing and painting together, as the engine suggests), or, while the writing provider hasn't been timed
    /// (`includesWriting` false), "About 9 minutes to paint on this Mac": a part is never presented as the whole.
    /// Writing: "About 5 seconds per idea on this Mac".
    static func estimateLine(seconds: UInt32?, job: ProviderJob, place: Place, includesWriting: Bool = true) -> String? {
        guard let seconds else { return nil }
        let words = roughly(seconds: max(seconds, 1))
        let per = job == .images ? (includesWriting ? "per wallpaper" : "to paint") : "per idea"
        return words.prefix(1).uppercased() + words.dropFirst() + " \(per)\(place.suffix)"
    }

    /// The same estimate after a model's name in its menu: "Qwen-Image 2.1 (about 9 minutes)", or inside a title's
    /// own parentheses: "Z-Image Turbo (default, about 30 seconds)".
    static func withEstimate(_ title: String, seconds: UInt32?) -> String {
        guard let seconds else { return title }
        let words = roughly(seconds: max(seconds, 1))
        if title.hasSuffix(")"), title.contains(" (") { return "\(title.dropLast()), \(words))" }
        return "\(title) (\(words))"
    }

    /// The price table's date (the core's `prices_as_of`, "2026-10-05") as "October 5, 2026".
    static func pricesDate(_ iso: String = pricesAsOf(), locale: Locale = .current) -> String {
        let parser = Date.ISO8601FormatStyle().year().month().day()
        guard let date = try? parser.parse(iso) else { return iso }
        return date.formatted(Date.FormatStyle(date: .long, time: .omitted, locale: locale, timeZone: TimeZone(identifier: "UTC")!))
    }

    static func bytes(_ count: UInt64) -> String {
        Int64(clamping: count).formatted(.byteCount(style: .file))
    }

    /// History dates: "Oct 5, 2026".
    static func day(_ unix: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(unix)).formatted(date: .abbreviated, time: .omitted)
    }
}

/// Which models made a wallpaper (user, 2026-10-06): "Written by Gemini 3.5 Flash-Lite (Google Gemini) · Painted by
/// Nano Banana 2 (Google Gemini) · 3840×2160 · about $0.04". From the generation's own record (the models the engine
/// actually used, defaults resolved), so it shows whether a model chosen in Settings was applied.
enum Provenance {
    /// Names for model ids, for when the provider's own list (`list_models`' display names) hasn't been read this
    /// session: Google's names from its model list (core/tests/fixtures/google), ComfyUI's from `BundledWorkflows`.
    /// OpenAI's list names its models by id, so they stay ids.
    static let knownNames: [String: String] = [
        "gemini-3.1-flash-image": "Nano Banana 2",
        "gemini-3.1-flash-lite-image": "Nano Banana 2 Lite",
        "gemini-3-pro-image": "Nano Banana Pro",
        "gemini-2.5-flash-image": "Nano Banana",
        "gemini-3.5-flash-lite": "Gemini 3.5 Flash-Lite",
        "gemini-3.1-flash-lite": "Gemini 3.1 Flash-Lite",
        "gemini-3.8-flash": "Gemini 3.8 Flash",
    ]

    /// A model in words: the provider's own name (`listed`, by id), else AutoPaper's, else the id. Demo's is "Demo";
    /// an empty id (a record from before models were kept) is the provider's name.
    static func modelName(_ id: String, kind: ProviderKind, listed: [String: String] = [:]) -> String {
        let model = id.trimmingCharacters(in: .whitespacesAndNewlines)
        if kind == .demo || model.isEmpty { return kind.name }
        if let name = listed[model], !name.isEmpty { return name }
        if kind == .comfyUi { return BundledWorkflows.name(of: model) }
        return knownNames[model] ?? model
    }

    /// The model and its provider: "Nano Banana 2 (Google Gemini)", "Z-Image Turbo (ComfyUI)", or "Demo" alone.
    static func who(_ kind: ProviderKind, model: String, listed: [String: String] = [:]) -> String {
        let name = modelName(model, kind: kind, listed: listed)
        return name == kind.name ? name : "\(name) (\(kind.name))"
    }

    struct Line: Equatable {
        /// "Written by Gemini 3.5 Flash-Lite (Google Gemini)".
        let written: String
        /// "Painted by Nano Banana 2 (Google Gemini)": a link to Settings → Providers where it's shown.
        let painted: String
        /// "3840×2160"; nil when the size wasn't kept.
        let size: String?
        /// "about $0.04"; nil when it cost nothing (local providers, Demo).
        let cost: String?

        /// The line as shown.
        var text: String { [written, painted, size, cost].compactMap { $0 }.joined(separator: " · ") }

        /// The line as VoiceOver says it: commas, and "3840 by 2160".
        var spoken: String {
            [written, painted, size.map { $0.replacingOccurrences(of: "×", with: " by ") }, cost].compactMap { $0 }.joined(separator: ", ")
        }

        /// What comes after the painter, with its separator (" · 3840×2160 · about $0.04"), for showing the painter
        /// as a link between the two halves.
        var tail: String { [size, cost].compactMap { $0 }.map { " · \($0)" }.joined() }
    }

    static func line(_ generation: Generation, listed: [String: String] = [:]) -> Line {
        let size = generation.width > 0 && generation.height > 0 ? "\(generation.width)×\(generation.height)" : nil
        let hosted: Set<ProviderKind> = [.openAi, .google, .openAiCompatible]
        let paid = generation.costMicrousd > 0 && (hosted.contains(generation.textProvider) || hosted.contains(generation.imageProvider))
        return Line(
            written: "Written by \(who(generation.textProvider, model: generation.textModel, listed: listed))",
            painted: "Painted by \(who(generation.imageProvider, model: generation.imageModel, listed: listed))",
            size: size,
            cost: paid ? "about \(Formatting.dollars(microUSD: generation.costMicrousd))" : nil
        )
    }

    /// The diagnostic log line for a new wallpaper (no prompt, no keys): "made <id>: text openAi/gpt-6-luna, image
    /// google/gemini-3.1-flash-image, 3840x2160".
    static func logLine(_ generation: Generation) -> String {
        "made \(generation.id): text \(generation.textProvider)/\(generation.textModel), "
            + "image \(generation.imageProvider)/\(generation.imageModel), \(generation.width)x\(generation.height)"
    }
}

/// The spoken description of a wallpaper: the engine's `describe` (title, summary, echo note) plus the rating in
/// AutoPaper's words, which `describe` leaves to the host.
enum Spoken {
    static func wallpaper(description: String, rating: Rating) -> String {
        guard let rated = rating.spoken else { return description }
        let base = description.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !base.isEmpty else { return rated }
        return base + (base.last.map { ".!?…".contains($0) } == true ? " " : ". ") + rated + "."
    }
}

/// The settings choices offered for budget and storage.
enum Choices {
    /// Monthly budget in cents; nil = no limit. Anything else is "Custom".
    static let budgets: [UInt32?] = [nil, 100, 200, 500, 1_000, 2_000]
    static let storageLimitsMB: [UInt32] = [256, 512, 1_024, 2_048, 5_120]

    static func storageTitle(_ megabytes: UInt32) -> String {
        megabytes >= 1_024 ? "\(megabytes / 1_024) GB" : "\(megabytes) MB"
    }
}

/// Normalising keyword text the way the engine does (trimmed, single-spaced), so a duplicate can be found
/// before asking the engine (which would change the existing keyword's weight).
enum KeywordText {
    static func normalised(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    /// The existing keyword this text duplicates (case-insensitively), if any.
    static func duplicate(of text: String, in keywords: [Keyword]) -> Keyword? {
        let wanted = normalised(text).lowercased()
        return keywords.first { normalised($0.text).lowercased() == wanted }
    }
}
