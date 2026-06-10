import Foundation

class BackgroundSessionManager: NSObject, URLSessionDelegate, URLSessionTaskDelegate, URLSessionDownloadDelegate {
    static let shared = BackgroundSessionManager()

    private var backgroundSession: URLSession!
    private var completionHandlers: [String: () -> Void] = [:]
    private var progressHandlers: [String: (Int64, Int64) -> Void] = [:]
    private var sessionCompletionHandlers: [String: () -> Void] = [:]

    private override init() {
        super.init()
        let config = URLSessionConfiguration.background(withIdentifier: "com.enkodu.background")
        config.isDiscretionary = false
        config.sessionSendsLaunchEvents = true
        config.allowsCellularAccess = true
        backgroundSession = URLSession(configuration: config, delegate: self, delegateQueue: nil)
    }

    func downloadFile(url: URL, destination: URL, jobId: String, completion: @escaping () -> Void, progress: @escaping (Int64, Int64) -> Void) {
        let task = backgroundSession.downloadTask(with: url)
        completionHandlers[jobId] = completion
        progressHandlers[jobId] = progress
        task.resume()
    }

    func uploadFile(url: URL, file: URL, uploadId: String, completion: @escaping () -> Void, progress: @escaping (Int64, Int64) -> Void) {
        var request = URLRequest(url: url)
        request.httpMethod = "PUT"
        let task = backgroundSession.uploadTask(with: request, fromFile: file)
        completionHandlers[uploadId] = completion
        progressHandlers[uploadId] = progress
        task.resume()
    }

    // MARK: - URLSessionDownloadDelegate

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        if let jobId = downloadTask.originalRequest?.url?.lastPathComponent {
            progressHandlers[jobId]?(totalBytesWritten, totalBytesExpectedToWrite)
        }
    }

    func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        if let jobId = downloadTask.originalRequest?.url?.lastPathComponent {
            completionHandlers[jobId]?()
            completionHandlers.removeValue(forKey: jobId)
            progressHandlers.removeValue(forKey: jobId)
        }
    }

    // MARK: - URLSessionTaskDelegate

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error = error {
            print("Background task error: \(error)")
        }
    }

    // MARK: - URLSessionDelegate

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        let identifier = session.configuration.identifier ?? ""
        sessionCompletionHandlers[identifier]?()
        sessionCompletionHandlers.removeValue(forKey: identifier)
    }

    func setCompletionHandler(_ handler: @escaping () -> Void, for identifier: String) {
        sessionCompletionHandlers[identifier] = handler
    }
}
