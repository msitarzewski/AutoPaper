import AutopaperCore
import Foundation
import Testing

@Suite("Surprise bands")
struct SurpriseBandTests {
    @Test("Slider positions map to the core's bands (thresholds 25, 50, 75)", arguments: [
        (0, SurpriseBand.faithful), (24, .faithful), (25, .fresh), (49, .fresh),
        (50, .adventurous), (74, .adventurous), (75, .wild), (100, .wild),
    ])
    func thresholds(percent: Int, band: SurpriseBand) {
        #expect(SurpriseBand(percent: percent) == band)
    }

    @Test func spokenValueNamesTheBand() {
        #expect(SurpriseBand.spokenValue(percent: 35) == "35 percent, fresh")
        #expect(SurpriseBand.spokenValue(percent: 80) == "80 percent, wild")
    }
}

@Suite("Error sentences")
struct ErrorSentenceTests {
    @Test func missingKeyNamesTheProvider() {
        #expect(Sentences.error(AutoPaperError.MissingKey(provider: .openAi)) == "Add your OpenAI key in Settings → Accounts to start.")
        #expect(Sentences.error(AutoPaperError.MissingKey(provider: .google)) == "Add your Google Gemini key in Settings → Accounts to start.")
        #expect(Sentences.error(AutoPaperError.MissingKey(provider: .openAiCompatible))
            == "Add the key for your OpenAI-compatible server in Settings → Accounts to start.")
    }

    @Test func rateLimitRoundsUpToWholeMinutes() {
        #expect(Sentences.error(AutoPaperError.RateLimited(provider: .openAi, retryAfterSecs: 30))
            == "OpenAI asked AutoPaper to wait; it'll try again in 1 minute.")
        #expect(Sentences.error(AutoPaperError.RateLimited(provider: .openAi, retryAfterSecs: 90))
            == "OpenAI asked AutoPaper to wait; it'll try again in 2 minutes.")
    }

    @Test func notRunningNamesTheAddressInUseNotTheDetail() {
        let error = AutoPaperError.ProviderUnavailable(provider: .comfyUi, reason: .notRunning, detail: "connection refused (os error 61)")
        // Blank address field: the core's default.
        #expect(Sentences.error(error) == "ComfyUI isn't running at http://127.0.0.1:8188. AutoPaper will try again later.")
        // The address the person set.
        #expect(Sentences.error(error, addresses: [.comfyUi: "http://127.0.0.1:8199"])
            == "ComfyUI isn't running at http://127.0.0.1:8199. AutoPaper will try again later.")
        // An OpenAI-compatible server has no default address.
        let compatible = AutoPaperError.ProviderUnavailable(provider: .openAiCompatible, reason: .notRunning, detail: "x")
        #expect(Sentences.error(compatible, retrying: false)
            == "Your OpenAI-compatible server isn't answering. Check that it's running and its address is right.")
    }

    @Test func testResultsDontPromiseARetry() {
        let error = AutoPaperError.ProviderUnavailable(provider: .comfyUi, reason: .notRunning, detail: "x")
        #expect(Sentences.error(error, retrying: false, addresses: [.comfyUi: "http://127.0.0.1:8199"])
            == "ComfyUI isn't running at http://127.0.0.1:8199.")
        #expect(Sentences.error(AutoPaperError.Offline, retrying: false) == "AutoPaper can't reach the internet.")
    }

    @Test func unavailabilityIsWordedByReason() {
        func sentence(_ reason: ProviderUnavailableReason, _ provider: ProviderKind = .openAi) -> String? {
            Sentences.error(AutoPaperError.ProviderUnavailable(provider: provider, reason: reason, detail: "timed out"), retrying: false)
        }
        #expect(sentence(.timedOut) == "OpenAI took too long to answer.")
        #expect(sentence(.serverError) == "OpenAI had a problem on its side.")
        #expect(sentence(.stopped, .comfyUi) == "The job was stopped in ComfyUI.")
        #expect(sentence(.other) == "OpenAI isn't available right now.")
        #expect(Sentences.error(AutoPaperError.ProviderUnavailable(provider: .openAi, reason: .timedOut, detail: "x"))
            == "OpenAI took too long to answer. AutoPaper will try again later.")
    }

    @Test("A keyword the writing model kept breaking is named, in one line, with no promise to retry")
    func keywordNotFollowed() {
        #expect(Sentences.keywordNotFollowed("lighthouse", .must) == "The writing model kept leaving out “lighthouse”.")
        #expect(Sentences.keywordNotFollowed("people", .avoid) == "The writing model kept including “people”, which this mood avoids.")
        // Spaced as the engine stores keywords.
        #expect(Sentences.keywordNotFollowed("  old   harbour ", .must) == "The writing model kept leaving out “old harbour”.")
        // Where there's no link (Siri, Shortcuts) it says where to change it; whether or not it's retrying.
        for retrying in [true, false] {
            #expect(Sentences.error(AutoPaperError.KeywordNotFollowed(keyword: "lighthouse", weight: .must, moodId: "m"), retrying: retrying)
                == "The writing model kept leaving out “lighthouse”. Change it in Moods.")
        }
        #expect(Sentences.error(AutoPaperError.KeywordNotFollowed(keyword: "people", weight: .avoid, moodId: "m"))
            == "The writing model kept including “people”, which this mood avoids. Change it in Moods.")
    }

    @Test("Every input problem is worded by its reason, never by the core's English detail", arguments: [
        InvalidInputReason.keywordEmpty, .keywordTooLong, .tooManyKeywords, .duplicateKeyword, .addressMissing,
        .addressNotAllowed, .addressInvalid, .displaySizeInvalid, .noModels, .workflowNeedsPrompt,
        .workflowNotApiFormat, .workflowInvalid, .nothingToEcho, .moodNameEmpty, .moodNameTooLong,
        .duplicateMoodName, .lastMood, .other,
    ])
    func invalidInputIsWordedByReason(reason: InvalidInputReason) {
        let detail = "the concept schema rejected field 7"
        let error = AutoPaperError.InvalidInput(reason: reason, detail: detail)
        for inSettings in [false, true] {
            let sentence = Sentences.error(error, inSettings: inSettings)
            #expect(sentence?.isEmpty == false)
            #expect(sentence?.contains(detail) == false)
            #expect(sentence?.contains("!") == false)
            #expect(sentence == Sentences.invalidInput(reason, inSettings: inSettings))
        }
    }

    @Test func invalidInputSentences() {
        #expect(Sentences.invalidInput(.keywordTooLong) == "Keywords can be up to 40 characters.")
        #expect(Sentences.invalidInput(.duplicateKeyword) == "Another keyword already has this text.")
        #expect(Sentences.invalidInput(.workflowNeedsPrompt) == "That workflow needs a {{prompt}} placeholder where AutoPaper puts the scene.")
        // Mistakes of the app's own are said generically.
        #expect(Sentences.invalidInput(.displaySizeInvalid) == Sentences.internalProblem)
        #expect(Sentences.invalidInput(.other) == Sentences.internalProblem)
    }

    @Test("Away from Settings, a problem with a setting says where to fix it", arguments: [
        InvalidInputReason.addressMissing, .addressNotAllowed, .addressInvalid, .workflowNeedsPrompt,
        .workflowNotApiFormat, .workflowInvalid,
    ])
    func settingProblemsPointAtSettings(reason: InvalidInputReason) {
        let away = Sentences.error(AutoPaperError.InvalidInput(reason: reason, detail: "x"))
        let inSettings = Sentences.error(AutoPaperError.InvalidInput(reason: reason, detail: "x"), retrying: false, inSettings: true)
        #expect(away?.contains("Settings → Providers") == true)
        #expect(inSettings?.contains("Settings → Providers") == false)
        #expect(away != inSettings)
    }

    @Test func moodProblemsAreWorded() {
        #expect(Sentences.invalidInput(.moodNameEmpty) == "A mood needs a name.")
        #expect(Sentences.invalidInput(.moodNameTooLong) == "Mood names can be up to 40 characters.")
        #expect(Sentences.invalidInput(.duplicateMoodName) == "Another mood already has this name.")
        #expect(Sentences.invalidInput(.lastMood) == "There's always at least one mood, so the last one can't be deleted.")
    }

    @Test("A painting failure names the provider and model, promises no retry, and away from Settings says where to look")
    func paintingFailed() {
        let error = AutoPaperError.PaintingFailed(provider: .comfyUi, model: "Qwen-Image 2.1", detail: "the KSampler node (8) failed")
        #expect(Sentences.error(error, inSettings: true) == "ComfyUI couldn't paint with Qwen-Image 2.1.")
        #expect(Sentences.error(error) == "ComfyUI couldn't paint with Qwen-Image 2.1. Check it in Settings → Providers.")
        #expect(Sentences.error(error)?.contains("try again") == false)
        #expect(Sentences.error(error)?.contains("KSampler") == false)
        // An unknown model is left out rather than guessed.
        #expect(Sentences.error(AutoPaperError.PaintingFailed(provider: .comfyUi, model: "", detail: "x"), inSettings: true)
            == "ComfyUI couldn't paint.")
    }

    @Test func keywordProblemsReadTheSameEverywhere() {
        for reason in [InvalidInputReason.keywordEmpty, .keywordTooLong, .tooManyKeywords, .duplicateKeyword, .nothingToEcho,
                       .moodNameEmpty, .moodNameTooLong, .duplicateMoodName, .lastMood] {
            #expect(Sentences.invalidInput(reason, inSettings: false) == Sentences.invalidInput(reason, inSettings: true))
        }
    }

    @Test func noModelsTellsWhatToDo() {
        #expect(Sentences.invalidInput(.noModels, inSettings: true) == "The server has no models to use yet. Download one there, then choose Refresh.")
        #expect(Sentences.invalidInput(.noModels, inSettings: false) == "Your server has no models to use yet. Download one there and AutoPaper will use it.")
    }

    @Test func refusedAddressesAreQuotedWithTheReason() {
        #expect(Sentences.addressRefused("ftp://nas.local", .addressNotAllowed)
            == "“ftp://nas.local” wasn't saved. AutoPaper can't use that address. Use https, or http only for this Mac or your local network, with no user name or password in it.")
        #expect(Sentences.addressRefused("localhost", .addressInvalid)
            == "“localhost” wasn't saved. That isn't a complete web address. Include http:// or https:// and the server's name.")
    }

    @Test func onlyAddressReasonsBelongUnderTheAddressField() {
        for reason in [InvalidInputReason.addressMissing, .addressNotAllowed, .addressInvalid] {
            #expect(reason.isAboutAddress, "\(reason)")
        }
        for reason in [InvalidInputReason.keywordEmpty, .noModels, .workflowNeedsPrompt, .other] {
            #expect(!reason.isAboutAddress, "\(reason)")
        }
    }

    @Test func unsupportedIsWordedFromTheProviderNotTheJobString() {
        #expect(Sentences.error(AutoPaperError.Unsupported(provider: .ollama, job: "make images"))
            == "Ollama can't paint. Choose a painting provider in Settings → Providers.")
        #expect(Sentences.error(AutoPaperError.Unsupported(provider: .comfyUi, job: "write concepts"))
            == "ComfyUI can't write ideas. Choose a writing provider in Settings → Providers.")
    }

    @Test func cancelSaysNothing() {
        #expect(Sentences.error(AutoPaperError.Cancelled) == nil)
        #expect(Sentences.error(CancellationError() as any Error) == nil)
    }

    @Test func everyVariantHasWords() {
        let errors: [AutoPaperError] = [
            .InvalidKey(provider: .google), .Refused(provider: .openAi), .Unsupported(provider: .ollama, job: "make images"),
            .BudgetReached(budgetCents: 500), .Offline, .InvalidResponse(detail: "x"), .NotFound, .NothingToRevisit,
            .Storage(detail: "disk"), .Internal(detail: "x"), .PaintingFailed(provider: .comfyUi, model: "Krea 2 Turbo", detail: "x"),
        ]
        for error in errors {
            let sentence = Sentences.error(error)
            #expect(sentence?.isEmpty == false, "\(error)")
            #expect(sentence?.contains("!") == false, "no exclamation marks: \(error)")
        }
    }

    @Test func revisitReasonsHaveTheirOwnLine() {
        #expect(Sentences.revisit(.overBudget) == "This month's budget is spent. AutoPaper is bringing back wallpapers you liked.")
        #expect(Set([RevisitReason.requested, .overBudget, .offline, .providerFailed].map(Sentences.revisit)).count == 4)
    }
}

