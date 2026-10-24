import AppKit
import CryptoKit
import Foundation
import SwiftData

@MainActor
enum NativeSelfTest {
    static func run() throws {
        let files = FileManager.default
        let directory = files.temporaryDirectory.appendingPathComponent("four-quadrants-self-test-" + UUID().uuidString)
        try files.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? files.removeItem(at: directory) }
        let store = directory.appendingPathComponent("FourQuadrants.store")
        let schema = LegacySchema.schema
        let container = try ModelContainer(for: schema, configurations: [
            ModelConfiguration("FourQuadrants", schema: schema, url: store, cloudKitDatabase: .none)
        ])
        let context = ModelContext(container)
        context.autosaveEnabled = false
        let date = DateCodec.parse("2026-10-12T00:00:00+08:00")!
        let parent = TaskItem(title: "Native export parent", weekStart: date)
        parent.taskUUID = "fixed-parent-uuid"
        parent.notes = "plain notes"
        parent.isUrgent = true
        parent.isImportant = true
        parent.dueDate = date.addingTimeInterval(86_400)
        parent.remindAt = date.addingTimeInterval(3_600)
        parent.createdAt = date
        parent.sortOrder = 2.5
        parent.links = ["https://example.test/a", "https://example.test/b"]
        parent.issueKey = "FQ-42"
        let child = TaskItem(title: "Child", weekStart: date)
        child.taskUUID = "fixed-child-uuid"
        child.isCompleted = true
        child.completedAt = date
        child.parent = parent
        let tag = Tag(name: "工作", colorHex: "#123456", sortOrder: 3)
        let note = TaskNote(text: "First progress note", createdAt: date)
        let secondNote = TaskNote(text: "Second progress note", createdAt: date.addingTimeInterval(1))
        let orphan = TaskNote(text: "Preserve unowned note", createdAt: date)
        let week = WeekNote(weekStart: date)
        for item in [parent, child] { context.insert(item) }
        context.insert(tag)
        context.insert(note)
        context.insert(secondNote)
        context.insert(orphan)
        context.insert(week)
        parent.tags = [tag]
        note.task = parent
        secondNote.task = parent
        let rich = try richTextFixture()
        parent.richContent = rich
        week.content = rich
        try context.save()
        let before = try fileDigests(directory, excluding: [])
        try check(before.keys.contains { $0.contains("_SUPPORT/") }, "Fixture uses external blob storage")
        let output = directory.appendingPathComponent("export.json")
        let first = try StoreExporter.export(source: store, output: output)
        try check(first.tasks.count == 2 && first.tags.count == 1 && first.weekNotes.count == 1, "Export model counts")
        let task = first.tasks.first { $0.id == parent.taskUUID }!
        try check(task.title == parent.title && task.notes == parent.notes && task.urgent && task.important,
                  "Task scalar mapping")
        try check(task.sortOrder == 2.5 && task.issueKey == "FQ-42" && task.links == parent.links,
                  "Task metadata mapping")
        try check(task.createdAt == DateCodec.iso(date) && task.dueDate == parent.dueDate.map(DateCodec.iso)
                  && task.remindAt == parent.remindAt.map(DateCodec.iso), "ISO8601 dates")
        try check(task.weekStart == DateCodec.day(date), "Local calendar weekStart")
        try check(task.richContent == rich.base64EncodedString(), "Exact task RTFD preservation")
        try check(first.weekNotes[0].richContent == rich.base64EncodedString()
                  && first.weekNotes[0].text.contains("Rich text fixture"), "Week RTFD and plain text")
        try check(first.tasks.first { $0.id == child.taskUUID }?.parentId == parent.taskUUID,
                  "Parent relationship")
        try check(task.tags == [first.tags[0].id] && task.progressNotes.count == 2, "Tags and progress notes")
        try check(first.orphanProgressNotes?.count == 1, "Orphan note preservation")
        let secondOutput = directory.appendingPathComponent("second.json")
        let second = try StoreExporter.export(source: store, output: secondOutput)
        try check(first.tags[0].id == second.tags[0].id, "Stable tag identity")
        try check(try Data(contentsOf: output) == Data(contentsOf: secondOutput), "Deterministic repeated export")
        let after = try fileDigests(directory, excluding: ["export.json", "second.json"])
        try check(before == after, "Source store/WAL/SHM/external blobs remain unchanged")
        do {
            _ = try StoreExporter.export(source: store, output: store)
            throw HelperError("Self-test failed: source overwrite was accepted")
        } catch let error as HelperError where !error.message.hasPrefix("Self-test failed:") {}

