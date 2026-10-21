import AppKit
import CryptoKit
import Foundation
import SQLite3
import SwiftData

struct HelperError: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}

enum DateCodec {
    static func iso(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date)
    }

    static func parse(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let result = formatter.date(from: value) { return result }
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: value)
    }

    static func day(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar.current
        formatter.timeZone = Calendar.current.timeZone
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: date)
    }
}

struct ExportProgressNote: Encodable {
    let id: String
    let text: String
    let createdAt: String
}

struct ExportTask: Encodable {
    let id: String
    let title: String
    let notes: String
    let urgent: Bool
    let important: Bool
    let completed: Bool
    let completedAt: String?
    let dueDate: String?
    let remindAt: String?
    let createdAt: String
    let sortOrder: Double
    let weekStart: String
    let parentId: String?
    let tags: [String]
    let links: [String]
    let issueKey: String?
    let richContent: String?
    let progressNotes: [ExportProgressNote]
}

struct ExportTag: Encodable {
    let id: String
    let name: String
    let colorHex: String
    let sortOrder: Double
}

struct ExportWeekNote: Encodable {
    let weekStart: String
    let text: String
    let richContent: String?
}

struct ExportDocument: Encodable {
    let schemaVersion = 1
    let tasks: [ExportTask]
    let tags: [ExportTag]
    let weekNotes: [ExportWeekNote]
    let settings: [String: String] = [:]
    // Corrupt/old stores may have unowned notes. Preserve them instead of discarding.
    let orphanProgressNotes: [ExportProgressNote]?
}

/// The source is only read using file APIs, never opened by SQLite/SwiftData.
/// Core Data migration and WAL recovery happen exclusively in this private copy.
final class StoreSnapshot {
    let directory: URL
    let store: URL
    private let files = FileManager.default

    static func relatedFiles(for source: URL) -> [URL] {
        let parent = source.deletingLastPathComponent()
        let basename = source.lastPathComponent
        let stem = source.deletingPathExtension().lastPathComponent
        let names = [basename, basename + "-wal", basename + "-shm", basename + "-journal",
                     "." + basename + "_SUPPORT", basename + "_SUPPORT",
                     "." + stem + "_SUPPORT", stem + "_SUPPORT"]
        return Array(Set(names)).sorted().map { parent.appendingPathComponent($0) }
    }

    init(source: URL) throws {
        let files = FileManager.default
        let source = source.resolvingSymlinksInPath().standardizedFileURL
        let attributes = try files.attributesOfItem(atPath: source.path)
        guard attributes[.type] as? FileAttributeType == .typeRegular else {
            throw HelperError("Source must be a regular SwiftData .store file.")
        }
        let handle = try FileHandle(forReadingFrom: source)
        defer { try? handle.close() }
        guard try handle.read(upToCount: 16) == Data("SQLite format 3\0".utf8) else {
            throw HelperError("The selected file is not a SQLite SwiftData store.")
        }
        directory = files.temporaryDirectory.appendingPathComponent("four-quadrants-export-" + UUID().uuidString,
                                                                      isDirectory: true)
        store = directory.appendingPathComponent(source.lastPathComponent)
        try files.createDirectory(at: directory, withIntermediateDirectories: false,
                                  attributes: [.posixPermissions: 0o700])
        do {
            let related = Self.relatedFiles(for: source)
            let before = try Self.fingerprints(related)
            for url in related where files.fileExists(atPath: url.path) {
                try files.copyItem(at: url, to: directory.appendingPathComponent(url.lastPathComponent))
            }
            guard try Self.fingerprints(related) == before else {
                throw HelperError("The source store changed during copying. Close the old FourQuadrants app and retry.")
            }
            // Reject copied symlinks before any persistence framework can follow them
            // back to original external-storage files.
            if let enumerator = files.enumerator(at: directory, includingPropertiesForKeys: [.isSymbolicLinkKey]) {
                for case let url as URL in enumerator {
                    if try url.resourceValues(forKeys: [.isSymbolicLinkKey]).isSymbolicLink == true {
                        throw HelperError("The store contains symbolic links; export from an ordinary local copy.")
                    }
                }
            }
            try Self.validateSQLiteCopy(store)
        } catch {
            try? files.removeItem(at: directory)
            throw error
        }
    }

    deinit { try? files.removeItem(at: directory) }

    private static func fingerprints(_ roots: [URL]) throws -> [String: String] {
        let files = FileManager.default
        var result: [String: String] = [:]
        for root in roots where files.fileExists(atPath: root.path) {
            var urls = [root]
            var isDirectory: ObjCBool = false
            _ = files.fileExists(atPath: root.path, isDirectory: &isDirectory)
            if isDirectory.boolValue,
               let enumerator = files.enumerator(at: root, includingPropertiesForKeys: nil) {
                urls += enumerator.allObjects.compactMap { $0 as? URL }
            }
            for url in urls {
                let attributes = try files.attributesOfItem(atPath: url.path)
                result[url.path] = "\(attributes[.size] ?? 0)-\(attributes[.modificationDate] ?? "")-\(attributes[.systemFileNumber] ?? 0)"
            }
        }
        return result
    }