@Suite("Money and schedule lines")
struct FormattingTests {
    @Test func dollars() {
        #expect(Formatting.dollars(microUSD: 1_200_000) == "$1.20")
        #expect(Formatting.dollars(microUSD: 0) == "$0.00")
        #expect(Formatting.dollars(microUSD: 4_000) == "$0.004")
        #expect(Formatting.dollars(cents: 500) == "$5.00")
    }

    @Test func budgetLine() {
        #expect(Formatting.budgetLine(spentMicroUSD: 1_200_000, budgetCents: 500) == "$1.20 of $5.00 this month (estimated)")
        #expect(Formatting.budgetLine(spentMicroUSD: 1_200_000, budgetCents: nil) == "$1.20 this month (estimated)")
    }

    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }

    private let locale = Locale(identifier: "en_US")
    /// Monday 5 October 2026, 08:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_791_187_200)

    @Test func nextLineToday() {
        let due = now.addingTimeInterval(3_600)
        #expect(Formatting.nextLine(due: due, paused: false, cadence: .daily, now: now, calendar: calendar, locale: locale)
            == "Next new wallpaper at 9:00\u{202F}AM")
    }

    @Test func nextLineTomorrowAndLater() {
        #expect(Formatting.nextLine(due: now.addingTimeInterval(86_400), paused: false, cadence: .daily, now: now, calendar: calendar, locale: locale)
            == "Next new wallpaper tomorrow at 8:00\u{202F}AM")
        #expect(Formatting.nextLine(due: now.addingTimeInterval(3 * 86_400), paused: false, cadence: .weekly, now: now, calendar: calendar, locale: locale)
            == "Next new wallpaper on Thursday at 8:00\u{202F}AM")
    }

    @Test func nextLinePausedManualAndDue() {
        #expect(Formatting.nextLine(due: now, paused: true, cadence: .daily, now: now) == "Paused")
        #expect(Formatting.nextLine(due: nil, paused: false, cadence: .manual, now: now) == "Only when you ask")
        #expect(Formatting.nextLine(due: now.addingTimeInterval(-60), paused: false, cadence: .hourly, now: now) == "Next new wallpaper soon")
    }

    @Test("Before scheduled wallpapers start, the due time isn't promised")
    func nextLineBeforeTheScheduleStarts() {
        // An empty library is due "now" in the engine, but the app's timer isn't armed yet.
        #expect(Formatting.nextLine(due: now, paused: false, cadence: .daily, armed: false, now: now)
            == "New wallpapers start after you make your first one")
        #expect(Formatting.nextLine(due: now.addingTimeInterval(3_600), paused: false, cadence: .hourly, armed: false, now: now)
            == Formatting.notStartedLine)
        // Paused and Only When I Ask are said first, armed or not.
        #expect(Formatting.nextLine(due: now, paused: true, cadence: .daily, armed: false, now: now) == "Paused")
        #expect(Formatting.nextLine(due: nil, paused: false, cadence: .manual, armed: false, now: now) == "Only when you ask")
    }
}

@Suite("Spoken descriptions and keywords")
struct SpokenAndKeywordTests {
    @Test func ratingIsAddedInAutoPapersWords() {
        #expect(Spoken.wallpaper(description: "Harbour at Dusk. Boats in the rain.", rating: .liked) == "Harbour at Dusk. Boats in the rain. Liked.")
        #expect(Spoken.wallpaper(description: "Harbour at Dusk", rating: .disliked) == "Harbour at Dusk. Disliked.")
        #expect(Spoken.wallpaper(description: "Harbour at Dusk.", rating: .unrated) == "Harbour at Dusk.")
    }

    @Test func duplicatesAreFoundCaseInsensitivelyAfterNormalising() {
        let keywords = [Keyword(id: "1", text: "Rainy night", weight: .avoid, position: 0, createdAt: 0)]
        #expect(KeywordText.duplicate(of: "  rainy   NIGHT ", in: keywords)?.id == "1")
        #expect(KeywordText.duplicate(of: "rain", in: keywords) == nil)
        #expect(KeywordText.normalised("  a \t b  ") == "a b")
    }

    @Test func storageChoicesReadAsSizes() {
        #expect(Choices.storageLimitsMB.map(Choices.storageTitle) == ["256 MB", "512 MB", "1 GB", "2 GB", "5 GB"])
    }
}

@Suite("The core's defaults")
struct CoreDefaultTests {
    @Test func modelPickerNamesTheDefault() {
        #expect(ProviderKind.openAi.defaultModelTitle(job: .concepts) == "Default (\(defaultModel(kind: .openAi, job: .concepts)))")
        #expect(!defaultModel(kind: .openAi, job: .concepts).isEmpty)
        #expect(ProviderKind.ollama.defaultModelTitle(job: .concepts) == "Default (the server's first model)")
        #expect(ProviderKind.openAiCompatible.defaultModelTitle(job: .images) == "Default (the server's first model)")
        #expect(ProviderKind.comfyUi.defaultModelTitle(job: .images, ownWorkflow: true) == "Default (your workflow's model)")
        // AutoPaper's ComfyUI workflows name their models: "Z-Image Turbo (default)", from the list, or from
        // AutoPaper's own names while the server can't be asked.
        #expect(ProviderKind.comfyUi.defaultModelTitle(job: .images) == "Z-Image Turbo (default)")
        let listed = [ModelInfo(id: defaultModel(kind: .comfyUi, job: .images), displayName: "Z-Image Turbo")]
        #expect(ProviderKind.comfyUi.defaultModelTitle(job: .images, models: listed) == "Z-Image Turbo (default)")
    }

    @Test func addressesComeFromTheCore() {
        #expect(ProviderKind.ollama.defaultAddress == defaultBaseUrl(kind: .ollama))
        #expect(ProviderKind.comfyUi.defaultAddress == "http://127.0.0.1:8188")
        #expect(ProviderKind.openAiCompatible.defaultAddress == nil)
        #expect(ProviderKind.openAi.defaultAddress == nil)
    }

    @Test func pricesDateReadsAsADate() {
        #expect(Formatting.pricesDate("2026-10-05", locale: Locale(identifier: "en_US")) == "October 5, 2026")
        #expect(!Formatting.pricesDate().contains("-"))
    }
}

