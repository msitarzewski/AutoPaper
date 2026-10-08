import AutopaperCore
import Foundation

/// A key the person needs to add or check, and where it lives. Shown as a single link to the field (the fix), never
/// as directions to follow.
struct KeyProblem: Equatable {
    let provider: ProviderKind
    /// The Keychain account (`secret_account_for`); nil for an OpenAI-compatible server with no address yet.
    let account: String?
    /// The provider refused the key, rather than there being none.
    let refused: Bool

    init?(_ error: any Error, selection: ProviderSelection? = nil) {
        let kind: ProviderKind
        switch error as? AutoPaperError {
        case .MissingKey(let provider): kind = provider; refused = false
        case .InvalidKey(let provider): kind = provider; refused = true
        default: return nil
        }
        provider = kind
        // The selection's own account (an OpenAI-compatible server's key belongs to its address), else the kind's.
        if let selection, selection.kind == kind, let own = secretAccountFor(selection: selection) {
            account = own
        } else {
            account = kind.keyAccount
        }
    }

    /// The link's title: the action itself.
    var linkTitle: String {
        let whose = provider == .openAiCompatible ? "your server's" : "your \(provider.name)"
        return refused ? "Check \(whose) key" : "Add \(whose) key"
    }

    /// Said before the title, for VoiceOver and Voice Control the title alone is enough to act on.
    var spokenContext: String {
        refused ? "\(provider.subject) didn't accept your key." : "\(provider.name) needs a key."
    }
}

extension ProviderKind {
    /// The Keychain account for this kind's key (OpenAI and Gemini; OpenAI-compatible keys depend on the server,
    /// see `secretAccountFor`).
    var keyAccount: String? {
        secretAccountFor(selection: ProviderSelection(kind: self, model: "", baseUrl: nil))
    }
}

/// A problem with one of AutoPaper's settings (or a mood's keywords), said once as a link that goes straight to where
/// it's fixed, never as directions to follow (spec 6a): in the Now view, in announcements and in notifications. A key
/// is the link alone ("Add your OpenAI key"); other problems are a short line and the link ("This month's budget is
/// spent. New wallpapers start again next month." · "Raise the budget"; "The writing model kept leaving out
/// “lighthouse”." · "Edit the keyword"). Worded from the error's variant and typed fields only.
struct SettingsProblem: Equatable {
    /// Where the link goes.
    enum Place: Equatable {
        /// Settings → Accounts, with this Keychain account's field focused (nil: the pane).
        case accounts(account: String?)
        /// Settings → Providers: `job`'s section is the one to fix (nil: either), with its server address field
        /// focused when `address`.
        case providers(job: ProviderJob?, address: Bool)
        /// Settings → Budget.
        case budget
        /// macOS's own Wallpaper settings: a picture of the person's that AutoPaper couldn't put back.
        case systemWallpaper
        /// A mood in the main window's Moods, with this keyword (its text, as the engine reported it) selected.
        case mood(id: String, keyword: String)
    }

    /// The line before the link; empty when the link says it all.
    let sentence: String
    /// The link: the action itself.
    let linkTitle: String
    /// What VoiceOver hears before the link's title (its hint, and the start of what's announced).
    let context: String
    let place: Place
    /// The icon before the line.
    let symbol: String

    /// What's announced and what a notification says: the problem, then the thing to do. The same words as the
    /// link, so what's heard matches what's shown.
    var spoken: String { "\(context) \(linkTitle)." }

    init(sentence: String, linkTitle: String, context: String? = nil, place: Place, symbol: String = "exclamationmark.triangle") {
        self.sentence = sentence
        self.linkTitle = linkTitle
        self.context = context ?? sentence
        self.place = place
        self.symbol = symbol
    }

    init(key: KeyProblem) {
        self.init(sentence: "", linkTitle: key.linkTitle, context: key.spokenContext, place: .accounts(account: key.account),
                  symbol: key.refused ? "exclamationmark.triangle" : "key")
    }

    init(painting: PaintingProblem) {
        self.init(sentence: painting.sentence, linkTitle: painting.linkTitle, place: .providers(job: .images, address: false))
    }

