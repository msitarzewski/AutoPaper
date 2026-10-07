import AutopaperCore
import Foundation

/// Moods in words: names for new and duplicated moods, a mood's keywords in short, and what VoiceOver says for a
/// mood. The engine owns the rules (1–40 characters, unique ignoring case); these only pick names that pass them,
/// so New Mood and Duplicate never fail on a name the person didn't type.
enum MoodText {
    /// The core's `Mood::MAX_NAME_LEN` (not exported; the engine refuses longer names with `MoodNameTooLong`).
    static let maxNameLength = 40

    /// New Mood's name (sentence case, as the spec words it).
    static let newMoodName = "New mood"

    /// `base`, or "base 2", "base 3"… — the first that no mood has (ignoring case and spacing), cut to fit 40
    /// characters with its number.
    static func unique(_ base: String, among names: [String]) -> String {
        let taken = Set(names.map(key))
        let trimmed = KeywordText.normalised(base)
        for number in 1... {
            let suffix = number == 1 ? "" : " \(number)"
            let stem = String(trimmed.prefix(maxNameLength - suffix.count)).trimmingCharacters(in: .whitespaces)
            let candidate = stem + suffix
            if !taken.contains(key(candidate)) { return candidate }
        }
        return trimmed
    }

    /// Duplicate's name: "Rainy beach copy", then "Rainy beach copy 2" (Finder's pattern).
    static func copyName(of name: String, among names: [String]) -> String {
        let suffix = " copy"
        let stem = String(KeywordText.normalised(name).prefix(maxNameLength - suffix.count - 3)).trimmingCharacters(in: .whitespaces)
        return unique(stem + suffix, among: names)
    }

    /// A mood's keywords in short, in their order: "rain · beach · night · no people" (an Avoid reads as what's left
    /// out). "No keywords yet" when it has none.
    static func summary(_ keywords: [Keyword]) -> String {
        guard !keywords.isEmpty else { return "No keywords yet" }
        return keywords.map(short).joined(separator: " · ")
    }

    /// The same for VoiceOver, with commas instead of dots: "rain, beach, night, no people".
    static func spokenSummary(_ keywords: [Keyword]) -> String {
        guard !keywords.isEmpty else { return "No keywords yet" }
        return keywords.map(short).joined(separator: ", ")
    }

    /// A mood's name as VoiceOver says it in lists and menus: "Rainy beach, current mood".
    static func spokenName(_ mood: Mood) -> String {
        mood.active ? "\(mood.name), current mood" : mood.name
    }

    /// A mood's row in the Moods list as VoiceOver says it: "Rainy beach, current mood, rain, beach, no people".
    static func spokenRow(_ mood: Mood) -> String {
        "\(spokenName(mood)), \(spokenSummary(mood.keywords))"
    }

    /// A mood's keywords grouped by weight, Must then Maybe then Avoid (each in the mood's order), leaving out
    /// weights it has none of. The summary of all moods shows them this way.
    static func keywordGroups(_ keywords: [Keyword]) -> [KeywordGroup] {
        KeywordWeight.all.compactMap { weight in
            let words = keywords.filter { $0.weight == weight }.map(\.text)
            return words.isEmpty ? nil : KeywordGroup(weight: weight, words: words)
        }
    }

    /// The keywords a wallpaper was made with (`Generation.keywords`), grouped the same way.
    static func keywordGroups(snapshots: [KeywordSnapshot]) -> [KeywordGroup] {
        KeywordWeight.all.compactMap { weight in
            let words = snapshots.filter { $0.weight == weight }.map(\.text)
            return words.isEmpty ? nil : KeywordGroup(weight: weight, words: words)
        }
    }

    /// "Surprise: Fresh (35%)".
    static func surpriseLine(_ surprise: Float) -> String {
        let percent = Int((min(max(surprise, 0), 1) * 100).rounded())
        return "Surprise: \(SurpriseBand(percent: percent).title) (\(percent)%)"
    }

    /// What a mood has made, on one line: "12 wallpapers · 3 liked · last made yesterday", or "Nothing made yet".
    /// `separator` is " · " to show and ", " to speak.
    static func madeLine(wallpapers: UInt32, liked: UInt32, lastMadeAt: Int64?, separator: String = " · ",
                         now: Date = .now, locale: Locale = .current) -> String {
        guard wallpapers > 0 else { return "Nothing made yet" }
        var parts = [count(Int(wallpapers), "wallpaper", "wallpapers"), "\(liked) liked"]
        if let lastMadeAt { parts.append("last made \(relative(lastMadeAt, now: now, locale: locale))") }
        return parts.joined(separator: separator)
    }