        let invalid = directory.appendingPathComponent("malformed.store")
        let badBytes = Data("this is not a SQLite store".utf8)
        try badBytes.write(to: invalid)
        let invalidOutput = directory.appendingPathComponent("invalid.json")
        let sentinel = Data("previous output must survive".utf8)
        try sentinel.write(to: invalidOutput)
        do {
            _ = try StoreExporter.export(source: invalid, output: invalidOutput)
            throw HelperError("Self-test failed: malformed store was accepted")
        } catch let error as HelperError where !error.message.hasPrefix("Self-test failed:") {}
        try check(try Data(contentsOf: invalid) == badBytes, "Malformed source remains unchanged")
        try check(try Data(contentsOf: invalidOutput) == sentinel, "Failed export preserves existing output")
        try noteMarkdownTests(directory)
        try reminderTests(directory)
        try parserTests()
        try backgroundTests(directory)
        withExtendedLifetime(container) {}
    }

    private static func richTextFixture() throws -> Data {
        let text = NSMutableAttributedString(string: "Rich text fixture\n", attributes: [
            .font: NSFont.boldSystemFont(ofSize: 18), .foregroundColor: NSColor.systemBlue
        ])
        // Large, uncompressed attachment makes Core Data exercise external blob storage.
        let attachment = NSTextAttachment()
        var bytes = Data(count: 768 * 768 * 4)
        bytes.withUnsafeMutableBytes { raw in
            for index in 0..<raw.count { raw[index] = UInt8.random(in: .min ... .max) }
        }
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 768, pixelsHigh: 768,
                                      bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                      isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 3072, bitsPerPixel: 32)!
        bytes.copyBytes(to: bitmap.bitmapData!, count: bytes.count)
        attachment.fileWrapper = FileWrapper(regularFileWithContents: bitmap.representation(using: .png, properties: [:])!)
        attachment.fileWrapper?.preferredFilename = "fixture.png"
        text.append(NSAttributedString(attachment: attachment))
        let encoded = try RichDocument.encode(text)
        let decoded = try RichDocument.decode(encoded)
        try check(decoded.string == text.string, "RTFD text round trip")
        try check(decoded.attribute(.attachment, at: decoded.length - 1, effectiveRange: nil) is NSTextAttachment,
                  "RTFD image attachment round trip")
        try check((decoded.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
            .map { NSFontManager.shared.traits(of: $0).contains(.boldFontMask) } == true,
                  "RTFD bold round trip")
        let formatted = NSMutableAttributedString(attributedString: decoded)
        let full = NSRange(location: 0, length: formatted.length)
        formatted.addAttribute(.strikethroughStyle, value: 1, range: full)
        formatted.addAttribute(.backgroundColor, value: NSColor.systemYellow, range: full)
        RichFormatting.changeFontSize(formatted, delta: 1)
        try check((formatted.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize == 19,
                  "Toolbar font size increment")
        RichFormatting.clear(formatted)
        try check(formatted.attribute(.strikethroughStyle, at: 0, effectiveRange: nil) == nil
                  && formatted.attribute(.backgroundColor, at: 0, effectiveRange: nil) == nil,
                  "Clear formatting removes decoration and highlight")
        try check(formatted.attribute(.attachment, at: formatted.length - 1, effectiveRange: nil) is NSTextAttachment,
                  "Clear formatting preserves images")
        let image = NSImage(data: attachment.fileWrapper!.regularFileContents!)!
        let resized = RichImages.make(image, width: 120)
        try check(abs((resized.attachmentCell?.cellSize().width ?? 0) - 120) < 0.5,
                  "Image resize controls TextKit cell size")
        let persistedImage = try RichDocument.decode(RichDocument.encode(NSAttributedString(attachment: resized)))
        let restored = persistedImage.attribute(.attachment, at: 0, effectiveRange: nil) as? NSTextAttachment
        try check(restored.flatMap(RichImages.image) != nil, "Resized image survives RTFD persistence")
        return encoded
    }

    private static func noteMarkdownTests(_ directory: URL) throws {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2,
                                      bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                      isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32)!
        for index in 0..<16 { bitmap.bitmapData![index] = index.isMultiple(of: 4) ? 255 : 128 }
        let png = bitmap.representation(using: .png, properties: [:])!
        let filename = "original [blue] image.png"
        let image = NSTextAttachment()
        image.fileWrapper = FileWrapper(regularFileWithContents: png)
        image.fileWrapper?.preferredFilename = filename
        let text = NSMutableAttributedString(string: "Bold", attributes: [.font: NSFont.boldSystemFont(ofSize: 14)])
        text.append(NSAttributedString(string: "\n"))
        text.append(NSAttributedString(string: "Italic", attributes: [
            .font: NSFontManager.shared.convert(NSFont.systemFont(ofSize: 14), toHaveTrait: .italicFontMask)
        ]))
        text.append(NSAttributedString(string: "\n"))
        text.append(NSAttributedString(string: "Strike", attributes: [.strikethroughStyle: 1]))
        text.append(NSAttributedString(string: "\n"))
        text.append(NSAttributedString(string: "x ` y", attributes: [.font: NSFont.monospacedSystemFont(ofSize: 14, weight: .regular)]))
        text.append(NSAttributedString(string: "\n"))
        text.append(NSAttributedString(string: "label [x]", attributes: [.link: URL(string: "https://example.test/a(b)?x=1&y=2")!]))
        text.append(NSAttributedString(string: "\n# [literal] *text* <tag> &amp;\n"))
        text.append(NSAttributedString(attachment: image))
        text.append(NSAttributedString(string: "\n"))
        text.append(NSAttributedString(attachment: image))
        let data = try RichDocument.encode(text)
        let input = directory.appendingPathComponent("note-markdown.rtfd")
        let output = directory.appendingPathComponent("note-markdown.json")
        let repeated = directory.appendingPathComponent("note-markdown-repeat.json")
        try data.write(to: input)
        let decoded = try RichDocument.decode(data)
        let first = try NoteMarkdown.export(input: input, output: output)
        let second = try NoteMarkdown.export(input: input, output: repeated)
        try check(first == second && Data(contentsOf: output) == Data(contentsOf: repeated),
                  "Deterministic Markdown extraction and JSON")
        try check(try JSONDecoder().decode(NoteMarkdownDocument.self, from: Data(contentsOf: output)) == first,
                  "Markdown output is Codable")
        try check(first.plainText == decoded.string && first.plainText.contains("\u{FFFC}"),
                  "Markdown plainText keeps full decoded text including attachment markers")
        try check(first.attachments.count == 2 && first.attachments.allSatisfy {
            $0.name == filename && $0.mimeType == "image/png" && Data(base64Encoded: $0.data) == png
        }, "Original image bytes and names preserved")
        try check(first.attachments.allSatisfy { UUID(uuidString: $0.id) != nil }
                  && first.attachments[0].id != first.attachments[1].id,
                  "Stable UUID attachment IDs include document position")
        try check(first.source.contains("**Bold**\n*Italic*\n~~Strike~~\n``x ` y``\n"),
                  "Markdown bold, italic, strikethrough, code and newlines")
        try check(first.source.contains(#"[label \[x\]](https://example.test/a%28b%29?x=1&y=2)"#)
                  && first.source.contains(##"\# \[literal\] \*text\* \<tag\> \&amp;"##),
                  "Markdown link destinations and literal text escaping")
        try check(first.source.contains("![original \\[blue\\] image\\.png](attachment:\(first.attachments[0].id))"),
                  "Markdown image attachment reference and label escaping")
        try check(try Data(contentsOf: input) == data, "Markdown extraction leaves original RTFD unchanged")

        let package = directory.appendingPathComponent("note-package.rtfd")
        let packageWrapper = try text.fileWrapper(from: NSRange(location: 0, length: text.length),
                                                 documentAttributes: [.documentType: NSAttributedString.DocumentType.rtfd])
        try packageWrapper.write(to: package, options: .atomic, originalContentsURL: nil)
        let packaged = try NoteMarkdown.export(input: package, output: directory.appendingPathComponent("note-package.json"))
        try check(packaged == first, "Package and flat RTFD extraction agree")
        try check(try NoteMarkdown.extract(NSAttributedString(string: "a\r\nb\rc\n")).source == "a\r\nb\rc\n",
                  "Markdown preserves all newline sequences")

        let binary = NSTextAttachment()
        let binaryBytes = Data([0, 255, 37, 10, 42])
        binary.fileWrapper = FileWrapper(regularFileWithContents: binaryBytes)
        binary.fileWrapper?.preferredFilename = "data [raw].bin"
        let binaryResult = try NoteMarkdown.extract(NSAttributedString(attachment: binary))
        try check(binaryResult.attachments[0].name == "data [raw].bin"
                  && binaryResult.attachments[0].mimeType == "application/octet-stream"
                  && Data(base64Encoded: binaryResult.attachments[0].data) == binaryBytes
                  && binaryResult.source.hasPrefix("[data \\[raw\\]\\.bin]"),
                  "Nonimage attachment bytes and normal Markdown link (MIME: \(binaryResult.attachments[0].mimeType), source: \(binaryResult.source))")
        let contentsOnly = NSTextAttachment(data: binaryBytes, ofType: "public.data")
        let contentsResult = try NoteMarkdown.extract(NSAttributedString(attachment: contentsOnly))
        try check(Data(base64Encoded: contentsResult.attachments[0].data) == binaryBytes,
                  "Attachment contents are preserved without a file wrapper")
        let directoryAttachment = NSTextAttachment()
        directoryAttachment.fileWrapper = FileWrapper(directoryWithFileWrappers: [
            "original.bin": FileWrapper(regularFileWithContents: binaryBytes)
        ])
        directoryAttachment.fileWrapper?.preferredFilename = "original.package"
        let packageResult = try NoteMarkdown.extract(NSAttributedString(attachment: directoryAttachment))
        let restored = FileWrapper(serializedRepresentation: Data(base64Encoded: packageResult.attachments[0].data)!)
        try check(try NoteMarkdown.extract(NSAttributedString(attachment: directoryAttachment)) == packageResult,
                  "Package attachment extraction is deterministic")
        try check(packageResult.attachments[0].mimeType == "application/octet-stream"
                  && restored?.fileWrappers?["original.bin"]?.regularFileContents == binaryBytes,
                  "Package attachment serialization preserves original contents")
        let displayOnly = NSTextAttachment()
        displayOnly.image = NSImage(data: png)
        let displayResult = try NoteMarkdown.extract(NSAttributedString(attachment: displayOnly))
        try check(displayResult.attachments[0].mimeType == "image/png"
                  && NSImage(data: Data(base64Encoded: displayResult.attachments[0].data)!) != nil,
                  "Display-only attachment falls back to valid PNG")

        let sentinel = Data("previous note output must survive".utf8)
        for invalid in [Data(), Data("this is not an RTFD document".utf8)] {
            let invalidInput = directory.appendingPathComponent("invalid-note.rtfd")
            try invalid.write(to: invalidInput)
            try sentinel.write(to: output)
            try rejectMarkdown("Malformed RTFD must fail") {
                _ = try NoteMarkdown.export(input: invalidInput, output: output)
            }
            try check(try Data(contentsOf: output) == sentinel, "Failed Markdown extraction preserves existing output")
            try check(try Data(contentsOf: invalidInput) == invalid, "Malformed RTFD input remains untouched")
        }
        try rejectMarkdown("Missing RTFD must fail") {
            _ = try NoteMarkdown.export(input: directory.appendingPathComponent("missing.rtfd"), output: output)
        }
        try rejectMarkdown("Markdown export must not overwrite original") {
            _ = try NoteMarkdown.export(input: input, output: input)
        }
        try rejectMarkdown("Markdown export must not write inside original package") {
            _ = try NoteMarkdown.export(input: package, output: package.appendingPathComponent("output.json"))
        }
        try rejectMarkdown("Unrecoverable attachment must fail") {
            _ = try NoteMarkdown.extract(NSAttributedString(attachment: NSTextAttachment()))
        }
        try rejectMarkdown("Orphan attachment marker must fail") {
            _ = try NoteMarkdown.extract(NSAttributedString(string: "\u{FFFC}"))
        }
        let oversized = NSTextAttachment(data: Data(count: NoteMarkdown.attachmentLimit + 1), ofType: "public.data")
        try rejectMarkdown("Oversized attachment must fail") {
            _ = try NoteMarkdown.extract(NSAttributedString(attachment: oversized))
        }
        let large = NSTextAttachment(data: Data(count: 17 * 1024 * 1024), ofType: "public.data")
        let total = NSMutableAttributedString(string: "")
        for _ in 0..<3 { total.append(NSAttributedString(attachment: large)) }
        try rejectMarkdown("Total attachment size must be limited") { _ = try NoteMarkdown.extract(total) }
    }

    private static func rejectMarkdown(_ message: String, action: () throws -> Void) throws {
        do { try action() } catch { return }
        throw HelperError("Self-test failed: " + message)
    }

    private static func reminderTests(_ directory: URL) throws {
        let url = directory.appendingPathComponent("reminders.json")
        let now = DateCodec.parse("2026-10-12T00:00:00Z")!
        for schemaVersion in [1, 2] {
            let json = """
            {"schemaVersion":\(schemaVersion),"tasks":[
              {"id":"future","title":"Future","completed":false,"remindAt":"2030-01-01T00:00:00.000Z"},
              {"id":"done","title":"Done","completed":true,"remindAt":"2030-01-01T00:00:00Z"},
              {"id":"past","title":"Past","completed":false,"remindAt":"2020-01-01T00:00:00Z"},
              {"id":"none","title":"None","completed":false}
            ]}
            """
            try Data(json.utf8).write(to: url)
            let plan = try ReminderPlan.load(url, now: now)
            try check(plan.pending.map(\.task.id) == ["future"],
                      "Schema \(schemaVersion) skips completed, missing and expired reminders")
            try check(plan.pending[0].date == DateCodec.parse("2030-01-01T00:00:00Z")!,
                      "Schema \(schemaVersion) preserves reminder timestamp")
            try check(plan.activeIDs == ["task-future", "task-past"],
                      "Overdue unfinished reminders retain ownership of delivered notifications")
            let delivered = ["task-future", "task-past", "task-done", "task-none", "task-deleted", "instant-other"]
            try check(plan.obsoleteDelivered(delivered) == ["task-done", "task-none", "task-deleted"],
                      "Sync only dismisses completed, cleared or deleted tasks, preserving unrelated notifications")
            let later = try ReminderPlan.load(url, now: DateCodec.parse("2031-01-01T00:00:00Z")!)
            try check(later.pending.isEmpty && later.obsoleteDelivered(delivered) == plan.obsoleteDelivered(delivered),
                      "Repeated sync after delivery does not erase an unfinished reminder")
            for tasks in ["[]", #"[{"id":"none","title":"None","completed":false}]"#] {
                try Data("{\"schemaVersion\":\(schemaVersion),\"tasks\":\(tasks)}".utf8).write(to: url)
                try check(try ReminderPlan.load(url, now: now).pending.isEmpty,
                          "Schema \(schemaVersion) accepts documents without reminders")
            }
            try Data(json.replacingOccurrences(of: "2030-01-01T00:00:00.000Z", with: "invalid").utf8).write(to: url)
            do {
                _ = try ReminderPlan.load(url, now: now)
                throw HelperError("Self-test failed: schema \(schemaVersion) accepted a malformed reminder timestamp")
            } catch let error as HelperError where error.message == "Task future has an invalid ISO8601 remindAt value." {}
        }
        let boundaries = """
        {"schemaVersion":2,"tasks":[
          {"id":"offset","title":"Offset","completed":false,"remindAt":"2026-10-12T08:00:01+08:00"},
          {"id":"fraction","title":"Fraction","completed":false,"remindAt":"2026-10-12T00:00:00.500Z"},
          {"id":"now","title":"Now","completed":false,"remindAt":"2026-10-12T00:00:00Z"}
        ]}
        """
        try Data(boundaries.utf8).write(to: url)
        let boundaryPlan = try ReminderPlan.load(url, now: now)
        try check(boundaryPlan.pending.map(\.task.id) == ["fraction", "offset"],
                  "Fractional seconds and offsets preserve chronological order; exact-now is not scheduled")
        try check(boundaryPlan.pending[1].date == now.addingTimeInterval(1), "Offset date represents the same UTC instant")
        for tasks in [#"[{"id":"","title":"Empty","completed":false}]"#,
                      #"[{"id":"same","title":"One","completed":false},{"id":"same","title":"Two","completed":true}]"#] {
            try Data("{\"schemaVersion\":2,\"tasks\":\(tasks)}".utf8).write(to: url)
            do {
                _ = try ReminderPlan.load(url, now: now)
                throw HelperError("Self-test failed: duplicate/empty reminder id accepted")
            } catch let error as HelperError where error.message == "Reminder JSON contains an empty or duplicate task id." {}
        }
        try check(HelperCLI.isNotificationBundle(HelperCLI.bundleID)
                  && HelperCLI.isNotificationBundle(HelperCLI.bundleID + ".test.fixture")
                  && !HelperCLI.isNotificationBundle(nil)
                  && !HelperCLI.isNotificationBundle("com.fourquadrants.gpui")
                  && !HelperCLI.isNotificationBundle(HelperCLI.bundleID + "-other"),
                  "CLI reexec recognizes only the helper and isolated test identities")
        for schemaVersion in [0, 3] {
            try Data("{\"schemaVersion\":\(schemaVersion),\"tasks\":[]}".utf8).write(to: url)
            do {
                _ = try ReminderPlan.load(url, now: now)
                throw HelperError("Self-test failed: unsupported reminder schema \(schemaVersion) was accepted")
            } catch let error as HelperError where error.message == "Unsupported reminder JSON schemaVersion." {}
        }
    }

    private static func parserTests() throws {
        let defaults = try JSONDecoder().decode(ParseSettings.self, from: Data(#"{"builtinParseTagName":" 工作 "}"#.utf8))
        let builtin = InputParser.parse(settings: defaults, text: "ABC-123 修复图片\n 第二行 https://example.test/a")
        try check(builtin.issueKey == "ABC-123" && builtin.title == "修复图片 第二行"
                  && builtin.links == ["https://example.test/a"] && builtin.tagName == "工作", "Builtin parser parity")
        let mid = InputParser.parse(settings: defaults, text: "修复 ABC-123 https://example.test/a")
        try check(mid.issueKey == nil && mid.title == "修复 ABC-123", "Issue key only at beginning")
        let settings = try JSONDecoder().decode(ParseSettings.self, from: Data(#"{"parseRules":[{"pattern":"["},{"pattern":"^(.*)END$","titleGroup":1,"tagName":"custom"}]}"#.utf8))
        let custom = InputParser.parse(settings: settings, text: "第一行\n第二行END")
        try check(custom.title == "第一行\n第二行" && custom.tagName == "custom", "Dot-all custom regex and invalid rule fallback")
        let groups = try JSONDecoder().decode(ParseSettings.self, from: Data(#"{"parseRules":[{"pattern":"^(ABC-42) (.+) (https://.*)$","issueGroup":1,"titleGroup":2,"linkGroup":3,"tagName":"issue"}]}"#.utf8))
        let matched = InputParser.parse(settings: groups, text: "ABC-42 修复标题 https://example.test/a")
        try check(matched.issueKey == "ABC-42" && matched.title == "修复标题"
                  && matched.links == ["https://example.test/a"] && matched.tagName == "issue", "Custom capture groups")
    }

    private static func backgroundTests(_ directory: URL) throws {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 16, pixelsHigh: 12,
                                      bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                      isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 64, bitsPerPixel: 32)!
        for x in 0..<16 {
            for y in 0..<12 {
                let pixel = bitmap.bitmapData!.advanced(by: y * 64 + x * 4)
                pixel[0] = x < 8 ? 255 : 0
                pixel[1] = 0
                pixel[2] = x < 8 ? 0 : 255
                pixel[3] = 255
            }
        }
        let input = directory.appendingPathComponent("background.png")
        let output = directory.appendingPathComponent("background-blurred.png")
        try bitmap.representation(using: .png, properties: [:])!.write(to: input)
        let size = try BackgroundImage.prepare(input: input, output: output, blur: 4)
        let rendered = NSBitmapImageRep(data: try Data(contentsOf: output))!
        try check(size.width == 16 && size.height == 12, "Background retains original size")
        try check(rendered.colorAt(x: 0, y: 0)!.alphaComponent > 0.99, "Clamped blur keeps opaque edges")
        let middle = rendered.colorAt(x: 7, y: 6)!.usingColorSpace(.deviceRGB)!
        try check(middle.redComponent > 0.1 && middle.blueComponent > 0.1, "Gaussian blur mixes neighboring pixels")
        for invalid in [Double.nan, Double.infinity, -1, 41] {
            do {
                _ = try BackgroundImage.prepare(input: input, output: output, blur: invalid)
                throw HelperError("Self-test failed: invalid blur accepted")
            } catch let error as HelperError where !error.message.hasPrefix("Self-test failed:") {}
        }
    }

    private static func fileDigests(_ directory: URL, excluding: Set<String>) throws -> [String: String] {
        let enumerator = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: [.isRegularFileKey])!
        var result: [String: String] = [:]
        for case let url as URL in enumerator {
            guard !excluding.contains(url.lastPathComponent),
                  try url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else { continue }
            result[url.path] = SHA256.hash(data: try Data(contentsOf: url)).map { String(format: "%02x", $0) }.joined()
        }
        return result
    }

    private static func check(_ condition: @autoclosure () throws -> Bool, _ message: String) throws {
        guard try condition() else { throw HelperError("Self-test failed: " + message) }
    }
}
