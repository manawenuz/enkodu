import Foundation

class TelemetryManager {
    static let shared = TelemetryManager()
    private let api: EnkoduApi
    private let clientId: String
    private let platform: String
    private let queue = OperationQueue()

    private init() {
        let serverURL = UserDefaults.standard.string(forKey: "serverURL") ?? "https://example.invalid"
        self.api = EnkoduApi(serverURL: serverURL)
        if let stored = UserDefaults.standard.string(forKey: "telemetry_client_id") {
            self.clientId = stored
        } else {
            self.clientId = UUID().uuidString
            UserDefaults.standard.set(self.clientId, forKey: "telemetry_client_id")
        }
        let systemVersion = UIDevice.current.systemVersion
        self.platform = "ios-\(systemVersion)"
        queue.maxConcurrentOperationCount = 1
    }

    func trackUploadStart(jobId: String? = nil) {
        track(eventType: "upload_start", jobId: jobId)
    }

    func trackUploadSuccess(jobId: String, durationMs: Int64, bytesTransferred: Int64) {
        track(eventType: "upload_finish", jobId: jobId, success: true, durationMs: durationMs, bytesTransferred: bytesTransferred)
    }

    func trackUploadFailure(error: String, jobId: String? = nil, durationMs: Int64? = nil) {
        track(eventType: "upload_finish", jobId: jobId, success: false, durationMs: durationMs, detail: error)
    }

    func trackDownloadStart(jobId: String) {
        track(eventType: "download_start", jobId: jobId)
    }

    func trackDownloadSuccess(jobId: String, durationMs: Int64, bytesTransferred: Int64) {
        track(eventType: "download_finish", jobId: jobId, success: true, durationMs: durationMs, bytesTransferred: bytesTransferred)
    }

    func trackDownloadFailure(error: String, jobId: String, durationMs: Int64? = nil) {
        track(eventType: "download_finish", jobId: jobId, success: false, durationMs: durationMs, detail: error)
    }

    func trackAppLaunch() {
        track(eventType: "app_launch")
    }

    func trackAv1GateResult(supported: Bool) {
        track(eventType: "av1_gate", success: supported, detail: supported ? "supported" : "unsupported")
    }

    func trackError(errorType: String, detail: String? = nil) {
        track(eventType: "error", detail: "\(errorType): \(detail ?? "")")
    }

    private func track(
        eventType: String,
        jobId: String? = nil,
        success: Bool = true,
        durationMs: Int64? = nil,
        bytesTransferred: Int64? = nil,
        detail: String? = nil
    ) {
        queue.addOperation {
            Task {
                do {
                    let request = TelemetryRequest(
                        clientId: self.clientId,
                        eventType: eventType,
                        eventDetail: detail,
                        jobId: jobId,
                        platform: self.platform,
                        success: success,
                        durationMs: durationMs,
                        bytesTransferred: bytesTransferred
                    )
                    try await self.api.postTelemetry(request: request)
                } catch {
                    print("Telemetry error: \(error)")
                }
            }
        }
    }
}

struct TelemetryRequest: Codable {
    let clientId: String?
    let eventType: String
    let eventDetail: String?
    let jobId: String?
    let platform: String?
    let success: Bool
    let durationMs: Int64?
    let bytesTransferred: Int64?
}
