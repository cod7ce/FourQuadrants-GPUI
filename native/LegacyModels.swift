import Foundation
import SwiftData

// The persistent names, types, defaults and relationships mirror the legacy
// Sources/Models schema. Keep these independent of the new application's model.
@Model
final class TaskItem {
    var title: String = ""
    var notes: String = ""
    var isUrgent: Bool = false
    var isImportant: Bool = false
    var isCompleted: Bool = false
    var completedAt: Date?
    var dueDate: Date?
    var remindAt: Date?
    var createdAt: Date = Date.now
    var sortOrder: Double = 0
    var weekStart: Date = Date.now
    var links: [String] = []
    var issueKey: String?
    var taskUUID: String = UUID().uuidString
    var parent: TaskItem?
    @Relationship(deleteRule: .cascade, inverse: \TaskItem.parent)
    var subtasks: [TaskItem]? = []
    @Relationship(inverse: \Tag.tasks)
    var tags: [Tag]? = []
    @Relationship(deleteRule: .cascade, inverse: \TaskNote.task)
    var progressNotes: [TaskNote]? = []
    @Attribute(.externalStorage) var richContent: Data?

    init(title: String = "", weekStart: Date = .now) {
        self.title = title
        self.weekStart = weekStart
    }
}

@Model
final class Tag {
    var name: String = ""
    var colorHex: String = "#1E88E5"
    var sortOrder: Double = 0
    var tasks: [TaskItem]? = []

    init(name: String = "", colorHex: String = "#1E88E5", sortOrder: Double = 0) {
        self.name = name
        self.colorHex = colorHex
        self.sortOrder = sortOrder
    }
}

@Model
final class WeekNote {
    var weekStart: Date = Date.now
    @Attribute(.externalStorage) var content: Data?

    init(weekStart: Date) { self.weekStart = weekStart }
}

@Model
final class TaskNote {
    var text: String = ""
    var createdAt: Date = Date.now
    var task: TaskItem?

    init(text: String = "", createdAt: Date = .now) {
        self.text = text
        self.createdAt = createdAt
    }
}

enum LegacySchema {
    static var schema: Schema { Schema([TaskItem.self, Tag.self, WeekNote.self, TaskNote.self]) }
}
