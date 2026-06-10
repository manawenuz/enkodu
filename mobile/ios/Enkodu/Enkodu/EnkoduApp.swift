import SwiftUI
import AVFoundation

@main
struct EnkoduApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    let persistenceController = PersistenceController.shared

    var body: some Scene {
        WindowGroup {
            OnboardingView()
                .environment(\.managedObjectContext, persistenceController.container.viewContext)
        }
    }
}
    }
}

class AppDelegate: NSObject, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        BackgroundTaskManager.shared.registerTasks()
        NotificationManager.shared
        return true
    }

    func applicationDidEnterBackground(_ application: UIApplication) {
        TransferManager.shared.pauseAll()
        BackgroundTaskManager.shared.scheduleTransferTask()
    }

    func applicationWillEnterForeground(_ application: UIApplication) {
        TransferManager.shared.resumeAll()
    }

    func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String, completionHandler: @escaping () -> Void) {
        BackgroundSessionManager.shared.setCompletionHandler(completionHandler, for: identifier)
    }
}