@Suite("History item actions")
@MainActor
struct HistoryActionTests {
    private func generation(rating: Rating = .unrated, image: String? = "/tmp/a.jpg", thumb: String? = "/tmp/a-thumb.jpg") -> Generation {
        let concept = Concept(
            title: "Harbour at Dusk", summary: "Boats in the rain.", setting: "", subject: "", elements: [], timeOfDay: "",
            weather: "", season: "", mood: [], palette: [], style: "", composition: "", keywordsUsed: [], wildcards: [], prompt: ""
        )
        return Generation(
            id: "g1", createdAt: 0, trigger: .manual, status: .ok, concept: concept, imagePath: image, thumbPath: thumb,
            width: 1920, height: 1080, rating: rating, echoOf: nil, echoNote: nil, textProvider: .demo, textModel: "demo",
            imageProvider: .demo, imageModel: "demo", surprise: 0.35, keywords: [], costMicrousd: 0, lastShownAt: nil,
            shownCount: 0, error: nil, moodId: "m1", moodName: "Rainy beach"
        )
    }

    private func actions(canMake: Bool = true) -> HistoryActions {
        HistoryActions(showOnDesktop: { _ in }, rate: { _, _ in }, makeEcho: { _ in }, showLineage: { _ in },
                       showInFinder: { _ in }, delete: { _ in }, canMake: canMake)
    }

    private func titles(_ groups: [[HistoryActions.Item]]) -> [String] {
        groups.joined().map(\.title)
    }

    @Test func originalAndEchoesOnlyWhenThereAreSome() {
        let lone = titles(actions().groups(for: generation(), hasEchoes: false))
        #expect(!lone.contains("Show Original and Echoes"))
        #expect(lone == ["Show on Desktop", "Like", "Dislike", "Make an Echo", "Show in Finder", "Delete…"])
        let related = titles(actions().groups(for: generation(), hasEchoes: true))
        #expect(related == ["Show on Desktop", "Like", "Dislike", "Make an Echo", "Show Original and Echoes", "Show in Finder", "Delete…"])
    }

    @Test func ratingItemsAreCheckmarkedAndSpokenAsWhatTheyDo() {
        let liked = actions().groups(for: generation(rating: .liked), hasEchoes: false).joined()
        let like = liked.first { $0.title == "Like" }, dislike = liked.first { $0.title == "Dislike" }
        #expect(like?.isOn == true && like?.spokenTitle == "Remove Like")
        #expect(dislike?.isOn == false && dislike?.spokenTitle == "Dislike")
    }

    @Test func showOnDesktopNeedsTheImageAndAFreeEngine() {
        func showEnabled(_ actions: HistoryActions, _ generation: Generation) -> Bool? {
            actions.groups(for: generation, hasEchoes: false).joined().first { $0.title == "Show on Desktop" }?.isEnabled
        }
        #expect(showEnabled(actions(), generation()) == true)
        #expect(showEnabled(actions(canMake: false), generation()) == false)
        // Pruned by the storage limit: only the thumbnail is left, so there's nothing to put on the desktop.
        #expect(showEnabled(actions(), generation(image: nil)) == false)
        let finder = actions().groups(for: generation(image: nil), hasEchoes: false).joined().first { $0.title == "Show in Finder" }
        #expect(finder?.isEnabled == true)
    }

    @Test func deleteIsLastAndDestructive() {
        let last = actions().groups(for: generation(), hasEchoes: true).last
        #expect(last?.count == 1 && last?.first?.isDestructive == true && last?.first?.spokenTitle == "Delete")
    }
}

@Suite("Key problems are one link to the key")
struct KeyProblemTests {
    @Test("A missing key links to adding it, under the provider's Keychain account")
    func missing() throws {
        let problem = try #require(KeyProblem(AutoPaperError.MissingKey(provider: .openAi)))
        #expect(problem.linkTitle == "Add your OpenAI key")
        #expect(problem.account == "openai.api_key")
        #expect(!problem.refused)
    }

    @Test("A refused key links to checking it")
    func refused() throws {
        let problem = try #require(KeyProblem(AutoPaperError.InvalidKey(provider: .google)))
        #expect(problem.linkTitle.hasPrefix("Check your "))
        #expect(problem.refused)
        #expect(problem.account == "google.api_key")
    }

    @Test("An OpenAI-compatible server's key is the one for its own address")
    func compatibleServer() throws {
        let selection = ProviderSelection(kind: .openAiCompatible, model: "", baseUrl: "http://192.168.1.5:1234/v1")
        let problem = try #require(KeyProblem(AutoPaperError.MissingKey(provider: .openAiCompatible), selection: selection))
        #expect(problem.linkTitle == "Add your server's key")
        #expect(problem.account == "openai_compatible.api_key@http://192.168.1.5:1234")
    }

    @Test("Other errors aren't key problems")
    func others() {
        #expect(KeyProblem(AutoPaperError.Offline) == nil)
        #expect(KeyProblem(AutoPaperError.NotFound) == nil)
    }
}

@Suite("A problem with a setting is one link to the fix (spec 6a)")
struct SettingsProblemTests {
    private let demo = ProviderSelection(kind: .demo, model: "", baseUrl: nil)

    private func problem(_ error: AutoPaperError, writing: ProviderSelection? = nil, painting: ProviderSelection? = nil,
                         ownWorkflow: Bool = false) -> SettingsProblem? {
        SettingsProblem(error, writing: writing ?? demo, painting: painting ?? demo, ownWorkflow: ownWorkflow)
    }

    @Test("A key is the link alone, and what's said is what the link says")
    func key() throws {
        let openAI = ProviderSelection(kind: .openAi, model: "", baseUrl: nil)
        let refused = try #require(problem(.InvalidKey(provider: .openAi), writing: openAI))
        #expect(refused.sentence.isEmpty)
        #expect(refused.linkTitle == "Check your OpenAI key")
        #expect(refused.place == .accounts(account: "openai.api_key"))
        #expect(refused.spoken == "OpenAI didn't accept your key. Check your OpenAI key.")
        #expect(!refused.spoken.contains("Settings →"))
        let missing = try #require(problem(.MissingKey(provider: .google)))
        #expect(missing.spoken == "Google Gemini needs a key. Add your Google Gemini key.")
        // An OpenAI-compatible server's key is its own address's.
        let server = ProviderSelection(kind: .openAiCompatible, model: "", baseUrl: "http://192.168.1.5:1234/v1")
        #expect(problem(.MissingKey(provider: .openAiCompatible), painting: server)?.place
            == .accounts(account: "openai_compatible.api_key@http://192.168.1.5:1234"))
    }

    @Test("A spent budget links to raising it")
    func budget() throws {
        let spent = try #require(problem(.BudgetReached(budgetCents: 500)))
        #expect(spent.sentence == "This month's budget is spent. New wallpapers start again next month.")
        #expect(spent.linkTitle == "Raise the budget")
        #expect(spent.place == .budget)
        #expect(SettingsProblem.overBudget(bringingBack: true).sentence == Sentences.revisit(.overBudget))
    }

    @Test("A provider that can't do the job links to choosing one that can")
    func unsupported() throws {
        let ollama = try #require(problem(.Unsupported(provider: .ollama, job: "make images")))
        #expect(ollama.sentence == "Ollama can't paint.")
        #expect(ollama.linkTitle == "Choose a painting provider")
        #expect(ollama.place == .providers(job: .images, address: false))
        let comfy = try #require(problem(.Unsupported(provider: .comfyUi, job: "write concepts")))
        #expect(comfy.linkTitle == "Choose a writing provider")
        #expect(comfy.place == .providers(job: .concepts, address: false))
    }

    @Test("A server address problem links to that section's address field", arguments: [
        InvalidInputReason.addressMissing, .addressNotAllowed, .addressInvalid,
    ])
    func address(reason: InvalidInputReason) throws {
        let compatible = ProviderSelection(kind: .openAiCompatible, model: "", baseUrl: reason == .addressMissing ? nil : "ftp://nas.local")
        let found = try #require(problem(.InvalidInput(reason: reason, detail: "x"), painting: compatible))
        #expect(found.place == .providers(job: .images, address: true))
        #expect(!found.sentence.contains("Settings"))
        #expect(found.linkTitle == (reason == .addressMissing ? "Enter the server address" : "Check the server address"))
    }

    @Test("A painting problem keeps its line and link")
    func painting() throws {
        let comfy = ProviderSelection(kind: .comfyUi, model: "", baseUrl: nil)
        let found = try #require(problem(.PaintingFailed(provider: .comfyUi, model: "Krea 2 Turbo", detail: "x"), painting: comfy))
        #expect(found.sentence == "ComfyUI couldn't paint with Krea 2 Turbo.")
        #expect(found.linkTitle == "Check ComfyUI in Settings")
        #expect(found.place == .providers(job: .images, address: false))
        #expect(found.spoken == "ComfyUI couldn't paint with Krea 2 Turbo. Check ComfyUI in Settings.")
    }

    @Test("A keyword the writing model kept breaking is one line naming it, linked to its mood")
    func keywordNotFollowed() throws {
        let openAI = ProviderSelection(kind: .openAi, model: "", baseUrl: nil)
        let must = try #require(problem(.KeywordNotFollowed(keyword: "lighthouse", weight: .must, moodId: "mood-1"), writing: openAI))
        #expect(must.sentence == "The writing model kept leaving out “lighthouse”.")
        #expect(must.linkTitle == "Edit the keyword")
        #expect(must.place == .mood(id: "mood-1", keyword: "lighthouse"))
        #expect(must.spoken == "The writing model kept leaving out “lighthouse”. Edit the keyword.")
        let avoid = try #require(problem(.KeywordNotFollowed(keyword: "people", weight: .avoid, moodId: "mood-2"), writing: openAI))
        #expect(avoid.sentence == "The writing model kept including “people”, which this mood avoids.")
        #expect(avoid.place == .mood(id: "mood-2", keyword: "people"))
        // Said once: the line names no place, and nothing about trying again (asking again rarely helps).
        for found in [must, avoid] {
            #expect(!found.sentence.contains("Settings"))
            #expect(!found.sentence.contains("Moods"))
            #expect(!found.spoken.contains("try again"))
        }
    }

    @Test("Passing problems and mistakes of AutoPaper's own are words alone")
    func notSettings() {
        #expect(problem(.Offline) == nil)
        #expect(problem(.ProviderUnavailable(provider: .comfyUi, reason: .notRunning, detail: "x")) == nil)
        #expect(problem(.InvalidInput(reason: .other, detail: "x")) == nil)
        #expect(problem(.InvalidInput(reason: .noModels, detail: "x")) == nil)
        #expect(problem(.Cancelled) == nil)
    }
}

