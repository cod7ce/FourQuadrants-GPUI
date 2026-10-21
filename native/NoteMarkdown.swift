import AppKit
import CryptoKit
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct NoteMarkdownAttachment: Codable, Equatable {
    let id: String
    let name: String
    let mimeType: String
    let data: String
}

struct NoteMarkdownDocument: Codable, Equatable {
    // Keep the decoded string, including U+FFFC, for exact legacy-note deduplication.
    let plainText: String
    let source: String
    let attachments: [NoteMarkdownAttachment]
}

@MainActor
enum NoteMarkdown {
    static let attachmentLimit = 20 * 1024 * 1024
    static let totalAttachmentLimit = 50 * 1024 * 1024
    private static let inlineImageTypes: Set<String> = [
        "image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/tiff"
    ]

    @discardableResult
    static func export(input: URL, output: URL) throws -> NoteMarkdownDocument {
        let input = input.resolvingSymlinksInPath().standardizedFileURL
        let output = output.resolvingSymlinksInPath().standardizedFileURL
        guard output != input, !output.path.hasPrefix(input.path + "/") else {
            throw HelperError("Markdown output must not overwrite the original RTFD or its contents.")
        }
        let attributes = try FileManager.default.attributesOfItem(atPath: input.path)
        let type = attributes[.type] as? FileAttributeType
        guard type == .typeDirectory || type == .typeRegular else {
            throw HelperError("Note input must be a regular RTFD file or an RTFD package.")
        }
        // Editor loading accepts missing/empty input; extraction must never turn those into a blank note.
        let text: NSAttributedString
        if type == .typeRegular {
            let data = try Data(contentsOf: input)
            guard !data.isEmpty else {
                throw HelperError("The RTFD input is empty; the note was not converted.")
            }
            text = try RichDocument.decode(data)
        } else {
            text = try RichDocument.load(input)
            guard FileManager.default.fileExists(atPath: input.path) else {
                throw HelperError("The RTFD package disappeared while the note was being converted.")
            }
        }
        let result = try extract(text)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        try encoder.encode(result).write(to: output, options: .atomic)
        return result
    }

    static func extract(_ text: NSAttributedString) throws -> NoteMarkdownDocument {
        var source = ""
        var attachments: [NoteMarkdownAttachment] = []
        var totalBytes = 0
        var position = 0
        var pending = ""
        var pendingStyle = Style()
        let string = text.string as NSString
        func flush() {
            source += markdown(pending, style: pendingStyle)
            pending = ""
        }
        while position < text.length {
            var range = NSRange()
            let attributes = text.attributes(at: position, effectiveRange: &range)
            let style = Style(attributes)
            if style != pendingStyle {
                flush()
                pendingStyle = style
            }
            let fragments = string.substring(with: range).components(separatedBy: "\u{FFFC}")
            var offset = range.location
            for (index, fragment) in fragments.enumerated() {
                pending += fragment
                offset += fragment.utf16.count
                if index < fragments.count - 1 {
                    flush()
                    guard let attachment = attributes[.attachment] as? NSTextAttachment else {
                        throw HelperError("The note contains an attachment marker without an attachment at position \(offset).")
                    }
                    let extracted = try extractAttachment(attachment, position: offset)
                    guard extracted.bytes.count <= attachmentLimit else {
                        throw HelperError("Attachment \(extracted.name) exceeds the 20 MiB limit.")
                    }
                    totalBytes += extracted.bytes.count
                    guard totalBytes <= totalAttachmentLimit else {
                        throw HelperError("The note's attachments exceed the 50 MiB total limit.")
                    }
                    let id = attachmentID(extracted.bytes, position: offset)
                    attachments.append(NoteMarkdownAttachment(id: id, name: extracted.name,
                                                              mimeType: extracted.mimeType,
                                                              data: extracted.bytes.base64EncodedString()))
                    let label = escape(extracted.name.replacingOccurrences(of: "\r", with: " ")
                        .replacingOccurrences(of: "\n", with: " "))
                    source += "\(extracted.inlineImage ? "!" : "")[\(label)](attachment:\(id))"
                    offset += 1
                }
            }
            position = NSMaxRange(range)
        }
        flush()
        return NoteMarkdownDocument(plainText: text.string, source: source, attachments: attachments)
    }

    private struct Payload {
        let bytes: Data
        let name: String
        let mimeType: String
        let inlineImage: Bool
    }

