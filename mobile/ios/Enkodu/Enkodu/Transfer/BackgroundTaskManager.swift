import BackgroundTasks
import UIKit

class BackgroundTaskManager {
    static let shared = BackgroundTaskManager()
    private let taskIdentifier = "com.enkodu.transfers"

    func registerTasks() {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: taskIdentifier, using: nil) { task in
            self.handleTransferTask(task: task as! BGProcessingTask)
        }
    }

    func scheduleTransferTask() {
        let request = BGProcessingTaskRequest(identifier: taskIdentifier)
        request.requiresNetworkConnectivity = true
        request.requiresExternalPower = false

        do {
            try BGTaskScheduler.shared.submit(request)
        } catch {
            print("Could not schedule transfer task: \(error)")
        }
    }

    private func handleTransferTask(task: BGProcessingTask) {
        let queue = OperationQueue()
        queue.maxConcurrentOperationCount = 1

        let lastOperation = BlockOperation {
            task.setTaskCompleted(success: true)
        }

        // Resume any pending transfers
        Task {
            await TransferManager.shared.resumeAll()
        }

        lastOperation.addDependency(queue.operations.last ?? BlockOperation())
        queue.addOperation(lastOperation)

        task.expirationHandler = {
            queue.cancelAllOperations()
        }
    }
}