@Suite("ComfyUI's model menu")
struct ModelMenuTests {
    private let zImage = ModelInfo(id: "z_image_turbo_bf16.safetensors", displayName: "Z-Image Turbo")
    private let krea = ModelInfo(id: "krea2_turbo_fp8_scaled.safetensors", displayName: "Krea 2 Turbo")
    private let qwen = ModelInfo(id: "qwen_image_2.1_int8_convrot.safetensors", displayName: "Qwen-Image 2.1")

    private func titles(_ models: [ModelInfo], saved: String = "", listed: Bool = true) -> [String] {
        ModelMenu.items(kind: .comfyUi, job: .images, models: models, saved: saved, listed: listed).map(\.title)
    }

    @Test("The default first, then the others by name, each in words")
    func readableTitles() {
        #expect(titles([qwen, zImage, krea]) == ["Z-Image Turbo (default)", "Krea 2 Turbo", "Qwen-Image 2.1"])
        let items = ModelMenu.items(kind: .comfyUi, job: .images, models: [zImage, krea, qwen], saved: "", listed: true)
        #expect(items.map(\.tag) == ["", krea.id, qwen.id])
    }

    @Test("A saved model the server no longer offers is marked, not offered")
    func unavailableModel() {
        let saved = "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot.safetensors"
        #expect(titles([zImage, krea], saved: saved)
            == ["Z-Image Turbo (default)", "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot (not available)", "Krea 2 Turbo"])
        // Before ComfyUI has answered (not running), it's what's saved, in words where AutoPaper knows the model.
        #expect(titles([], saved: saved, listed: false)
            == ["Z-Image Turbo (default)", "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot"])
        #expect(titles([], saved: krea.id, listed: false) == ["Z-Image Turbo (default)", "Krea 2 Turbo"])
        #expect(titles([zImage], saved: qwen.id) == ["Z-Image Turbo (default)", "Qwen-Image 2.1 (not available)"])
    }

    @Test("AutoPaper's names for its own workflows' models match the core's workflow files")
    func bundledNamesMatchTheCore() throws {
        // core/resources/comfyui: each *.map.json names its workflow; the workflow's UNETLoader loads the model file.
        let folder = URL(filePath: #filePath).deletingLastPathComponent()
            .appending(path: "../../../core/resources/comfyui").standardizedFileURL
        let maps = try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
            .filter { $0.lastPathComponent.hasSuffix(".map.json") }
        var found: [String: String] = [:]
        for map in maps {
            let mapJSON = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: map)) as? [String: Any])
            let name = try #require(mapJSON["name"] as? String)
            let template = folder.appending(path: try #require(mapJSON["template"] as? String))
            let text = try String(contentsOf: template, encoding: .utf8)
            let model = try #require(text.firstMatch(of: /"unet_name":\s*"([^"]+)"/)?.1)
            found[String(model)] = name
        }
        #expect(found == BundledWorkflows.names)
        // The core's default is one of them.
        #expect(BundledWorkflows.names[defaultModel(kind: .comfyUi, job: .images)] == "Z-Image Turbo")
    }

    @Test("Other providers keep their name and id")
    func otherProviders() {
        let info = ModelInfo(id: "llama3.3:70b", displayName: "Llama 3.3 70B")
        #expect(ModelMenu.title(of: info, kind: .ollama) == "Llama 3.3 70B (llama3.3:70b)")
        #expect(ModelMenu.title(of: ModelInfo(id: "gpt-6-luna", displayName: "gpt-6-luna"), kind: .openAi) == "gpt-6-luna")
    }

    @Test func workflowChoiceTitles() {
        #expect(WorkflowChoice.autoPaperTitle == "AutoPaper's")
        #expect(WorkflowChoice.chooseTitle(hasFile: false) == "Your Own…")
        #expect(WorkflowChoice.chooseTitle(hasFile: true) == "Choose Another…")
        #expect(WorkflowChoice.ownTitle(fileName: "flux-dev-api.json") == "flux-dev-api.json")
        #expect(WorkflowChoice.ownTitle(fileName: nil) == "Your Own Workflow")
    }
}

@Suite("A painting problem is one line and one link")
struct PaintingProblemTests {
    private let qwen = ProviderSelection(kind: .comfyUi, model: "qwen_image_2.1_int8_convrot.safetensors", baseUrl: nil)

    @Test("PaintingFailed names the model the engine names, never the detail, and links to ComfyUI's settings")
    func couldntPaint() throws {
        let failed = AutoPaperError.PaintingFailed(provider: .comfyUi, model: "Qwen-Image 2.1",
                                                   detail: "ComfyUI couldn't paint with Qwen-Image 2.1: the KSampler node (8) failed")
        let problem = try #require(PaintingProblem(failed, painting: qwen, ownWorkflow: false))
        #expect(problem.kind == .couldntPaint)
        #expect(problem.sentence == "ComfyUI couldn't paint with Qwen-Image 2.1.")
        #expect(problem.linkTitle == "Check ComfyUI in Settings")
        #expect(!problem.sentence.contains("try again"))
        #expect(!problem.sentence.contains("KSampler"))
        // No model known: said without one. A person's own workflow: the engine names its model file.
        #expect(PaintingProblem(AutoPaperError.PaintingFailed(provider: .comfyUi, model: "", detail: "x"), painting: qwen, ownWorkflow: false)?.sentence
            == "ComfyUI couldn't paint.")
        #expect(PaintingProblem(AutoPaperError.PaintingFailed(provider: .comfyUi, model: "flux1-dev", detail: "x"), painting: qwen, ownWorkflow: true)?.sentence
            == "ComfyUI couldn't paint with flux1-dev.")
    }

    @Test("Typed, whatever the stage: no guessing from where the run stopped")
    func typedNotGuessed() throws {
        // A painting failure is one whichever painter is chosen now (the error says which provider).
        let openAI = ProviderSelection(kind: .openAi, model: "", baseUrl: nil)
        let other = AutoPaperError.PaintingFailed(provider: .openAiCompatible, model: "sd-3.5", detail: "x")
        let problem = try #require(PaintingProblem(other, painting: openAI, ownWorkflow: false))
        #expect(problem.sentence == "Your OpenAI-compatible server couldn't paint with sd-3.5.")
        #expect(problem.linkTitle == "Check OpenAI-compatible in Settings")
        // An unusable idea (the writer's answer) isn't a painting problem, and neither is a passing one.
        #expect(PaintingProblem(AutoPaperError.InvalidResponse(detail: "x"), painting: qwen, ownWorkflow: false) == nil)
        let notRunning = AutoPaperError.ProviderUnavailable(provider: .comfyUi, reason: .notRunning, detail: "x")
        #expect(PaintingProblem(notRunning, painting: qwen, ownWorkflow: false) == nil)
        let stalled = AutoPaperError.ProviderUnavailable(provider: .comfyUi, reason: .timedOut, detail: "stalled")
        #expect(PaintingProblem(stalled, painting: qwen, ownWorkflow: false) == nil)
        #expect(PaintingProblem(AutoPaperError.Cancelled, painting: qwen, ownWorkflow: false) == nil)
        #expect(PaintingProblem(AutoPaperError.InvalidInput(reason: .other, detail: "x"), painting: qwen, ownWorkflow: false) == nil)
    }

    @Test("A person's workflow that can't be used links to its row, without directions", arguments: [
        InvalidInputReason.workflowNeedsPrompt, .workflowNotApiFormat, .workflowInvalid,
    ])
    func ownWorkflow(reason: InvalidInputReason) throws {
        let error = AutoPaperError.InvalidInput(reason: reason, detail: "x")
        let painting = ProviderSelection(kind: .comfyUi, model: "", baseUrl: nil)
        let problem = try #require(PaintingProblem(error, painting: painting, ownWorkflow: true))
        #expect(problem.sentence.hasPrefix("Your ComfyUI workflow "))
        #expect(!problem.sentence.contains("Settings"))
        #expect(problem.linkTitle == "Check ComfyUI in Settings")
        #expect(PaintingProblem(error, painting: painting, ownWorkflow: false) == nil)
        #expect(PaintingProblem(error, painting: ProviderSelection(kind: .openAi, model: "", baseUrl: nil), ownWorkflow: true) == nil)
    }
}

@Suite("Reading a person's own ComfyUI workflow")
struct OwnWorkflowTests {
    @Test("The model its first loader loads, as the core finds it")
    func model() {
        let checkpoint = #"{"3": {"class_type": "KSampler", "inputs": {"seed": {{seed}}}}, "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "sd_xl_base_1.0.safetensors"}}, "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "masterpiece, {{prompt}}"}}, "5": {"class_type": "EmptyLatentImage", "inputs": {"width": "{{width}}", "height": {{height}}}}}"#
        #expect(OwnWorkflow.read(checkpoint) == .model("sd_xl_base_1.0.safetensors"))
        #expect(OwnWorkflow.modelLine(OwnWorkflow.read(checkpoint)) == "sd_xl_base_1.0 (from your workflow)")
        let unet = #"{"prompt": {"10": {"class_type": "UNETLoader", "inputs": {"unet_name": "qwen_image_2.1_int8_convrot.safetensors"}}, "4": {"class_type": "TextEncodeQwenImage21", "inputs": {"prompt": "{{prompt}}"}}}, "client_id": "x"}"#
        #expect(OwnWorkflow.modelLine(OwnWorkflow.read(unet)) == "qwen_image_2.1_int8_convrot (from your workflow)")
        let none = #"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}"#
        #expect(OwnWorkflow.read(none) == .model(nil))
        #expect(OwnWorkflow.modelLine(OwnWorkflow.read(none)) == "Set in your workflow")
    }

