import Foundation

struct ParseSettings: Decodable {
    let parseRules: [InputParseRule]
    let builtinParseTagName: String

    private enum CodingKeys: String, CodingKey { case parseRules, builtinParseTagName }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        parseRules = try values.decodeIfPresent([InputParseRule].self, forKey: .parseRules) ?? []
        builtinParseTagName = try values.decodeIfPresent(String.self, forKey: .builtinParseTagName) ?? ""
    }
}

struct InputParseRule: Decodable {
    let pattern: String
    let issueGroup: Int
    let titleGroup: Int
    let linkGroup: Int
    let tagName: String

    private enum CodingKeys: String, CodingKey { case pattern, issueGroup, titleGroup, linkGroup, tagName }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        pattern = try values.decodeIfPresent(String.self, forKey: .pattern) ?? ""
        issueGroup = try values.decodeIfPresent(Int.self, forKey: .issueGroup) ?? 0
        titleGroup = try values.decodeIfPresent(Int.self, forKey: .titleGroup) ?? 0
        linkGroup = try values.decodeIfPresent(Int.self, forKey: .linkGroup) ?? 0
        tagName = try values.decodeIfPresent(String.self, forKey: .tagName) ?? ""
    }
}

struct ParsedInput {
    var title: String
    var issueKey: String?
    var links: [String]
    var tagName: String?
    var isEmpty: Bool { issueKey == nil && title.isEmpty && links.isEmpty }
    var json: [String: Any] {
        ["title": title, "issueKey": issueKey as Any? ?? NSNull(), "links": links,
         "tagName": tagName as Any? ?? NSNull()]
    }
}

enum InputParser {
    static func parse(settings: ParseSettings, text: String) -> ParsedInput {
        for rule in settings.parseRules {
            if let result = apply(rule, to: text), !result.isEmpty { return result }
        }
        var result = builtin(text)
        let tag = settings.builtinParseTagName.trimmingCharacters(in: .whitespaces)
        if !tag.isEmpty { result.tagName = tag }
        return result
    }

    static func export(rules: URL, input: URL, output: URL) throws {
        let settings = try JSONDecoder().decode(ParseSettings.self, from: Data(contentsOf: rules))
        let text = try String(contentsOf: input, encoding: .utf8)
        let result = parse(settings: settings, text: text)
        try JSONSerialization.data(withJSONObject: result.json, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
            .write(to: output, options: .atomic)
    }

    private static func apply(_ rule: InputParseRule, to text: String) -> ParsedInput? {
        guard !rule.pattern.isEmpty,
              let regex = try? NSRegularExpression(pattern: rule.pattern, options: [.dotMatchesLineSeparators]) else {
            return nil
        }
        let ns = text as NSString
        guard let match = regex.firstMatch(in: text, range: NSRange(location: 0, length: ns.length)) else { return nil }
        func group(_ index: Int) -> String? {
            guard index > 0, index < match.numberOfRanges else { return nil }
            let range = match.range(at: index)
            guard range.location != NSNotFound else { return nil }
            return ns.substring(with: range).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return ParsedInput(title: group(rule.titleGroup) ?? "", issueKey: group(rule.issueGroup),
                           links: group(rule.linkGroup).map { [$0] } ?? [],
                           tagName: rule.tagName.isEmpty ? nil : rule.tagName)
    }

    // Match the legacy Foundation parser: links anywhere, issue key only at the
    // beginning after links are removed, then join title lines and whitespace.
    private static func builtin(_ raw: String) -> ParsedInput {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        var links: [String] = []
        if let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue) {
            let ns = text as NSString
            let matches = detector.matches(in: text, range: NSRange(location: 0, length: ns.length))
            links = matches.compactMap { $0.url?.absoluteString }
            var mutable = ns
            for match in matches.reversed() {
                mutable = mutable.replacingCharacters(in: match.range, with: " ") as NSString
            }
            text = mutable as String
        }
        var issueKey: String?
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if let regex = try? NSRegularExpression(pattern: #"\b[A-Z][A-Z0-9]*-\d+\b"#) {
            let ns = trimmed as NSString
            if let first = regex.firstMatch(in: trimmed, range: NSRange(location: 0, length: ns.length)),
               first.range.location == 0 {
                issueKey = ns.substring(with: first.range)
                text = ns.replacingCharacters(in: first.range, with: " ")
            } else { text = trimmed }
        } else { text = trimmed }
        let title = text.components(separatedBy: .newlines)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }.joined(separator: " ")
            .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return ParsedInput(title: title, issueKey: issueKey, links: links, tagName: nil)
    }
}