    /// "yesterday", "3 days ago", "5 minutes ago" (never in the future: a clock that moved back reads "now").
    static func relative(_ unix: Int64, now: Date = .now, locale: Locale = .current) -> String {
        let date = min(Date(timeIntervalSince1970: TimeInterval(unix)), now)
        if now.timeIntervalSince(date) < 60 { return "just now" }
        let formatter = RelativeDateTimeFormatter()
        formatter.dateTimeStyle = .named
        formatter.unitsStyle = .full
        formatter.formattingContext = .middleOfSentence
        formatter.locale = locale
        return formatter.localizedString(for: date, relativeTo: now)
    }

    /// The totals above the summary of all moods: moods, wallpapers made, liked.
    static func totals(moods: Int, stats: [MoodStats]) -> [Total] {
        let made = stats.reduce(0) { $0 + Int($1.wallpapers) }
        let liked = stats.reduce(0) { $0 + Int($1.liked) }
        return [
            Total(value: moods, caption: moods == 1 ? "mood" : "moods"),
            Total(value: made, caption: made == 1 ? "wallpaper made" : "wallpapers made"),
            Total(value: liked, caption: "liked"),
        ]
    }

    /// The thumbnails of a mood's latest wallpapers as one spoken line: "Latest wallpapers: Harbour at dusk, Fog".
    static func spokenLatest(_ titles: [String]) -> String {
        titles.count == 1 ? "Latest wallpaper: \(titles[0])" : "Latest wallpapers: \(titles.joined(separator: ", "))"
    }

    struct KeywordGroup: Equatable {
        let weight: KeywordWeight
        let words: [String]

        /// "Must: rain, beach".
        var line: String { "\(weight.title): \(words.joined(separator: ", "))" }
    }

    struct Total: Equatable {
        let value: Int
        let caption: String

        /// "3 moods".
        var spoken: String { "\(value) \(caption)" }
    }

    private static func count(_ value: Int, _ one: String, _ many: String) -> String {
        "\(value) \(value == 1 ? one : many)"
    }

    private static func short(_ keyword: Keyword) -> String {
        keyword.weight == .avoid ? "no \(keyword.text)" : keyword.text
    }

    private static func key(_ name: String) -> String {
        KeywordText.normalised(name).lowercased()
    }
}

/// Where a moved row lands, for List's `onMove` (which gives the gap it was dropped in) and for Move Up / Down.
enum Reorder {
    /// The 0-based position an item from `from` takes when dropped at gap `destination` (0…count).
    static func position(from: Int, droppedAt destination: Int) -> Int {
        destination > from ? destination - 1 : destination
    }

    /// `items` with the one at `from` moved to `position` (clamped), as the engine will order them.
    static func moved<T>(_ items: [T], from: Int, to position: Int) -> [T] {
        guard items.indices.contains(from) else { return items }
        var result = items
        let item = result.remove(at: from)
        result.insert(item, at: min(max(position, 0), result.count))
        return result
    }
}

/// The Moods summary's chart (user, 2026-10-06): wallpapers per day over the last 30 days, stacked by mood, from the
/// engine's `activity`. Days are the person's local days; each mood keeps one colour (by when it was made, so
/// reordering the list doesn't repaint it); past eight moods, and for moods since deleted, the bars fold into
/// "Other moods".
enum MoodActivity {
    static let days = 30
    /// Categorical colours available (the dataviz palette's eight; the ninth series is "Other moods").
    static let slots = 8
    static let otherName = "Other moods"

    /// The `day_bounds` for `activity`: the local midnight starting each of the last `days` days (today last), then
    /// tomorrow's, so a 23- or 25-hour day (daylight saving) is still one day.
    static func dayBounds(days: Int = days, now: Date = .now, calendar: Calendar = .current) -> [Int64] {
        let today = calendar.startOfDay(for: now)
        return (0...days).compactMap { offset in
            calendar.date(byAdding: .day, value: offset - days + 1, to: today).map { Int64($0.timeIntervalSince1970) }
        }
    }