    @Test("What the core would refuse is refused when the file is chosen")
    func problems() {
        #expect(OwnWorkflow.read(#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "fixed"}}}"#) == .problem(.workflowNeedsPrompt))
        #expect(OwnWorkflow.read(#"{"last_node_id": 9, "nodes": [{"id": 1, "widgets_values": ["{{prompt}}"]}], "links": []}"#)
            == .problem(.workflowNotApiFormat))
        #expect(OwnWorkflow.read(#"{"text": "{{prompt}}"}"#) == .problem(.workflowNotApiFormat))
        #expect(OwnWorkflow.read(#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": {{prompt}}}}}"#) == .problem(.workflowInvalid))
    }

    @Test func plainNames() {
        #expect(OwnWorkflow.plainName("krea2_turbo_fp8_scaled.safetensors") == "krea2_turbo_fp8_scaled")
        #expect(OwnWorkflow.plainName("flux/dev.gguf") == "flux/dev")
        #expect(OwnWorkflow.plainName("custom") == "custom")
    }
}

@Suite("Moods in words")
struct MoodTextTests {
    private func keyword(_ text: String, _ weight: KeywordWeight = .must) -> Keyword {
        Keyword(id: text, text: text, weight: weight, position: 0, createdAt: 0)
    }

    @Test("New Mood picks a name no mood has, ignoring case and spacing")
    func newMoodNames() {
        #expect(MoodText.unique(MoodText.newMoodName, among: []) == "New mood")
        #expect(MoodText.unique("New mood", among: ["Rainy beach", "new  MOOD"]) == "New mood 2")
        #expect(MoodText.unique("New mood", among: ["New mood", "New mood 2"]) == "New mood 3")
    }

    @Test("Duplicate adds “copy”, and every name fits the engine's 40 characters")
    func copyNames() {
        #expect(MoodText.copyName(of: "Rainy beach", among: ["Rainy beach"]) == "Rainy beach copy")
        #expect(MoodText.copyName(of: "Rainy beach", among: ["Rainy beach", "Rainy beach copy"]) == "Rainy beach copy 2")
        let long = String(repeating: "a", count: 40)
        let copy = MoodText.copyName(of: long, among: [long])
        #expect(copy.count <= MoodText.maxNameLength)
        #expect(copy.hasSuffix(" copy"))
        let names = (1...12).map { _ in "x" } .enumerated().map { "\(String(repeating: "b", count: 38)) \($0.offset + 1)" }
        #expect(MoodText.unique(String(repeating: "b", count: 40), among: names).count <= MoodText.maxNameLength)
    }

    @Test("Keywords in short: in order, an Avoid as what's left out")
    func summary() {
        let keywords = [keyword("rain"), keyword("beach", .maybe), keyword("night"), keyword("people", .avoid)]
        #expect(MoodText.summary(keywords) == "rain · beach · night · no people")
        #expect(MoodText.spokenSummary(keywords) == "rain, beach, night, no people")
        #expect(MoodText.summary([]) == "No keywords yet")
    }

    @Test("The current mood is said, not shown by colour alone")
    func spokenName() {
        let current = Mood(id: "1", name: "Rainy beach", position: 0, surprise: 0.35, createdAt: 0, keywords: [], active: true)
        var other = current
        other.active = false
        #expect(MoodText.spokenName(current) == "Rainy beach, current mood")
        #expect(MoodText.spokenName(other) == "Rainy beach")
    }

    @Test("A mood's row is spoken as one line: its name, whether it's current, its keywords")
    func spokenRow() {
        var current = Mood(id: "1", name: "Rainy beach", position: 0, surprise: 0.35, createdAt: 0,
                           keywords: [keyword("rain"), keyword("beach"), keyword("people", .avoid)], active: true)
        #expect(MoodText.spokenRow(current) == "Rainy beach, current mood, rain, beach, no people")
        current.active = false
        current.keywords = []
        #expect(MoodText.spokenRow(current) == "Rainy beach, No keywords yet")
    }

    @Test("Reordering lands where the engine will put it")
    func reorder() {
        // List's onMove gives the gap: dropping the first of four into gap 3 puts it third.
        #expect(Reorder.position(from: 0, droppedAt: 3) == 2)
        #expect(Reorder.position(from: 3, droppedAt: 0) == 0)
        #expect(Reorder.moved(["a", "b", "c", "d"], from: 0, to: 2) == ["b", "c", "a", "d"])
        #expect(Reorder.moved(["a", "b", "c"], from: 2, to: 0) == ["c", "a", "b"])
        #expect(Reorder.moved(["a", "b"], from: 0, to: 9) == ["b", "a"])
        #expect(Reorder.moved(["a", "b"], from: 5, to: 0) == ["a", "b"])
    }
}

@Suite("Progress and estimates in words")
struct ProgressWordingTests {
    @Test("Rough durations don't flicker: seconds by fives, then whole minutes, then hours", arguments: [
        (3, "a few seconds"), (12, "about 10 seconds"), (43, "about 45 seconds"), (57, "about 55 seconds"),
        (58, "about 1 minute"), (89, "about 1 minute"), (91, "about 2 minutes"), (540, "about 9 minutes"),
        (5_399, "about 90 minutes"), (5_400, "about 2 hours"), (3 * 3_600, "about 3 hours"),
    ] as [(UInt32, String)])
    func roughly(seconds: UInt32, words: String) {
        #expect(Formatting.roughly(seconds: seconds) == words)
    }

    @Test func timeLeftOnlyWithARealNumber() {
        #expect(Formatting.timeLeft(seconds: nil) == nil)
        #expect(Formatting.timeLeft(seconds: 0) == "Almost done")
        #expect(Formatting.timeLeft(seconds: 360) == "About 6 minutes left")
        #expect(Formatting.timeLeft(seconds: 20) == "About 20 seconds left")
    }

    @Test("The capsule: a ring only with a real fraction, Cancel only while making one")
    func workPresentation() {
        let painting = WorkPresentation(stage: "Painting…", fraction: 0.4, secondsLeft: 360, generating: true)
        #expect(painting.fraction == 0.4)
        #expect(painting.timeLeft == "About 6 minutes left")
        #expect(painting.spokenProgress == "40 percent, about 6 minutes left")
        #expect(painting.menuLine == "Painting… (about 6 minutes left)")
        #expect(painting.cancellable)
        let composing = WorkPresentation(stage: "Composing an idea…", fraction: nil, secondsLeft: nil, generating: true)
        #expect(composing.fraction == nil && composing.spokenProgress == nil && composing.menuLine == "Composing an idea…")
        // Show on Desktop: already made, nothing to cancel.
        let showing = WorkPresentation(stage: nil, fraction: nil, secondsLeft: nil, generating: false)
        #expect(!showing.cancellable)
        #expect(showing.stage == "Preparing for your displays…")
        #expect(WorkPresentation(stage: nil, fraction: 1.7, secondsLeft: nil, generating: true).fraction == 1)
    }

    @Test("Where a provider's timings come from")
    func places() {
        func place(_ kind: ProviderKind, _ url: String? = nil) -> Formatting.Place {
            Formatting.Place(ProviderSelection(kind: kind, model: "", baseUrl: url))
        }
        #expect(place(.comfyUi) == .thisMac)
        #expect(place(.ollama, "http://localhost:11434") == .thisMac)
        #expect(place(.comfyUi, "http://studio.local:8188") == .host("studio.local"))
        #expect(place(.openAiCompatible, "http://192.168.1.5:1234/v1") == .host("192.168.1.5"))
        #expect(place(.openAiCompatible, "https://api.example.com/v1") == .hosted)
        #expect(place(.demo) == .thisMac)
        #expect(place(.openAi) == .hosted)
    }

    @Test("Estimates: per wallpaper or per idea, only once AutoPaper has timed one")
    func estimates() {
        #expect(Formatting.estimateLine(seconds: 540, job: .images, place: .thisMac) == "About 9 minutes per wallpaper on this Mac")
        #expect(Formatting.estimateLine(seconds: 4, job: .concepts, place: .host("studio.local")) == "A few seconds per idea on studio.local")
        #expect(Formatting.estimateLine(seconds: 40, job: .images, place: .hosted) == "About 40 seconds per wallpaper")
        #expect(Formatting.estimateLine(seconds: nil, job: .images, place: .thisMac) == nil)
        // The writer hasn't been timed yet: the painting alone isn't passed off as a whole wallpaper.
        #expect(Formatting.estimateLine(seconds: 20, job: .images, place: .thisMac, includesWriting: false) == "About 20 seconds to paint on this Mac")
        #expect(Formatting.estimateLine(seconds: 4, job: .concepts, place: .thisMac, includesWriting: false) == "A few seconds per idea on this Mac")
        #expect(Formatting.withEstimate("Qwen-Image 2.1", seconds: 540) == "Qwen-Image 2.1 (about 9 minutes)")
        #expect(Formatting.withEstimate("Z-Image Turbo (default)", seconds: 30) == "Z-Image Turbo (default, about 30 seconds)")
        #expect(Formatting.withEstimate("Krea 2 Turbo", seconds: nil) == "Krea 2 Turbo")
    }
}

