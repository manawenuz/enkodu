import Foundation

protocol EnkoduApiProtocol {
    func getStatus() async throws -> StatusResponse
    func getJob(jobId: String) async throws -> JobResponse
    func downloadOutput(jobId: String, range: String?) async throws -> Data
    func startResumableUpload(filename: String, filepath: String?, totalSize: Int64) async throws -> ResumableStartResponse
    func uploadChunk(uploadId: String, start: Int64, end: Int64, total: Int64, data: Data) async throws -> ChunkResponse
    func finishResumableUpload(uploadId: String) async throws -> UploadFinishResponse
    func getChecksum(jobId: String) async throws -> ChecksumResponse
}

actor EnkoduApi: EnkoduApiProtocol {
    private let baseURL: URL
    private let session: URLSession
    private let authTokenProvider: () -> String?
    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()

    init(serverURL: String, authTokenProvider: @escaping () -> String? = { AuthStore.shared.companionToken }) {
        self.baseURL = URL(string: serverURL)!
        self.authTokenProvider = authTokenProvider
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 300
        config.timeoutIntervalForResource = 3600
        self.session = URLSession(configuration: config)

        // Snake_case to camelCase decoding
        decoder.keyDecodingStrategy = .convertFromSnakeCase
    }

    func getStatus() async throws -> StatusResponse {
        let request = authorizedRequest(path: "status")
        let (data, _) = try await send(request)
        return try decoder.decode(StatusResponse.self, from: data)
    }

    func getJob(jobId: String) async throws -> JobResponse {
        let request = authorizedRequest(path: "jobs/\(jobId)")
        let (data, _) = try await send(request)
        return try decoder.decode(JobResponse.self, from: data)
    }

    func downloadOutput(jobId: String, range: String? = nil) async throws -> Data {
        var request = authorizedRequest(path: "jobs/\(jobId)/output")
        if let range = range {
            request.setValue(range, forHTTPHeaderField: "Range")
        }
        let (data, _) = try await send(request)
        return data
    }

    func startResumableUpload(filename: String, filepath: String?, totalSize: Int64) async throws -> ResumableStartResponse {
        let request = ResumableStartRequest(filename: filename, filepath: filepath, totalSize: totalSize)
        let body = try encoder.encode(request)
        var urlRequest = authorizedRequest(path: "jobs/upload/resumable/start")
        urlRequest.httpMethod = "POST"
        urlRequest.httpBody = body
        urlRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let (data, _) = try await send(urlRequest)
        return try decoder.decode(ResumableStartResponse.self, from: data)
    }

    func uploadChunk(uploadId: String, start: Int64, end: Int64, total: Int64, data: Data) async throws -> ChunkResponse {
        var request = authorizedRequest(path: "jobs/upload/resumable/\(uploadId)/chunk")
        request.httpMethod = "PUT"
        request.httpBody = data
        request.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        request.setValue("bytes \(start)-\(end)/\(total)", forHTTPHeaderField: "Content-Range")
        let (respData, _) = try await send(request)
        return try decoder.decode(ChunkResponse.self, from: respData)
    }

    func finishResumableUpload(uploadId: String) async throws -> UploadFinishResponse {
        var request = authorizedRequest(path: "jobs/upload/resumable/\(uploadId)/finish")
        request.httpMethod = "POST"
        let (data, _) = try await send(request)
        return try decoder.decode(UploadFinishResponse.self, from: data)
    }

    func getChecksum(jobId: String) async throws -> ChecksumResponse {
        let request = authorizedRequest(path: "jobs/\(jobId)/checksum")
        let (data, _) = try await send(request)
        return try decoder.decode(ChecksumResponse.self, from: data)
    }

    func getHealthz() async throws -> HealthzResponse {
        let url = baseURL.appendingPathComponent("healthz")
        let (data, _) = try await session.data(from: url)
        return try decoder.decode(HealthzResponse.self, from: data)
    }

    func postTelemetry(request: TelemetryRequest) async throws {
        var urlRequest = URLRequest(url: baseURL.appendingPathComponent("telemetry"))
        urlRequest.httpMethod = "POST"
        urlRequest.httpBody = try encoder.encode(request)
        urlRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let (_, response) = try await session.data(for: urlRequest)
        guard let httpResponse = response as? HTTPURLResponse, (200...299).contains(httpResponse.statusCode) else {
            throw EnkoduError.invalidResponse
        }
    }

    func testProtectedConnection() async -> AuthCheckResult {
        do {
            _ = try await getStatus()
            return .connected
        } catch EnkoduError.authRequired {
            return .tokenRejected
        } catch EnkoduError.permissionDenied {
            return .permissionDenied
        } catch {
            return .serverUnreachable(error.localizedDescription)
        }
    }

    func authorizedRequest(path: String) -> URLRequest {
        var request = URLRequest(url: baseURL.appendingPathComponent(path))
        if let token = authTokenProvider()?.trimmingCharacters(in: .whitespacesAndNewlines),
           !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw EnkoduError.invalidResponse
        }
        switch httpResponse.statusCode {
        case 200...299:
            return (data, httpResponse)
        case 401:
            throw EnkoduError.authRequired
        case 403:
            throw EnkoduError.permissionDenied
        default:
            throw EnkoduError.httpStatus(httpResponse.statusCode)
        }
    }
}

enum AuthCheckResult: Equatable {
    case connected
    case tokenRejected
    case permissionDenied
    case serverUnreachable(String)

    var authState: AuthState {
        switch self {
        case .connected:
            return .connected
        case .tokenRejected:
            return .tokenRejected
        case .permissionDenied:
            return .permissionDenied
        case .serverUnreachable:
            return .serverUnreachable
        }
    }

    var message: String {
        switch self {
        case .connected:
            return "Server reachable and companion token accepted"
        case .tokenRejected:
            return "Server rejected the companion token"
        case .permissionDenied:
            return "Token accepted, but this companion is not permitted"
        case .serverUnreachable(let detail):
            return "Server unreachable: \(detail)"
        }
    }
}

// MARK: - Models

struct StatusResponse: Codable {
    let ok: Bool
    let pending: Int
    let active: Int
    let done: Int
    let failed: Int
}

struct JobResponse: Codable {
    let id: String
    let status: String
    let percent: Double?
    let fps: Double?
    let speed: String?
    let worker: String?
    let error: String?
    let outputSize: Int64?
    let sourceSize: Int64?
    let verifyStatus: String?
    let verifyDetail: String?
}

struct ResumableStartRequest: Codable {
    let filename: String
    let filepath: String?
    let totalSize: Int64
}

struct ResumableStartResponse: Codable {
    let uploadId: String
    let chunkSize: Int64
    let expiresIn: Int64
}

struct ChunkResponse: Codable {
    let ok: Bool
    let received: Int64
    let total: Int64
}

struct UploadFinishResponse: Codable {
    let jobId: String
    let priorityPosition: Int64
    let clientName: String
    let deduped: Bool
}

struct ChecksumResponse: Codable {
    let jobId: String
    let status: String
    let sourceSha256: String?
    let outputSha256: String?
}

struct HealthzResponse: Codable {
    let ok: Bool
}

enum EnkoduError: Error {
    case authRequired
    case permissionDenied
    case downloadFailed
    case uploadFailed
    case maxRetriesExceeded
    case invalidResponse
    case httpStatus(Int)
}