    private static func validateSQLiteCopy(_ url: URL) throws {
        var database: OpaquePointer?
        guard sqlite3_open_v2(url.path, &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            if let database { sqlite3_close(database) }
            throw HelperError("The temporary store copy cannot be opened as SQLite.")
        }
        defer { sqlite3_close(database) }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "PRAGMA quick_check", -1, &statement, nil) == SQLITE_OK else {
            throw HelperError("The temporary store copy is not a valid SQLite database.")
        }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_step(statement) == SQLITE_ROW,
              let check = sqlite3_column_text(statement, 0), String(cString: check) == "ok" else {
            throw HelperError("The temporary store copy failed SQLite integrity checking. Close the old app and retry.")
        }
        var entities: OpaquePointer?
        guard sqlite3_prepare_v2(database, "SELECT Z_NAME FROM Z_PRIMARYKEY", -1, &entities, nil) == SQLITE_OK else {
            throw HelperError("The selected SQLite database is not a Core Data / SwiftData store.")
        }
        defer { sqlite3_finalize(entities) }
        var names = Set<String>()
        while sqlite3_step(entities) == SQLITE_ROW {
            if let name = sqlite3_column_text(entities, 0) { names.insert(String(cString: name)) }
        }
        guard names.contains("TaskItem") else {
            throw HelperError("The selected store does not contain the FourQuadrants TaskItem model.")
        }
    }
}

@MainActor
enum StoreExporter {
    static func export(source: URL, output: URL) throws -> ExportDocument {
        let canonicalSource = source.resolvingSymlinksInPath().standardizedFileURL
        let canonicalOutput = output.resolvingSymlinksInPath().standardizedFileURL
        for protected in StoreSnapshot.relatedFiles(for: canonicalSource) {
            let path = protected.resolvingSymlinksInPath().standardizedFileURL.path
            guard canonicalOutput.path != path, !canonicalOutput.path.hasPrefix(path + "/") else {
                throw HelperError("The JSON destination must not overwrite any source-store file or support directory.")
            }
        }
        let snapshot = try StoreSnapshot(source: canonicalSource)
        // Keep snapshot alive until all external blobs and relationships are materialized.
        let document = try withExtendedLifetime(snapshot) { try readCopy(snapshot.store) }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        try encoder.encode(document).write(to: output, options: .atomic)
        return document
    }

    private static func stableID(_ identifier: PersistentIdentifier, prefix: String) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let hash = SHA256.hash(data: try encoder.encode(identifier))
        return prefix + "-" + hash.map { String(format: "%02x", $0) }.joined()
    }

    private static func readCopy(_ store: URL) throws -> ExportDocument {
        let schema = LegacySchema.schema
        let configuration = ModelConfiguration("FourQuadrants", schema: schema, url: store,
                                               allowsSave: true, cloudKitDatabase: .none)
        let container = try ModelContainer(for: schema, configurations: [configuration])
        let context = ModelContext(container)
        context.autosaveEnabled = false
        let tasks = try context.fetch(FetchDescriptor<TaskItem>())
        let tags = try context.fetch(FetchDescriptor<Tag>())
        let weeks = try context.fetch(FetchDescriptor<WeekNote>())
        let notes = try context.fetch(FetchDescriptor<TaskNote>())
        var tagIDs: [PersistentIdentifier: String] = [:]
        let exportedTags = try tags.map { tag in
            let id = try stableID(tag.persistentModelID, prefix: "tag")
            tagIDs[tag.persistentModelID] = id
            return ExportTag(id: id, name: tag.name, colorHex: tag.colorHex, sortOrder: tag.sortOrder)
        }.sorted { ($0.sortOrder, $0.id) < ($1.sortOrder, $1.id) }
        func exportNote(_ note: TaskNote) throws -> ExportProgressNote {
            ExportProgressNote(id: try stableID(note.persistentModelID, prefix: "note"),
                               text: note.text, createdAt: DateCodec.iso(note.createdAt))
        }
        let exportedTasks = try tasks.map { task in
            let ids = try (task.tags ?? []).map { tag -> String in
                guard let id = tagIDs[tag.persistentModelID] else {
                    throw HelperError("A task refers to a missing tag; export stopped to avoid losing data.")
                }
                return id
            }
            return ExportTask(id: task.taskUUID, title: task.title, notes: task.notes,
                              urgent: task.isUrgent, important: task.isImportant, completed: task.isCompleted,
                              completedAt: task.completedAt.map(DateCodec.iso), dueDate: task.dueDate.map(DateCodec.iso),
                              remindAt: task.remindAt.map(DateCodec.iso), createdAt: DateCodec.iso(task.createdAt),
                              sortOrder: task.sortOrder, weekStart: DateCodec.day(task.weekStart),
                              parentId: task.parent?.taskUUID, tags: ids.sorted(), links: task.links,
                              issueKey: task.issueKey, richContent: task.richContent?.base64EncodedString(),
                              progressNotes: try (task.progressNotes ?? []).map(exportNote)
                                .sorted { ($0.createdAt, $0.id) > ($1.createdAt, $1.id) })
        }.sorted { ($0.weekStart, $0.sortOrder, $0.id) < ($1.weekStart, $1.sortOrder, $1.id) }
        let exportedWeeks = weeks.map { week in
            ExportWeekNote(weekStart: DateCodec.day(week.weekStart), text: plainText(week.content),
                           richContent: week.content?.base64EncodedString())
        }.sorted { $0.weekStart < $1.weekStart }
        let orphans = try notes.filter { $0.task == nil }.map(exportNote)
        return ExportDocument(tasks: exportedTasks, tags: exportedTags, weekNotes: exportedWeeks,
                              orphanProgressNotes: orphans.isEmpty ? nil : orphans)
    }

    private static func plainText(_ data: Data?) -> String {
        guard let data, !data.isEmpty else { return "" }
        do { return try RichDocument.decode(data).string }
        catch {
            FileHandle.standardError.write(Data("Warning: a week note has invalid RTFD; its original bytes were preserved.\n".utf8))
            return ""
        }
    }
}