@Suite("Dock, desktop, schedule and menu decisions")
struct DecisionTests {
    @Test("There's always a way in: the Dock while a window is open or the menu bar item is hidden")
    func dock() {
        #expect(!DockPolicy.needsDock(openWindows: 0, showInMenuBar: true))
        #expect(DockPolicy.needsDock(openWindows: 1, showInMenuBar: true))
        #expect(DockPolicy.needsDock(openWindows: 0, showInMenuBar: false))
    }

    @Test("Only AutoPaper's own renders, nothing, or a missing file may be replaced")
    func mayReplace() {
        let renders = URL(filePath: "/tmp/AutoPaper/renders", directoryHint: .isDirectory)
        let ours = URL(filePath: "/tmp/AutoPaper/renders/g1-2560x1600.jpg")
        let theirs = URL(filePath: "/Users/someone/Pictures/beach.heic")
        #expect(DesktopRules.mayReplace(picture: nil, fileExists: false, rendersFolder: renders))
        #expect(DesktopRules.mayReplace(picture: theirs, fileExists: false, rendersFolder: renders))
        #expect(DesktopRules.mayReplace(picture: ours, fileExists: true, rendersFolder: renders))
        #expect(!DesktopRules.mayReplace(picture: theirs, fileExists: true, rendersFolder: renders))
        // A folder whose name starts the same isn't inside it.
        #expect(!DesktopRules.isRender(URL(filePath: "/tmp/AutoPaper/renders-old/x.jpg"), in: renders))
        #expect(!DesktopRules.mayReplace(picture: ours, fileExists: true, rendersFolder: nil))
    }

    @Test("The timer starts early by the estimate, but never spins")
    func timer() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let due = now.addingTimeInterval(3_600), start = now.addingTimeInterval(3_000)
        #expect(ScheduleRules.timerDate(armed: true, nextStart: start, nextDue: due, holdUntil: nil, lastAttempt: nil, minimumGap: 60) == start)
        #expect(ScheduleRules.timerDate(armed: true, nextStart: nil, nextDue: due, holdUntil: nil, lastAttempt: nil, minimumGap: 60) == due)
        #expect(ScheduleRules.timerDate(armed: false, nextStart: start, nextDue: due, holdUntil: nil, lastAttempt: nil, minimumGap: 60) == nil)
        #expect(ScheduleRules.timerDate(armed: true, nextStart: nil, nextDue: nil, holdUntil: nil, lastAttempt: nil, minimumGap: 60) == nil)
        // Overdue right after launch: not before the hold, nor within a minute of the last attempt.
        let past = now.addingTimeInterval(-600)
        #expect(ScheduleRules.timerDate(armed: true, nextStart: past, nextDue: past, holdUntil: now.addingTimeInterval(15), lastAttempt: nil, minimumGap: 60)
            == now.addingTimeInterval(15))
        #expect(ScheduleRules.timerDate(armed: true, nextStart: past, nextDue: past, holdUntil: nil, lastAttempt: now, minimumGap: 60)
            == now.addingTimeInterval(60))
    }

    @Test("What's on My Desktop doesn't describe a wallpaper that isn't there")
    func desktopAnswer() {
        #expect(DesktopAnswer.text(description: "Harbour at Dusk. Boats in the rain.", title: "Harbour at Dusk", ownPictureShowing: false)
            == "Harbour at Dusk. Boats in the rain.")
        #expect(DesktopAnswer.text(description: "Harbour at Dusk. Boats in the rain.", title: "Harbour at Dusk", ownPictureShowing: true)
            == "Your own picture is on the desktop. AutoPaper's last wallpaper was “Harbour at Dusk”.")
        #expect(DesktopAnswer.text(description: nil, title: nil, ownPictureShowing: false) == "There's no AutoPaper wallpaper on the desktop yet.")
    }

    @Test("Show wallpapers: Over my wallpaper by default, with the spec's titles")
    func wallpaperMode() {
        #expect(WallpaperMode.stored(nil) == .overlay)
        #expect(WallpaperMode.stored("bogus") == .overlay)
        #expect(WallpaperMode.stored("replace") == .replace)
        #expect(WallpaperMode.allCases.map(\.title) == ["Over my wallpaper", "As my wallpaper"])
    }

    @Test("Restore My Wallpaper uncovers the person's wallpaper (pausing doesn't); a new one covers it again")
    func overlayCoverage() {
        #expect(OverlayRules.uncovered(after: .restoreMyWallpaper, before: false))
        #expect(!OverlayRules.uncovered(after: .shown, before: true))
        // A liked one brought back doesn't cover what the person uncovered, and covers what was covered.
        #expect(OverlayRules.uncovered(after: .broughtBack, before: true))
        #expect(!OverlayRules.uncovered(after: .broughtBack, before: false))
        #expect(OverlayRules.covers(mode: .overlay, hasWallpaper: true, uncovered: false))
        #expect(!OverlayRules.covers(mode: .overlay, hasWallpaper: true, uncovered: true))
        #expect(!OverlayRules.covers(mode: .overlay, hasWallpaper: false, uncovered: false))
        #expect(!OverlayRules.covers(mode: .replace, hasWallpaper: true, uncovered: false))
        // A pure fade, shorter with Reduce Motion.
        #expect(OverlayRules.fadeDuration(reduceMotion: true) < OverlayRules.fadeDuration(reduceMotion: false))
    }

    @Test("Switching Show wallpapers carries the wallpaper showing over, and nothing else")
    func modeSwitch() {
        #expect(ModeSwitch.plan(from: .overlay, to: .overlay, autoPaperShowing: true) == nil)
        #expect(ModeSwitch.plan(from: .overlay, to: .replace, autoPaperShowing: true) == .carryOver)
        #expect(ModeSwitch.plan(from: .overlay, to: .replace, autoPaperShowing: false) == .leaveUncovered)
        // Paused or not (pausing keeps AutoPaper's wallpaper showing), what shows carries over.
        #expect(ModeSwitch.plan(from: .replace, to: .overlay, autoPaperShowing: true) == .carryOver)
        // The person's own picture was showing: Over my wallpaper starts uncovered.
        #expect(ModeSwitch.plan(from: .replace, to: .overlay, autoPaperShowing: false) == .leaveUncovered)
    }

    @Test("The person's picture is put back as macOS can set it again")
    func restorablePicture() {
        let photo = URL(filePath: "/Users/someone/Pictures/beach.heic")
        #expect(DesktopRules.restorablePicture(for: photo, fileExists: { _ in true }) == photo)
        // macOS's own pictures are reported as an asset file that may be gone; their descriptor can be set again.
        let asset = URL(filePath: "/Users/someone/Library/Application Support/com.apple.mobileAssetDesktop/Sonoma.heic")
        let descriptor = "/System/Library/Desktop Pictures/Sonoma.madesktop"
        #expect(DesktopRules.restorablePicture(for: asset, fileExists: { $0 == descriptor }) == URL(filePath: descriptor))
        // Nothing to set again (an Aerial's frame, say).
        #expect(DesktopRules.restorablePicture(for: asset, fileExists: { _ in false }) == nil)
        #expect(DesktopRules.restorablePicture(for: URL(string: "https://example.com/x.jpg")!, fileExists: { _ in true }) == nil)
    }

    @Test("Now says whose picture is showing, in the mode's own words")
    func ownPictureLine() {
        #expect(DesktopStatus.ownPictureLine(mode: .overlay, ownPictureShowing: false, paused: false) == nil)
        #expect(DesktopStatus.ownPictureLine(mode: .overlay, ownPictureShowing: true, paused: true)
            == "Your own wallpaper is showing. New wallpapers are paused, so it stays until you make one.")
        #expect(DesktopStatus.ownPictureLine(mode: .overlay, ownPictureShowing: true, paused: false)
            == "Your own wallpaper is showing. AutoPaper's next new wallpaper covers it again.")
        #expect(DesktopStatus.ownPictureLine(mode: .replace, ownPictureShowing: true, paused: false)
            == "Your own desktop picture is showing. AutoPaper will replace it with its next new wallpaper.")
    }

    @Test("Menu items are cut at a word")
    func menuText() {
        let note = "Echo of “Black ocean, silver structures” (March 2024): after a storm, at sunrise."
        let short = MenuText.short(note)
        #expect(short.count <= 51 && short.hasSuffix("…"))
        #expect(!short.contains("sunrise"))
        #expect(MenuText.short("Harbour at Dusk") == "Harbour at Dusk")
    }
}

@Suite("The sidebar, Now's toolbar button and the Gallery's filmstrip")
struct WindowDecisionTests {
    @Test("The sidebar marks the mood showing, or its section")
    func sidebarShowing() {
        #expect(SidebarItem.showing(section: .moods, mood: "m1", moodsExpanded: true) == .mood("m1"))
        #expect(SidebarItem.showing(section: .moods, mood: nil, moodsExpanded: true) == .section(.moods))
        // A closed group never leaves the mark on a hidden row.
        #expect(SidebarItem.showing(section: .moods, mood: "m1", moodsExpanded: false) == .section(.moods))
        #expect(SidebarItem.showing(section: .now, mood: "m1", moodsExpanded: true) == .section(.now))
        #expect(SidebarItem.showing(section: .history, mood: nil, moodsExpanded: false) == .section(.history))
    }