    private static func extractAttachment(_ attachment: NSTextAttachment, position: Int) throws -> Payload {
        let wrapper = attachment.fileWrapper
        let originalName = wrapper?.preferredFilename ?? wrapper?.filename
        let bytes: Data
        let mimeType: String
        let inlineImage: Bool
        let name: String
        if let wrapper, !wrapper.isRegularFile {
            guard let serialized = wrapper.serializedRepresentation else {
                throw HelperError("Unable to preserve the attachment package at position \(position).")
            }
            bytes = serialized
            mimeType = "application/octet-stream"
            inlineImage = false
            name = originalName ?? "attachment-\(position).bin"
        } else if let original = wrapper?.regularFileContents ?? attachment.contents {
            bytes = original
            // Inspect original bytes first; a filename alone must not turn a file into an inline image.
            let imageType = CGImageSourceCreateWithData(original as CFData, nil).flatMap { source -> UTType? in
                guard CGImageSourceGetCount(source) > 0, let identifier = CGImageSourceGetType(source) else { return nil }
                return UTType(identifier as String)
            }
            let declaredType = attachment.fileType.flatMap { UTType($0) }
            let namedType = originalName.flatMap { UTType(filenameExtension: ($0 as NSString).pathExtension) }
            let type = imageType ?? declaredType ?? namedType
            let imageMIME = imageType?.preferredMIMEType ?? ""
            inlineImage = inlineImageTypes.contains(imageMIME)
            mimeType = inlineImage ? imageMIME : "application/octet-stream"
            name = originalName ?? "attachment-\(position).\(type?.preferredFilenameExtension ?? "bin")"
        } else {
            // Only synthesize display bytes when there is no original payload to preserve.
            guard let image = RichImages.image(attachment),
                  let tiff = image.tiffRepresentation,
                  let bitmap = NSBitmapImageRep(data: tiff),
                  let png = bitmap.representation(using: .png, properties: [:]) else {
                throw HelperError("Unable to recover attachment data at position \(position).")
            }
            bytes = png
            mimeType = "image/png"
            inlineImage = true
            name = originalName ?? "image-\(position).png"
        }
        return Payload(bytes: bytes, name: name, mimeType: mimeType, inlineImage: inlineImage)
    }

    private static func attachmentID(_ data: Data, position: Int) -> String {
        var hash = SHA256()
        hash.update(data: data)
        var positionBytes = UInt64(position).bigEndian
        withUnsafeBytes(of: &positionBytes) { hash.update(data: Data($0)) }
        var bytes = Array(hash.finalize().prefix(16))
        // UUID version 8 (custom SHA256 derivation), RFC 9562 variant.
        bytes[6] = (bytes[6] & 0x0f) | 0x80
        bytes[8] = (bytes[8] & 0x3f) | 0x80
        let hex = bytes.map { String(format: "%02x", $0) }
        return [hex[0..<4], hex[4..<6], hex[6..<8], hex[8..<10], hex[10..<16]]
            .map { $0.joined() }.joined(separator: "-")
    }

    private struct Style: Equatable {
        var bold = false
        var italic = false
        var strike = false
        var code = false
        var link: String?

        init(_ attributes: [NSAttributedString.Key: Any] = [:]) {
            if let font = attributes[.font] as? NSFont {
                let traits = font.fontDescriptor.symbolicTraits
                bold = traits.contains(.bold)
                italic = traits.contains(.italic)
                code = traits.contains(.monoSpace)
            }
            strike = (attributes[.strikethroughStyle] as? NSNumber)?.intValue != nil
                && (attributes[.strikethroughStyle] as? NSNumber)?.intValue != 0
            if let url = attributes[.link] as? URL {
                link = url.absoluteString
            } else if let value = attributes[.link] as? String, !value.isEmpty {
                link = value
            }
        }
    }

    private static func markdown(_ text: String, style: Style) -> String {
        var result = ""
        var line = ""
        // Keep CR, LF and CRLF exactly, with formatting delimiters inside each line.
        for character in text {
            if character.isNewline {
                result += markdownLine(line, style: style) + String(character)
                line = ""
            } else {
                line.append(character)
            }
        }
        return result + markdownLine(line, style: style)
    }

    private static func markdownLine(_ line: String, style: Style) -> String {
        guard let first = line.firstIndex(where: { !$0.isWhitespace }),
              let last = line.lastIndex(where: { !$0.isWhitespace }) else { return line }
        let end = line.index(after: last)
        let content = String(line[first..<end])
        var rendered: String
        if style.code {
            let longest = content.split(omittingEmptySubsequences: false, whereSeparator: { $0 != "`" })
                .map(\.count).max() ?? 0
            let delimiter = String(repeating: "`", count: longest + 1)
            let padding = content.first == "`" || content.last == "`" ? " " : ""
            rendered = delimiter + padding + content + padding + delimiter
        } else {
            rendered = escape(content)
        }
        if style.bold && style.italic { rendered = "***" + rendered + "***" }
        else if style.bold { rendered = "**" + rendered + "**" }
        else if style.italic { rendered = "*" + rendered + "*" }
        if style.strike { rendered = "~~" + rendered + "~~" }
        if let link = style.link { rendered = "[" + rendered + "](" + escapeDestination(link) + ")" }
        return String(line[..<first]) + rendered + String(line[end...])
    }

    private static func escape(_ text: String) -> String {
        let punctuation: Set<Character> = Set("\\`*_{}[]<>()#+-.!|~=&")
        return text.reduce(into: "") { result, character in
            if punctuation.contains(character) { result.append("\\") }
            result.append(character)
        }
    }

    private static func escapeDestination(_ value: String) -> String {
        value.unicodeScalars.reduce(into: "") { result, scalar in
            if CharacterSet.whitespacesAndNewlines.contains(scalar)
                || CharacterSet.controlCharacters.contains(scalar) || "\\()< >".unicodeScalars.contains(scalar) {
                result += String(scalar).utf8.map { String(format: "%%%02X", $0) }.joined()
            } else {
                result.unicodeScalars.append(scalar)
            }
        }
    }
}
