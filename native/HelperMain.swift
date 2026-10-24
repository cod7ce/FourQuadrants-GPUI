import AppKit
import Foundation

@main
@MainActor
enum HelperCLI {
    private static var editor: RichEditor?
    private static var notifications: Notifications?
    static let bundleID = "com.fourquadrants.gpui.native-helper"
    static let help = """
    Four Quadrants native helper (macOS 14+)
      export-store <source.store> <output.json>
      edit-rich <input-rtfd-path> <output-rtfd-path> <title>
      note-markdown <input.rtfd> <output.json>
      notify <id> <title> <body>
      sync-reminders <data.json>
      notification-status
      prepare-background <input-image> <output.png> <blur:0..40>
      parse-input <rules.json> <textfile> <output.json>
      self-test

    export-store reads a temporary copy of the store, WAL/SHM and external blobs;
    quit the legacy app first for a consistent snapshot. Output is schemaVersion 1.
    edit-rich reads/writes flat RTFD bytes, including embedded images. Missing input
    starts a blank document. Save writes output; close exits with {saved, output}.
    note-markdown extracts Markdown and original attachment bytes without opening
    windows or modifying the RTFD. Output contains plainText, source, attachments.
    Notification commands automatically use the adjacent FourQuadrantsHelper.app.
    sync-reminders uses UserNotifications calendar triggers, not the Reminders app.
    Success is JSON on stdout, failures on stderr with nonzero exit status.
    """

    static func main() {
        do {
            let args = Array(CommandLine.arguments.dropFirst())
            guard let command = args.first else { print(help); return }
            switch command {
            case "help", "--help", "-h": print(help)
            case "export-store":
                try require(args, count: 3)
                let document = try StoreExporter.export(source: file(args[1]), output: file(args[2]))
                printJSON(["exported": true, "tasks": document.tasks.count, "tags": document.tags.count,
                           "weekNotes": document.weekNotes.count])
            case "edit-rich":
                try require(args, count: 4)
                editor = try RichEditor(input: file(args[1]), output: file(args[2]), title: args[3])
                editor?.show()
                NSApplication.shared.run()
            case "note-markdown":
                try require(args, count: 3)
                let document = try NoteMarkdown.export(input: file(args[1]), output: file(args[2]))
                printJSON(["extracted": true, "attachments": document.attachments.count])
            case "notify":
                try require(args, count: 4)
                try runBundledIfNeeded(args)
                beginNotifications {
                    try await $0.notify(id: args[1], title: args[2], body: args[3])
                }
            case "sync-reminders":
                try require(args, count: 2)
                // Parse before bundle/permission side effects, keeping invalid JSON harmless.
                let plan = try ReminderPlan.load(file(args[1]))
                try runBundledIfNeeded(args)
                beginNotifications { try await $0.sync(plan) }
            case "notification-status":
                try require(args, count: 1)
                try runBundledIfNeeded(args)
                beginNotifications { await $0.status() }
            case "prepare-background":
                try require(args, count: 4)
                guard let blur = Double(args[3]) else { throw HelperError("Background blur must be a number from 0 through 40.") }
                let dimensions = try BackgroundImage.prepare(input: file(args[1]), output: file(args[2]), blur: blur)
                printJSON(["prepared": true, "width": dimensions.width, "height": dimensions.height])
            case "parse-input":
                try require(args, count: 4)
                try InputParser.export(rules: file(args[1]), input: file(args[2]), output: file(args[3]))
                printJSON(["parsed": true])
            case "self-test":
                try require(args, count: 1)
                try NativeSelfTest.run()
                printJSON(["passed": true])
            default:
                throw HelperError("Unknown command: \(command)\n\(help)")
            }
        } catch { fail(error) }
    }

    private static func file(_ path: String) -> URL {
        URL(fileURLWithPath: NSString(string: path).expandingTildeInPath).standardizedFileURL
    }

    private static func require(_ args: [String], count: Int) throws {
        guard args.count == count else { throw HelperError("Invalid arguments.\n\(help)") }
    }

    static func isNotificationBundle(_ identifier: String?) -> Bool {
        guard let identifier else { return false }
        return identifier == bundleID || identifier.hasPrefix(bundleID + ".test.")
    }

    private static func runBundledIfNeeded(_ args: [String]) throws {
        if isNotificationBundle(Bundle.main.bundleIdentifier),
           Bundle.main.bundleURL.pathExtension == "app" { return }
        let executable = Bundle.main.executableURL ?? file(CommandLine.arguments[0])
        let bundled = executable.deletingLastPathComponent()
            .appendingPathComponent("FourQuadrantsHelper.app/Contents/MacOS/four-quadrants-helper")
        guard FileManager.default.isExecutableFile(atPath: bundled.path) else {
            throw HelperError("Notifications require FourQuadrantsHelper.app next to the CLI. Run scripts/build-native.sh and keep the generated bundle beside it.")
        }
        let child = Process()
        child.executableURL = bundled
        child.arguments = args
        child.standardInput = FileHandle.standardInput
        child.standardOutput = FileHandle.standardOutput
        child.standardError = FileHandle.standardError
        try child.run()
        child.waitUntilExit()
        exit(child.terminationReason == .exit ? child.terminationStatus : EXIT_FAILURE)
    }

    private static func beginNotifications(_ action: @escaping @MainActor (Notifications) async throws -> Void) {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        notifications = Notifications()
        Task { @MainActor in
            do {
                try await action(notifications!)
                exit(EXIT_SUCCESS)
            } catch { fail(error) }
        }
        app.run()
    }

    static func printJSON(_ object: [String: Any]) {
        if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
           let string = String(data: data, encoding: .utf8) { print(string) }
    }

    private static func fail(_ error: Error) -> Never {
        FileHandle.standardError.write(Data(("four-quadrants-helper: " + error.localizedDescription + "\n").utf8))
        exit(EXIT_FAILURE)
    }
}