    @Test("Choosing Moods shows the summary; a mood shows it; Now and History keep the mood for later")
    func sidebarDestination() {
        #expect(SidebarItem.section(.moods).destination(keeping: "m1") == (.moods, nil))
        #expect(SidebarItem.mood("m2").destination(keeping: "m1") == (.moods, "m2"))
        #expect(SidebarItem.section(.now).destination(keeping: "m1") == (.now, "m1"))
        #expect(SidebarItem.section(.history).destination(keeping: nil) == (.history, nil))
    }

    @Test("New Wallpaper Now becomes Stop while one is made, like Safari's reload")
    func makeOrStop() {
        let idle = MakeOrStop(ready: true, working: false, cancellable: false)
        #expect(idle == .make(enabled: true))
        #expect((idle.title, idle.symbol, idle.help) == ("New Wallpaper Now", "arrow.clockwise", "New Wallpaper Now (⌘R)"))
        #expect(!MakeOrStop(ready: false, working: false, cancellable: false).isEnabled, "not before the engine opens")
        let working = MakeOrStop(ready: true, working: true, cancellable: true)
        #expect(working == .stop(enabled: true))
        #expect((working.title, working.symbol) == ("Stop", "xmark"))
        #expect(working.help.contains("Esc"))
        // Putting a finished one on the desktop: nothing left to stop.
        #expect(MakeOrStop(ready: true, working: true, cancellable: false) == .stop(enabled: false))
    }

    @Test("In another mood's detail the button uses that mood first; Stop is the same everywhere")
    func makeOrStopFromAnotherMood() {
        let other = MakeOrStop(ready: true, working: false, cancellable: false, otherMood: true)
        #expect(other == .make(enabled: true, otherMood: true))
        #expect((other.title, other.symbol, other.help)
            == ("Use This Mood and Make a New Wallpaper", "arrow.clockwise", "Use This Mood and Make a New Wallpaper (⌘R)"))
        #expect(!MakeOrStop(ready: false, working: false, cancellable: false, otherMood: true).isEnabled, "not before the engine opens")
        // The current mood's detail: exactly Now's button.
        #expect(MakeOrStop(ready: true, working: false, cancellable: false, otherMood: false) == .make(enabled: true))
        let working = MakeOrStop(ready: true, working: true, cancellable: true, otherMood: true)
        #expect(working == .stop(enabled: true))
        #expect((working.title, working.help) == ("Stop", "Stop Making This Wallpaper (Esc)"))
    }

    @Test("The filmstrip's arrows stay within what's loaded, and the next page loads near its end")
    func filmstrip() {
        #expect(Filmstrip.move(from: nil, by: 1, count: 5) == 0)
        #expect(Filmstrip.move(from: 2, by: 1, count: 5) == 3)
        #expect(Filmstrip.move(from: 4, by: 1, count: 5) == 4)
        #expect(Filmstrip.move(from: 0, by: -1, count: 5) == 0)
        #expect(Filmstrip.move(from: nil, by: 1, count: 0) == nil)
        #expect(!Filmstrip.needsMore(at: 10, count: 60, exhausted: false))
        #expect(Filmstrip.needsMore(at: 52, count: 60, exhausted: false))
        #expect(!Filmstrip.needsMore(at: 59, count: 60, exhausted: true))
        #expect(HistoryLayout(rawValue: "gallery") == .gallery)
        #expect(HistoryLayout.allCases.map(\.title) == ["Grid", "Gallery"])
    }

    @Test("The sidebar footer's status is one short line")
    func footerLine() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let locale = Locale(identifier: "en_US")
        let now = Date(timeIntervalSince1970: 1_791_187_200) // Monday 5 October 2026, 08:00 UTC
        let at3 = now.addingTimeInterval(7 * 3_600)
        #expect(Formatting.footerLine(stage: nil, due: at3, paused: false, cadence: .daily, armed: true, now: now, calendar: calendar, locale: locale)
            == "Next wallpaper at 3:00\u{202F}PM")
        #expect(Formatting.footerLine(stage: nil, due: at3.addingTimeInterval(86_400), paused: false, cadence: .daily, armed: true, now: now,
                                      calendar: calendar, locale: locale) == "Next wallpaper tomorrow at 3:00\u{202F}PM")
        #expect(Formatting.footerLine(stage: "Painting…", due: at3, paused: false, cadence: .daily, armed: true, now: now) == "Painting…")
        #expect(Formatting.footerLine(stage: nil, due: at3, paused: true, cadence: .daily, armed: true, now: now) == "Paused")
        #expect(Formatting.footerLine(stage: nil, due: now, paused: false, cadence: .daily, armed: false, now: now) == "Starts after your first wallpaper")
        // Where the sidebar is narrow, Time Machine's short form.
        #expect(Formatting.footerShortLine(stage: nil, due: at3, paused: false, cadence: .daily, armed: true, now: now, calendar: calendar, locale: locale)
            == "Next: Today, 3:00\u{202F}PM")
        #expect(Formatting.footerShortLine(stage: nil, due: at3.addingTimeInterval(86_400), paused: false, cadence: .daily, armed: true, now: now,
                                           calendar: calendar, locale: locale) == "Next: Tomorrow, 3:00\u{202F}PM")
        #expect(Formatting.footerShortLine(stage: nil, due: at3.addingTimeInterval(3 * 86_400), paused: false, cadence: .weekly, armed: true,
                                           now: now, calendar: calendar, locale: locale) == "Next: Thursday, 3:00\u{202F}PM")
        #expect(Formatting.footerShortLine(stage: "Painting…", due: at3, paused: false, cadence: .daily, armed: true, now: now) == "Painting…")
        #expect(Formatting.footerShortLine(stage: nil, due: at3, paused: true, cadence: .daily, armed: true, now: now) == "Paused")
        #expect(Formatting.footerShortLine(stage: nil, due: now, paused: false, cadence: .daily, armed: false, now: now) == "Not started yet")
        // Now's own line is unchanged.
        #expect(Formatting.nextLine(due: at3, paused: false, cadence: .daily, now: now, calendar: calendar, locale: locale)
            == "Next new wallpaper at 3:00\u{202F}PM")
    }
}

@Suite("Which models made a wallpaper")
struct ProvenanceTests {
    private func generation(text: ProviderKind = .demo, textModel: String = "demo", image: ProviderKind = .demo,
                            imageModel: String = "demo", width: UInt32 = 3840, height: UInt32 = 2160, cost: UInt64 = 0) -> Generation {
        let concept = Concept(
            title: "Harbour at Dusk", summary: "", setting: "", subject: "", elements: [], timeOfDay: "", weather: "", season: "",
            mood: [], palette: [], style: "", composition: "", keywordsUsed: [], wildcards: [], prompt: ""
        )
        return Generation(
            id: "g1", createdAt: 0, trigger: .manual, status: .ok, concept: concept, imagePath: nil, thumbPath: nil,
            width: width, height: height, rating: .unrated, echoOf: nil, echoNote: nil, textProvider: text, textModel: textModel,
            imageProvider: image, imageModel: imageModel, surprise: 0.35, keywords: [], costMicrousd: cost, lastShownAt: nil,
            shownCount: 0, error: nil, moodId: nil, moodName: nil
        )
    }

    @Test("Hosted models by name with their provider, and the estimated cost")
    func hosted() {
        let line = Provenance.line(generation(text: .google, textModel: "gemini-3.5-flash-lite", image: .google,
                                              imageModel: "gemini-3.1-flash-image", cost: 40_000))
        #expect(line.text == "Written by Gemini 3.5 Flash-Lite (Google Gemini) · Painted by Nano Banana 2 (Google Gemini) · 3840×2160 · about $0.04")
        #expect(line.spoken == "Written by Gemini 3.5 Flash-Lite (Google Gemini), Painted by Nano Banana 2 (Google Gemini), 3840 by 2160, about $0.04")
        #expect(line.tail == " · 3840×2160 · about $0.04")
    }

    @Test("The provider's own list names a model first; an unknown id stays an id")
    func names() {
        #expect(Provenance.modelName("gpt-image-2.5-flare", kind: .openAi) == "gpt-image-2.5-flare")
        #expect(Provenance.modelName("gemini-9-image", kind: .google, listed: ["gemini-9-image": "Nano Banana 9"]) == "Nano Banana 9")
        #expect(Provenance.modelName("gemini-3-pro-image", kind: .google) == "Nano Banana Pro")
        #expect(Provenance.who(.openAi, model: "gpt-image-2.5-flare") == "gpt-image-2.5-flare (OpenAI)")
        #expect(Provenance.who(.ollama, model: "") == "Ollama", "a record without a model: the provider")
    }

    @Test("Local and Demo wallpapers say no cost; ComfyUI's models by their workflow's name")
    func local() {
        let demo = Provenance.line(generation())
        #expect(demo.text == "Written by Demo · Painted by Demo · 3840×2160")
        let comfy = Provenance.line(generation(text: .ollama, textModel: "llama3.2", image: .comfyUi,
                                               imageModel: "z_image_turbo_bf16.safetensors", width: 0, height: 0))
        #expect(comfy.text == "Written by llama3.2 (Ollama) · Painted by Z-Image Turbo (ComfyUI)")
        #expect(comfy.tail.isEmpty)
        let ownFile = Provenance.line(generation(image: .comfyUi, imageModel: "flux/dev.gguf"))
        #expect(ownFile.painted == "Painted by flux/dev (ComfyUI)")
    }

    @Test("The diagnostic line names providers and models, never the prompt")
    func logLine() {
        let line = Provenance.logLine(generation(text: .openAi, textModel: "gpt-6-luna", image: .google, imageModel: "gemini-3.1-flash-image"))
        #expect(line == "made g1: text openAi/gpt-6-luna, image google/gemini-3.1-flash-image, 3840x2160")
    }

