import Foundation
import CoreData

@MainActor
class TransferManager: ObservableObject {
    static let shared = TransferManager()

    private let api: EnkoduApi
    private let context: NSManagedObjectContext
    private let chunkSize: Int64 = 8 * 1024 * 1024 // 8 MiB
    private let maxRetries = 10
    private let baseDelayMs: Double = 500
    private let maxDelayMs: Double = 30000
    private let backoffMultiplier = 1.5

    @Published var activeTransfers: [TransferState] = []

    private init() {
        let serverURL = UserDefaults.standard.string(forKey: "serverURL") ?? "https://example.invalid"
        self.api = EnkoduApi(serverURL: serverURL)
        self.context = PersistenceController.shared.container.viewContext
    }

    // MARK: - Background Upload

    func uploadFileBackground(file: URL, onProgress: @escaping (Int64, Int64) -> Void = { _, _ in }, completion: @escaping (Result<UploadFinishResponse, Error>) -> Void) {
        let totalSize = (try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        let serverURL = UserDefaults.standard.string(forKey: "serverURL") ?? "https://example.invalid"

        Task {
            do {
                let start = try await api.startResumableUpload(
                    filename: file.lastPathComponent,
                    filepath: file.path,
                    totalSize: Int64(totalSize)
                )
                let uploadId = start.uploadId

                // Save state for resume
                let state = TransferState(context: context)
                state.id = UUID()
                state.uploadId = uploadId
                state.filePath = file.path
                state.totalBytes = Int64(totalSize)
                state.status = TransferStatus.pending.rawValue
                state.transferType = TransferType.upload.rawValue
                state.createdAt = Date()
                state.updatedAt = Date()
                PersistenceController.shared.save()
                TelemetryManager.shared.trackUploadStart(jobId: uploadId)

                // Use background session for the actual upload
                let url = URL(string: "\(serverURL)/jobs/upload/resumable/\(uploadId)/chunk")!
                BackgroundSessionManager.shared.uploadFile(
                    url: url,
                    file: file,
                    uploadId: uploadId,
                    completion: { [weak self] in
                        // When all chunks are done, finish
                        Task {
                            do {
                                let finish = try await self?.api.finishResumableUpload(uploadId: uploadId)
                                if let finish = finish {
                                    TelemetryManager.shared.trackUploadSuccess(jobId: uploadId, durationMs: 0, bytesTransferred: Int64(totalSize))
                                    completion(.success(finish))
                                }
                            } catch {
                                TelemetryManager.shared.trackUploadFailure(error: "Background finish failed: \(error.localizedDescription)", jobId: uploadId)
                                completion(.failure(error))
                            }
                        }
                    },
                    progress: { bytes, total in
                        onProgress(bytes, total)
                    }
                )
            } catch {
                TelemetryManager.shared.trackUploadFailure(error: "Background start failed: \(error.localizedDescription)")
                completion(.failure(error))
            }
        }
    }

    // MARK: - Upload

    func uploadFile(file: URL, onProgress: @escaping (Int64, Int64) -> Void) async throws -> UploadFinishResponse {
        let totalSize = try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        let existing = try await fetchTransferState(filePath: file.path)
        let startTime = Date()

        let uploadId: String
        let chunkSize: Int64

        if let existing = existing, let existingUploadId = existing.uploadId {
            uploadId = existingUploadId
            chunkSize = self.chunkSize
            print("Resuming upload for \(file.lastPathComponent) from \(existing.bytesTransferred) bytes")
        } else {
            do {
                let start = try await api.startResumableUpload(
                    filename: file.lastPathComponent,
                    filepath: file.path,
                    totalSize: Int64(totalSize)
                )
                uploadId = start.uploadId
                chunkSize = start.chunkSize
                let state = TransferState(context: context)
                state.id = UUID()
                state.uploadId = uploadId
                state.filePath = file.path
                state.totalBytes = Int64(totalSize)
                state.status = TransferStatus.pending.rawValue
                state.transferType = TransferType.upload.rawValue
                state.createdAt = Date()
                state.updatedAt = Date()
                PersistenceController.shared.save()
                TelemetryManager.shared.trackUploadStart(jobId: uploadId)
            } catch {
                let duration = Int64(Date().timeIntervalSince(startTime) * 1000)
                TelemetryManager.shared.trackUploadFailure(error: "Start failed: \(error.localizedDescription)", durationMs: duration)
                throw error
            }
        }

        var offset = existing?.bytesTransferred ?? 0
        try await updateStatus(uploadId: uploadId, status: .active, bytesTransferred: offset)

        let handle = try FileHandle(forReadingFrom: file)
        defer { try? handle.close() }

        do {
            while offset < Int64(totalSize) {
                let end = min(offset + chunkSize - 1, Int64(totalSize) - 1)
                try handle.seek(toOffset: UInt64(offset))
                let chunk = handle.readData(ofLength: Int(end - offset + 1))

                let result = try await retryWithBackoff { attempt in
                    try await api.uploadChunk(
                        uploadId: uploadId,
                        start: offset,
                        end: end,
                        total: Int64(totalSize),
                        data: chunk
                    )
                }

                offset = end + 1
                try await updateStatus(uploadId: uploadId, status: .active, bytesTransferred: offset)
                onProgress(offset, Int64(totalSize))
            }

            let finish = try await retryWithBackoff { _ in
                try await api.finishResumableUpload(uploadId: uploadId)
            }

            if let state = try await fetchTransferState(uploadId: uploadId) {
                context.delete(state)
                PersistenceController.shared.save()
            }

            let duration = Int64(Date().timeIntervalSince(startTime) * 1000)
            TelemetryManager.shared.trackUploadSuccess(jobId: uploadId, durationMs: duration, bytesTransferred: Int64(totalSize))
            NotificationManager.shared.showTransferComplete(
                title: "Upload Complete",
                body: "\(file.lastPathComponent) queued for processing"
            )

            return finish
        } catch {
            let duration = Int64(Date().timeIntervalSince(startTime) * 1000)
            TelemetryManager.shared.trackUploadFailure(error: "Upload failed: \(error.localizedDescription)", jobId: uploadId, durationMs: duration)
            throw error
        }
    }

    // MARK: - Download

    func downloadFile(jobId: String, outputFile: URL, totalSize: Int64, onProgress: @escaping (Int64, Int64) -> Void) async throws {
        let tempFile = outputFile.deletingLastPathComponent().appendingPathComponent(outputFile.lastPathComponent + ".part")
        let existing = try await fetchTransferState(jobId: jobId)
        var offset = existing?.bytesTransferred ?? 0
        let startTime = Date()

        if FileManager.default.fileExists(atPath: tempFile.path) && offset == 0 {
            let attrs = try FileManager.default.attributesOfItem(atPath: tempFile.path)
            offset = attrs[.size] as? Int64 ?? 0
        }

        let state = TransferState(context: context)
        state.id = UUID()
        state.jobId = jobId
        state.filePath = outputFile.path
        state.localTempPath = tempFile.path
        state.totalBytes = totalSize
        state.bytesTransferred = offset
        state.status = TransferStatus.active.rawValue
        state.transferType = TransferType.download.rawValue
        state.createdAt = Date()
        state.updatedAt = Date()
        PersistenceController.shared.save()
        TelemetryManager.shared.trackDownloadStart(jobId: jobId)

        let handle = try FileHandle(forWritingTo: tempFile)
        defer { try? handle.close() }

        do {
            while offset < totalSize {
                let end = min(offset + chunkSize - 1, totalSize - 1)
                let range = "bytes=\(offset)-\(end)"

                let data = try await retryWithBackoff { _ in
                    try await api.downloadOutput(jobId: jobId, range: range)
                }

                try handle.seek(toOffset: UInt64(offset))
                try handle.write(contentsOf: data)
                offset += Int64(data.count)
                try await updateStatus(jobId: jobId, status: .active, bytesTransferred: offset)
                onProgress(offset, totalSize)
            }

            try FileManager.default.moveItem(at: tempFile, to: outputFile)
            if let state = try await fetchTransferState(jobId: jobId) {
                context.delete(state)
                PersistenceController.shared.save()
            }

            let duration = Int64(Date().timeIntervalSince(startTime) * 1000)
            TelemetryManager.shared.trackDownloadSuccess(jobId: jobId, durationMs: duration, bytesTransferred: totalSize)
            NotificationManager.shared.showTransferComplete(
                title: "Download Complete",
                body: "Saved to \(outputFile.lastPathComponent)"
            )
        } catch {
            let duration = Int64(Date().timeIntervalSince(startTime) * 1000)
            TelemetryManager.shared.trackDownloadFailure(error: "Download failed: \(error.localizedDescription)", jobId: jobId, durationMs: duration)
            throw error
        }
    }

    // MARK: - Retry

    private func retryWithBackoff<T>(operation: (Int) async throws -> T) async throws -> T {
        var lastError: Error?
        for attempt in 0..<maxRetries {
            do {
                return try await operation(attempt)
            } catch {
                lastError = error
                let delay = calculateDelay(attempt: attempt)
                print("Retry \(attempt)/\(maxRetries) after \(delay)ms: \(error)")
                try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000))
            }
        }
        NotificationManager.shared.showTransferError(
            title: "Transfer Failed",
            body: "Max retries exceeded: \(lastError?.localizedDescription ?? "Unknown error")"
        )
        throw EnkoduError.maxRetriesExceeded
    }

    private func calculateDelay(attempt: Int) -> Double {
        let base = baseDelayMs * pow(backoffMultiplier, Double(attempt))
        let jitter = base * 0.3 * Double.random(in: 0...1)
        return min(base + jitter, maxDelayMs)
    }

    // MARK: - State management

    private func fetchTransferState(uploadId: String? = nil, jobId: String? = nil, filePath: String? = nil) async throws -> TransferState? {
        let request = TransferState.fetchRequest()
        if let uploadId = uploadId {
            request.predicate = NSPredicate(format: "uploadId == %@", uploadId)
        } else if let jobId = jobId {
            request.predicate = NSPredicate(format: "jobId == %@", jobId)
        } else if let filePath = filePath {
            request.predicate = NSPredicate(format: "filePath == %@", filePath)
        }
        request.fetchLimit = 1
        return try context.fetch(request).first
    }

    private func updateStatus(uploadId: String? = nil, jobId: String? = nil, status: TransferStatus, bytesTransferred: Int64) async throws {
        let state = try await fetchTransferState(uploadId: uploadId, jobId: jobId)
        state?.status = status.rawValue
        state?.bytesTransferred = bytesTransferred
        state?.updatedAt = Date()
        PersistenceController.shared.save()
    }

    // MARK: - Pause / Resume

    func pauseAll() {
        let request = TransferState.fetchRequest()
        request.predicate = NSPredicate(format: "status == %@", TransferStatus.active.rawValue)
        do {
            let active = try context.fetch(request)
            for state in active {
                state.status = TransferStatus.paused.rawValue
                state.updatedAt = Date()
            }
            PersistenceController.shared.save()
        } catch {
            print("Failed to pause transfers: \(error)")
        }
    }

    func resumeAll() {
        // Resume logic would be triggered by the UI
        // This just marks paused transfers as pending for resumption
        let request = TransferState.fetchRequest()
        request.predicate = NSPredicate(format: "status == %@", TransferStatus.paused.rawValue)
        do {
            let paused = try context.fetch(request)
            for state in paused {
                state.status = TransferStatus.pending.rawValue
                state.updatedAt = Date()
            }
            PersistenceController.shared.save()
        } catch {
            print("Failed to resume transfers: \(error)")
        }
    }
}
