import AppKit
import Foundation

/// An event's detail split into what came before its JSON ("POST https://… / HTTP 200 · 1.23 s") and the JSON itself,
/// formatted, checked and coloured for reading. Details with no JSON stay plain text.
struct EventBody {
    let title: String
    let head: String
    let json: String
    /// False when the text doesn't parse as JSON (the Console cuts a body off at 65,536 bytes).
    let valid: Bool
    let truncated: Bool

    /// A JSON text on its own (an event's schema), titled by the caller.
    init(json text: String, title: String) {
        let parsed = try? JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed])
        self.title = title
        head = ""
        json = text
        valid = parsed != nil
        truncated = false
    }

    init?(detail: String, kind: String) {
        let lines = detail.split(separator: "\n", omittingEmptySubsequences: false)
        guard let start = lines.firstIndex(where: { line in
            guard let first = line.first(where: { !$0.isWhitespace }) else { return false }
            return first == "{" || first == "["
        }) else { return nil }
        head = lines[..<start].joined(separator: "\n")
        var body = lines[start...].joined(separator: "\n")
        truncated = body.contains("[Console detail truncated")
        if let marker = body.range(of: "\n[Console detail truncated") { body = String(body[..<marker.lowerBound]) }
        let parsed = try? JSONSerialization.jsonObject(with: Data(body.utf8), options: [.fragmentsAllowed])
        valid = parsed != nil
        if let parsed, !body.contains("\n"),
           let data = try? JSONSerialization.data(withJSONObject: parsed, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]),
           let pretty = String(data: data, encoding: .utf8) {
            body = pretty
        }
        json = body
        switch kind {
        case "request": title = "Request body"
        case "response": title = "Response body"
        default: title = "Body"
        }
    }

    var size: String { ByteCountFormatter.string(fromByteCount: Int64(json.utf8.count), countStyle: .file) }
}

/// Colours JSON text: keys, strings, numbers, true/false/null, and the punctuation between them. System colours, so
/// light and dark mode and increased contrast apply; the text itself is unchanged.
enum JSONColors {
    private static let pattern = try? NSRegularExpression(
        pattern: #"("(?:\\.|[^"\\])*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b|([{}\[\],:])"#)

    static func highlight(_ text: String) -> AttributedString {
        let styled = NSMutableAttributedString(string: text, attributes: [.foregroundColor: NSColor.labelColor])
        guard let pattern else { return AttributedString(styled) }
        let whole = NSRange(text.startIndex..., in: text)
        for match in pattern.matches(in: text, range: whole) {
            let color: NSColor
            if match.range(at: 1).location != NSNotFound {
                color = match.range(at: 2).location != NSNotFound ? .systemTeal : .systemGreen
                styled.addAttribute(.foregroundColor, value: color, range: match.range(at: 1))
                if match.range(at: 2).location != NSNotFound {
                    styled.addAttribute(.foregroundColor, value: NSColor.secondaryLabelColor, range: match.range(at: 2))
                }
                continue
            }
            if match.range(at: 3).location != NSNotFound { color = .systemOrange }
            else if match.range(at: 4).location != NSNotFound { color = .systemPink }
            else { color = .secondaryLabelColor }
            styled.addAttribute(.foregroundColor, value: color, range: match.range)
        }
        return AttributedString(styled)
    }
}


/// A writing call's "instructions" event split into what was sent: the system instructions, the user prompt, the JSON
/// schema and the temperature. Nil when the text isn't in that shape.
struct InstructionsDetail: Equatable {
    let system: String
    let user: String
    let schema: String
    let temperature: String?

    init?(detail: String) {
        guard detail.hasPrefix("System instructions:\n"),
              let userMark = detail.range(of: "\n\nUser prompt:\n"),
              let schemaMark = detail.range(of: "\n\nSchema:\n", range: userMark.upperBound..<detail.endIndex) else { return nil }
        system = String(detail[detail.index(detail.startIndex, offsetBy: "System instructions:\n".count)..<userMark.lowerBound])
        user = String(detail[userMark.upperBound..<schemaMark.lowerBound])
        var rest = String(detail[schemaMark.upperBound...])
        var temperature: String?
        if let mark = rest.range(of: "\n\nTemperature (where supported): ") {
            temperature = String(rest[mark.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
            rest = String(rest[..<mark.lowerBound])
        }
        schema = rest
        self.temperature = temperature
    }
}

/// The few kinds of line the composer's instructions use, so they can be shown as a reader would see them.
enum MarkdownBlock: Equatable {
    case heading(level: Int, text: String)
    case bullet(text: String)
    case numbered(marker: String, text: String)
    case paragraph(text: String)
    case space

    /// Each line is one block (the instructions put a paragraph on one line); blank lines become `.space`, repeated
    /// ones once.
    static func parse(_ text: String) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        for raw in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty {
                if blocks.last != .space, !blocks.isEmpty { blocks.append(.space) }
            } else if line.hasPrefix("#") {
                let level = line.prefix { $0 == "#" }.count
                blocks.append(.heading(level: min(level, 6), text: line.drop { $0 == "#" }.trimmingCharacters(in: .whitespaces)))
            } else if line.hasPrefix("- ") || line.hasPrefix("* ") {
                blocks.append(.bullet(text: String(line.dropFirst(2))))
            } else if let dot = line.firstIndex(of: "."), line[..<dot].allSatisfy(\.isNumber), !line[..<dot].isEmpty,
                      line[line.index(after: dot)...].hasPrefix(" ") {
                blocks.append(.numbered(marker: String(line[...dot]), text: line[line.index(after: dot)...].trimmingCharacters(in: .whitespaces)))
            } else {
                blocks.append(.paragraph(text: line))
            }
        }
        while blocks.last == .space { blocks.removeLast() }
        return blocks
    }
}