    @Test("Painting's Model menu changes the painting provider's model, Writing's the writer's")
    func providerChoice() {
        var settings = AutopaperCore.Settings(
            surprise: 0.35, cadence: .daily, paused: false, quietPeriod: .sixMonths, echoes: .sometimes,
            textProvider: ProviderSelection(kind: .google, model: "", baseUrl: nil),
            imageProvider: ProviderSelection(kind: .google, model: "", baseUrl: nil),
            imageQuality: .standard, monthlyBudgetCents: 500, fallback: .revisitLiked, replaceDisliked: true,
            setLockScreen: false, storageLimitMb: 2_048, comfyuiWorkflow: nil
        )
        ProviderChoice.choose(model: "gemini-3-pro-image", for: .images, in: &settings)
        #expect(settings.imageProvider == ProviderSelection(kind: .google, model: "gemini-3-pro-image", baseUrl: nil))
        #expect(settings.textProvider.model == "", "the writer is untouched")
        ProviderChoice.choose(model: " gemini-3.8-flash ", for: .concepts, in: &settings)
        #expect(settings.textProvider.model == "gemini-3.8-flash")
        #expect(settings.imageProvider.model == "gemini-3-pro-image")
        ProviderChoice.choose(model: "", for: .images, in: &settings)
        #expect(settings.imageProvider.model == "", "back to the default")
    }
}

@Suite("The summary of every mood")
struct MoodSummaryTests {
    private func keyword(_ text: String, _ weight: KeywordWeight = .must) -> Keyword {
        Keyword(id: text, text: text, weight: weight, position: 0, createdAt: 0)
    }

    private func mood(_ id: String, created: Int64 = 0, keywords: [Keyword] = []) -> Mood {
        Mood(id: id, name: id.capitalized, position: 0, surprise: 0.35, createdAt: created, keywords: keywords, active: false)
    }

    private func stats(_ id: String, wallpapers: UInt32, liked: UInt32 = 0, echoes: UInt32 = 0) -> MoodStats {
        MoodStats(moodId: id, wallpapers: wallpapers, liked: liked, disliked: 0, echoes: echoes, lastMadeAt: nil, latest: [])
    }

    @Test("Keywords by weight, Must first, leaving out weights with none")
    func keywordGroups() {
        let groups = MoodText.keywordGroups([keyword("rain"), keyword("people", .avoid), keyword("beach"), keyword("night", .maybe)])
        #expect(groups.map(\.line) == ["Must: rain, beach", "Maybe: night", "Avoid: people"])
        #expect(MoodText.keywordGroups([]).isEmpty)
        let made = MoodText.keywordGroups(snapshots: [KeywordSnapshot(text: "fog", weight: .maybe)])
        #expect(made.map(\.line) == ["Maybe: fog"])
        #expect(KeywordBreakdown.line([mood("a", keywords: [keyword("rain"), keyword("x", .avoid)]), mood("b", keywords: [keyword("sea")])])
            == "2 Must · 0 Maybe · 1 Avoid keywords")
        #expect(KeywordBreakdown.line([]) == "No keywords yet")
    }

    @Test("Surprise, what a mood made and when, in words")
    func moodLines() {
        #expect(MoodText.surpriseLine(0.35) == "Surprise: Fresh (35%)")
        #expect(MoodText.surpriseLine(1.4) == "Surprise: Wild (100%)")
        let now = Date(timeIntervalSince1970: 1_791_187_200)
        let locale = Locale(identifier: "en_US")
        let yesterday = Int64(now.timeIntervalSince1970) - 86_400
        #expect(MoodText.madeLine(wallpapers: 12, liked: 3, lastMadeAt: yesterday, now: now, locale: locale)
            == "12 wallpapers · 3 liked · last made yesterday")
        #expect(MoodText.madeLine(wallpapers: 1, liked: 0, lastMadeAt: yesterday, separator: ", ", now: now, locale: locale)
            == "1 wallpaper, 0 liked, last made yesterday")
        #expect(MoodText.madeLine(wallpapers: 0, liked: 0, lastMadeAt: nil) == "Nothing made yet")
        #expect(MoodText.relative(Int64(now.timeIntervalSince1970) - 3 * 86_400, now: now, locale: locale) == "3 days ago")
        #expect(MoodText.relative(Int64(now.timeIntervalSince1970) + 600, now: now, locale: locale) == "just now", "a clock that moved back")
    }

    @Test("Totals count moods, wallpapers and likes")
    func totals() {
        let totals = MoodText.totals(moods: 3, stats: [stats("a", wallpapers: 10, liked: 2), stats("b", wallpapers: 1, liked: 1)])
        #expect(totals.map(\.spoken) == ["3 moods", "11 wallpapers made", "3 liked"])
        #expect(MoodText.totals(moods: 1, stats: [stats("a", wallpapers: 1)]).map(\.spoken) == ["1 mood", "1 wallpaper made", "0 liked"])
        #expect(MoodText.spokenLatest(["Fog", "Harbour"]) == "Latest wallpapers: Fog, Harbour")
        #expect(MoodText.spokenLatest(["Fog"]) == "Latest wallpaper: Fog")
    }
}

@Suite("The last 30 days, stacked by mood")
struct MoodActivityTests {
    private func mood(_ id: String, created: Int64) -> Mood {
        Mood(id: id, name: id, position: 0, surprise: 0.35, createdAt: created, keywords: [], active: false)
    }

    @Test("Days are the person's local days, including a 25-hour one")
    func dayBounds() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/New_York")!
        // Monday 2 November 2026, noon in New York: daylight saving ended on Sunday 1 November.
        let now = Date(timeIntervalSince1970: 1_793_638_800)
        let bounds = MoodActivity.dayBounds(days: 3, now: now, calendar: calendar)
        #expect(bounds.count == 4, "three days and the end of the last")
        #expect(bounds.last! - bounds[bounds.count - 2] == 86_400, "today")
        #expect(bounds[2] - bounds[1] == 90_000, "Sunday had 25 hours")
        #expect(bounds == bounds.sorted())
        #expect(MoodActivity.dayBounds(now: now, calendar: calendar).count == MoodActivity.days + 1)
    }

    @Test("Each mood keeps a colour by when it was made; past eight, the newest share Other moods")
    func colorSlots() {
        let moods = [mood("c", created: 30), mood("a", created: 10), mood("b", created: 20)]
        #expect(MoodActivity.colorSlots(moods) == ["a": 0, "b": 1, "c": 2])
        #expect(MoodActivity.colorSlots(moods.reversed()) == ["a": 0, "b": 1, "c": 2], "reordering the list doesn't repaint")
        let many = (0..<10).map { mood("m\($0)", created: Int64($0)) }
        let slots = MoodActivity.colorSlots(many)
        #expect(slots.count == 7)
        #expect(Set(slots.values) == Set(0..<7))
        #expect(slots["m7"] == nil && slots["m9"] == nil)
    }

    @Test("Segments stack in the list's order, Other moods on top, with each day's top marked")
    func segments() {
        let day1: Int64 = 1_791_158_400, day2 = day1 + 86_400
        let counts = [
            DayCount(dayStart: day1, moodId: "b", count: 2),
            DayCount(dayStart: day1, moodId: "a", count: 1),
            DayCount(dayStart: day1, moodId: nil, count: 1),
            DayCount(dayStart: day1, moodId: "x", count: 3),
            DayCount(dayStart: day2, moodId: "a", count: 4),
        ]
        let segments = MoodActivity.segments(counts, order: ["a", "b", "x"], slots: ["a": 0, "b": 1])
        let first = segments.filter { $0.day == Date(timeIntervalSince1970: TimeInterval(day1)) }
        #expect(first.map(\.series) == [.mood("a"), .mood("b"), .other])
        #expect(first.map(\.count) == [1, 2, 4], "a deleted mood and one past the palette share Other moods")
        #expect(first.map(\.base) == [0, 1, 3])
        #expect(first.map(\.isTop) == [false, false, true])
        let second = segments.filter { $0.day == Date(timeIntervalSince1970: TimeInterval(day2)) }
        #expect(second.map(\.count) == [4])
        #expect(second.first?.isTop == true)
        #expect(MoodActivity.dayLine(first) { $0 == .other ? "Other moods" : "Mood" } == "Mood 1, Mood 2, Other moods 4")
    }

    @Test("The scale is whole numbers, at most five ticks", arguments: [
        (0, [0, 1]), (1, [0, 1]), (2, [0, 1, 2]), (4, [0, 1, 2, 3, 4]), (7, [0, 2, 4, 6, 8]), (15, [0, 5, 10, 15]),
        (24, [0, 10, 20, 30]), (130, [0, 50, 100, 150]),
    ] as [(Int, [Int])])
    func ticks(maximum: Int, expected: [Int]) {
        #expect(MoodActivity.ticks(maximum: maximum) == expected)
    }

    @Test("A label a week apart, the last clear of the scale")
    func weekLabels() {
        let bounds = (0...30).map { Int64($0) * 86_400 }
        #expect(MoodActivity.weekLabels(bounds: bounds) == [3, 10, 17, 24].map { Int64($0) * 86_400 })
        #expect(MoodActivity.weekLabels(bounds: [0, 86_400]).isEmpty)
    }

    @Test func footnote() {
        #expect(MoodActivity.footnote(total: 42) == "42 wallpapers in the last 30 days. Kept on this Mac only.")
        #expect(MoodActivity.footnote(total: 1) == "1 wallpaper in the last 30 days. Kept on this Mac only.")
    }
}
