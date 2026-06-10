import UserNotifications

class NotificationManager: NSObject {
    static let shared = NotificationManager()

    private override init() {
        super.init()
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.requestAuthorization(options: [.alert, .badge, .sound]) { granted, error in
            if let error = error {
                print("Notification auth error: \(error)")
            }
        }
    }

    func showTransferProgress(title: String, body: String, progress: Double) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .none
        content.userInfo = ["progress": progress]

        let request = UNNotificationRequest(
            identifier: "transfer-progress",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    func showTransferComplete(title: String, body: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default

        let request = UNNotificationRequest(
            identifier: "transfer-complete-\(UUID().uuidString)",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    func showTransferError(title: String, body: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default

        let request = UNNotificationRequest(
            identifier: "transfer-error-\(UUID().uuidString)",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }
}

extension NotificationManager: UNUserNotificationCenterDelegate {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound])
    }
}
