import AppKit
import Foundation
@preconcurrency import UserNotifications

struct ReminderDocument: Decodable {
    let schemaVersion: Int
    let tasks: [ReminderTask]
}

struct ReminderTask: Decodable {
    let id: String
    let title: String
    let notes: String?
    let issueKey: String?
    let completed: Bool
    let remindAt: String?
}

struct PlannedReminder {
    let task: ReminderTask
    let date: Date
    var identifier: String { "task-" + task.id }
}

enum ReminderPlan {
    static func load(_ url: URL, now: Date = .now) throws -> [PlannedReminder] {
        let document = try JSONDecoder().decode(ReminderDocument.self, from: Data(contentsOf: url))
        guard (1...2).contains(document.schemaVersion) else { throw HelperError("Unsupported reminder JSON schemaVersion.") }
        var seen = Set<String>()
        var result: [PlannedReminder] = []
        for task in document.tasks {
            guard !task.id.isEmpty, seen.insert(task.id).inserted else {
                throw HelperError("Reminder JSON contains an empty or duplicate task id.")
            }
            guard let value = task.remindAt else { continue }
            guard let date = DateCodec.parse(value) else {
                throw HelperError("Task \(task.id) has an invalid ISO8601 remindAt value.")
            }
            if !task.completed && date > now { result.append(PlannedReminder(task: task, date: date)) }
        }
        return result.sorted { ($0.date, $0.identifier) < ($1.date, $1.identifier) }
    }
}

@MainActor
final class Notifications: NSObject, UNUserNotificationCenterDelegate {
    private let center: UNUserNotificationCenter

    override init() {
        center = UNUserNotificationCenter.current()
        super.init()
        center.delegate = self
    }

    private func authorize() async throws {
        let settings = await center.notificationSettings()
        switch settings.authorizationStatus {
        case .notDetermined:
            guard try await center.requestAuthorization(options: [.alert, .sound, .badge]) else {
                throw HelperError("Notification permission was declined. Enable Four Quadrants Helper in System Settings > Notifications.")
            }
        case .denied:
            throw HelperError("Notifications are disabled. Enable Four Quadrants Helper in System Settings > Notifications.")
        case .authorized, .provisional, .ephemeral:
            break
        @unknown default:
            throw HelperError("Unsupported notification authorization status.")
        }
    }

    func status() async {
        let settings = await center.notificationSettings()
        let pending = await center.pendingNotificationRequests()
        HelperCLI.printJSON(["authorizationStatus": settings.authorizationStatus.rawValue,
                             "pendingCount": pending.count,
                             "bundleIdentifier": Bundle.main.bundleIdentifier ?? ""])
    }

    func notify(id: String, title: String, body: String) async throws {
        guard !id.isEmpty else { throw HelperError("Notification id must not be empty.") }
        try await authorize()
        let content = UNMutableNotificationContent()
        content.title = title.isEmpty ? "Four Quadrants" : title
        content.body = body
        content.sound = .default
        let request = UNNotificationRequest(identifier: "instant-" + id, content: content, trigger: nil)
        try await center.add(request)
        HelperCLI.printJSON(["scheduled": true, "id": id])
    }

    func sync(_ plan: [PlannedReminder]) async throws {
        // No permission prompt is needed just to remove obsolete reminders.
        if !plan.isEmpty { try await authorize() }
        let wanted = Set(plan.map(\.identifier))
        let pending = await center.pendingNotificationRequests()
        let obsolete = pending.map(\.identifier).filter { $0.hasPrefix("task-") && !wanted.contains($0) }
        center.removePendingNotificationRequests(withIdentifiers: obsolete)
        let delivered = await center.deliveredNotifications()
        let obsoleteDelivered = delivered.map { $0.request.identifier }
            .filter { $0.hasPrefix("task-") && !wanted.contains($0) }
        center.removeDeliveredNotifications(withIdentifiers: obsoleteDelivered)

        for reminder in plan {
            // Scheduling takes time; skip reminders which expired during authorization.
            guard reminder.date > .now else {
                center.removePendingNotificationRequests(withIdentifiers: [reminder.identifier])
                continue
            }
            let content = UNMutableNotificationContent()
            content.title = reminder.task.title.isEmpty ? "Four Quadrants" : reminder.task.title
            content.body = reminder.task.notes ?? ""
            content.subtitle = reminder.task.issueKey ?? ""
            content.sound = .default
            content.userInfo = ["taskId": reminder.task.id]
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = .current
            var components = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: reminder.date)
            components.calendar = calendar
            components.timeZone = calendar.timeZone
            let trigger = UNCalendarNotificationTrigger(dateMatching: components, repeats: false)
            guard trigger.nextTriggerDate() != nil else {
                throw HelperError("Cannot form a calendar notification for task \(reminder.task.id).")
            }
            try await center.add(UNNotificationRequest(identifier: reminder.identifier, content: content, trigger: trigger))
        }
        // This async round trip also waits for queued removals. Detect system request limits
        // rather than claiming that all reminders were scheduled when some were discarded.
        let after = await center.pendingNotificationRequests()
        let afterIDs = Set(after.map(\.identifier))
        let stillFuture = Set(plan.filter { $0.date > Date.now.addingTimeInterval(1) }.map(\.identifier))
        let missing = stillFuture.subtracting(afterIDs)
        guard missing.isEmpty else {
            throw HelperError("macOS did not retain \(missing.count) reminders (system pending-notification limit may have been reached).")
        }
        guard !afterIDs.contains(where: { $0.hasPrefix("task-") && !wanted.contains($0) }) else {
            throw HelperError("macOS has not removed all obsolete reminders; retry sync-reminders.")
        }
        HelperCLI.printJSON(["scheduled": afterIDs.intersection(wanted).count, "removed": obsolete.count])
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}