    /// Each mood's colour slot (0–7), by when it was made (oldest first; then id). With more than eight moods the
    /// newest share "Other moods" (no slot), so no colour is ever generated past the palette.
    static func colorSlots(_ moods: [Mood]) -> [String: Int] {
        let byAge = moods.sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }
        let coloured = byAge.count > slots ? Array(byAge.prefix(slots - 1)) : byAge
        return Dictionary(uniqueKeysWithValues: coloured.enumerated().map { ($1.id, $0) })
    }

    /// Who a bar segment belongs to.
    enum Series: Hashable {
        case mood(String)
        case other
    }

    /// One stacked segment: a day, a mood (or the rest), how many.
    struct Segment: Identifiable, Equatable {
        let day: Date
        let series: Series
        let count: Int
        /// Where it starts in its day's stack (the wallpapers below it).
        let base: Int
        /// The day's topmost segment (its end is rounded; the others are square).
        let isTop: Bool
        var id: String { "\(day.timeIntervalSince1970)-\(series)" }
    }

    /// The engine's counts as stacked segments, each day's in the moods' list order with "Other moods" on top.
    /// `order` is the moods' ids in the person's order; `slots` from `colorSlots`.
    static func segments(_ counts: [DayCount], order: [String], slots: [String: Int]) -> [Segment] {
        let place = Dictionary(uniqueKeysWithValues: order.enumerated().map { ($1, $0) })
        var byDay: [Int64: [Series: Int]] = [:]
        for count in counts {
            let series: Series = count.moodId.flatMap { slots[$0] != nil ? Series.mood($0) : nil } ?? .other
            byDay[count.dayStart, default: [:]][series, default: 0] += Int(count.count)
        }
        func rank(_ series: Series) -> Int {
            if case .mood(let id) = series { return place[id] ?? Int.max - 1 }
            return Int.max
        }
        return byDay.keys.sorted().flatMap { start -> [Segment] in
            let day = Date(timeIntervalSince1970: TimeInterval(start))
            let stack = byDay[start, default: [:]].sorted { rank($0.key) < rank($1.key) }
            var base = 0
            return stack.enumerated().map { index, entry in
                defer { base += entry.value }
                return Segment(day: day, series: entry.key, count: entry.value, base: base, isTop: index == stack.count - 1)
            }
        }
    }

    /// The y-axis: whole numbers from 0 to a top at or above `maximum`, at most five ticks (0, 1, 2 · 0, 2, 4, 6 ·
    /// 0, 5, 10, 15…).
    static func ticks(maximum: Int) -> [Int] {
        let top = max(maximum, 1)
        let step = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1_000].first { (top + $0 - 1) / $0 <= 4 } ?? max(1, top / 4)
        let last = (top + step - 1) / step * step
        return Array(stride(from: 0, through: last, by: step))
    }

    /// Where the day axis is labelled: a week apart, ending five days before the last day, so the last label sits
    /// clear of the scale on the trailing side (`bounds` as from `dayBounds`).
    static func weekLabels(bounds: [Int64]) -> [Int64] {
        let starts = Array(bounds.dropLast())
        return stride(from: starts.count - 6, through: 0, by: -7).reversed().map { starts[$0] }
    }

    /// "42 wallpapers in the last 30 days. Kept on this Mac only."
    static func footnote(total: Int) -> String {
        let made = total == 1 ? "1 wallpaper" : "\(total) wallpapers"
        return "\(made) in the last \(days) days. Kept on this Mac only."
    }

    /// One day as VoiceOver and the table say it: "Rainy beach 2, Night city 1".
    static func dayLine(_ segments: [Segment], name: (Series) -> String) -> String {
        segments.map { "\(name($0.series)) \($0.count)" }.joined(separator: ", ")
    }
}

/// The keywords of every mood by weight, for the summary: "12 Must · 7 Maybe · 3 Avoid keywords".
enum KeywordBreakdown {
    static func line(_ moods: [Mood]) -> String {
        let keywords = moods.flatMap(\.keywords)
        guard !keywords.isEmpty else { return "No keywords yet" }
        let parts = KeywordWeight.all.map { weight in "\(keywords.filter { $0.weight == weight }.count) \(weight.title)" }
        return parts.joined(separator: " · ") + (keywords.count == 1 ? " keyword" : " keywords")
    }
}