    /// The month's budget is spent; `bringingBack` when AutoPaper brought back a liked one instead.
    static func overBudget(bringingBack: Bool) -> SettingsProblem {
        SettingsProblem(
            sentence: bringingBack
                ? "This month's budget is spent. AutoPaper is bringing back wallpapers you liked."
                : "The next wallpaper would exceed this month's budget. Your current wallpaper stays. New wallpapers start again next month.",
            linkTitle: "Raise the budget",
            place: .budget,
            symbol: "dollarsign.circle"
        )
    }

    /// Exact amounts and the next run's estimate come from the engine, including when its estimate exceeds
    /// what's left even though the person hasn't spent the whole limit.
    static func budget(_ status: BudgetStatus) -> SettingsProblem? {
        guard status.blocked else { return nil }
        return SettingsProblem(sentence: status.message, linkTitle: "Adjust the budget", place: .budget, symbol: "dollarsign.circle")
    }

    /// One of the person's pictures couldn't be put back (an Aerial, or one recorded before AutoPaper kept a record).
    static let pictureNotPutBack = SettingsProblem(
        sentence: "AutoPaper couldn't put your own picture back.",
        linkTitle: "Open Wallpaper settings",
        place: .systemWallpaper,
        symbol: "photo"
    )

    /// The problem behind `error` from making a wallpaper, when it's one with a setting; nil otherwise (it's said in
    /// words alone). `writing` and `painting` are the providers in use; `ownWorkflow` whether ComfyUI runs the
    /// person's own workflow file.
    init?(_ error: any Error, writing: ProviderSelection?, painting: ProviderSelection?, ownWorkflow: Bool) {
        let selections = [(ProviderJob.concepts, writing), (.images, painting)].compactMap { job, selection in selection.map { (job, $0) } }
        switch error as? AutoPaperError {
        case .MissingKey(let kind), .InvalidKey(let kind):
            guard let key = KeyProblem(error, selection: selections.first { $0.1.kind == kind }?.1) else { return nil }
            self.init(key: key)
            return
        case .BudgetReached:
            self = .overBudget(bringingBack: false)
            return
        case .Unsupported(let provider, _):
            // Worded from what the provider can do, not the core's English `job`.
            if provider.writes && !provider.paints {
                self.init(sentence: "\(provider.subject) can't paint.", linkTitle: "Choose a painting provider", place: .providers(job: .images, address: false))
            } else if provider.paints && !provider.writes {
                self.init(sentence: "\(provider.subject) can't write ideas.", linkTitle: "Choose a writing provider", place: .providers(job: .concepts, address: false))
            } else {
                self.init(sentence: "\(provider.subject) can't do that.", linkTitle: "Choose another provider", place: .providers(job: nil, address: false))
            }
            return
        case .InvalidInput(let reason, _) where reason.isAboutAddress:
            let job = Self.addressJob(reason, selections)
            let kind = selections.first { $0.0 == job }?.1.kind
            let sentence: String
            switch reason {
            case .addressMissing: sentence = "\(kind?.subject ?? "Your server") has no address yet."
            case .addressNotAllowed: sentence = "AutoPaper can't use your server's address. It needs https, or http only for this Mac or your local network, with no user name or password in it."
            default: sentence = "Your server's address isn't complete."
            }
            self.init(sentence: sentence, linkTitle: reason == .addressMissing ? "Enter the server address" : "Check the server address",
                      place: .providers(job: job, address: true))
            return
        case .KeywordNotFollowed(let keyword, let weight, let moodId):
            // The fix is in the mood (reword the keyword, make it a Maybe, or remove it), not the provider's.
            self.init(sentence: Sentences.keywordNotFollowed(keyword, weight), linkTitle: "Edit the keyword",
                      place: .mood(id: moodId, keyword: keyword), symbol: "text.badge.xmark")
            return
        default:
            break
        }
        guard let painting = PaintingProblem(error, painting: painting, ownWorkflow: ownWorkflow) else { return nil }
        self.init(painting: painting)
    }

    /// The section whose address is wrong: for a missing one, the provider that has no default address and none
    /// typed; otherwise one with an address typed; else the first that takes an address.
    private static func addressJob(_ reason: InvalidInputReason, _ selections: [(ProviderJob, ProviderSelection)]) -> ProviderJob? {
        let local = selections.filter { $0.1.kind.takesAddress }
        let match = reason == .addressMissing
            ? local.first { $0.1.baseUrl == nil && $0.1.kind.defaultAddress == nil }
            : local.first { $0.1.baseUrl != nil }
        return (match ?? local.first)?.0
    }
}
