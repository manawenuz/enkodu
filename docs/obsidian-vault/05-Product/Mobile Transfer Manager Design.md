# Mobile Transfer Manager Design

## Purpose

Define the retry/resume strategy for mobile companion uploads and downloads. Mobile apps face constraints that desktop companions do not: app suspension, background restrictions, battery optimization, cellular data limits, and limited disk space.

## Problem Statement

Mobile devices must transfer large video files (hundreds of MB to several GB) over unreliable networks. The transfer must:

1. Resume from the exact byte where it left off after app suspension, network change, or crash.
2. Retry transient failures with exponential backoff, respecting battery and data usage.
3. Never silently fail — user must see recoverable state.
4. Respect the AV1 hardware decode gate: unsupported devices must not upload or download.

## Architecture

```
┌─────────────────────────────────────────┐
│         Mobile Transfer Manager          │
├─────────────────────────────────────────┤
│  TransferState (persisted to SQLite)    │
│  ├─ upload_id / job_id                  │
│  ├─ file_path (local)                   │
│  ├─ bytes_transferred                   │
│  ├─ total_bytes                         │
│  ├─ status: pending/active/paused/failed │
│  ├─ last_error                          │
│  ├─ retry_count                         │
│  ├─ created_at / updated_at             │
│  └─ network_type (wifi/cellular)         │
├─────────────────────────────────────────┤
│  TransferOrchestrator                    │
│  ├─ enqueue()                            │
│  ├─ pause()                              │
│  ├─ resume()                             │
│  ├─ cancel()                             │
│  ├─ retry_failed()                       │
│  └─ observer callbacks                   │
├─────────────────────────────────────────┤
│  ResumableUploader (Android/iOS)         │
│  ├─ start() → upload_id                  │
│  ├─ send_chunk()                         │
│  ├─ finish() → job_id                    │
│  └─ resume_from(byte_offset)             │
├─────────────────────────────────────────┤
│  ResumableDownloader (Android/iOS)       │
│  ├─ download()                           │
│  ├─ resume_from(byte_offset)             │
│  └─ verify_after_download()              │
├─────────────────────────────────────────┤
│  RetryPolicy                             │
│  ├─ max_retries: 8                       │
│  ├─ base_delay: 500ms                    │
│  ├─ max_delay: 30s                       │
│  ├─ backoff: 1.5x                        │
│  └─ retryable_errors: timeout, 5xx, 429  │
├─────────────────────────────────────────┤
│  Constraints                              │
│  ├─ max_concurrent_uploads: 1            │
│  ├─ max_concurrent_downloads: 2           │
│  ├─ wifi_only_uploads_mb: 100            │
│  ├─ wifi_only_downloads: true            │
│  └─ battery_min_percent: 15              │
└─────────────────────────────────────────┘
```

## State Machine

```
         ┌─────────┐
         │  IDLE   │
         └────┬────┘
              │ enqueue()
              ▼
         ┌─────────┐     network error     ┌─────────┐
         │PENDING  │──────────────────────▶│ PAUSED  │
         │(queued) │◄──────────────────────│         │
         └────┬────┘      resume()          └─────────┘
              │ start()
              ▼
         ┌─────────┐
         │ACTIVE   │
         │(transfer)│
         └────┬────┘
              │ finish()
              ▼
         ┌─────────┐
         │DONE     │
         └─────────┘
              │
              │ fail (permanent) / max retries
              ▼
         ┌─────────┐
         │FAILED   │
         └─────────┘
              │
              │ retry_failed()
              ▼
              (back to PENDING)
```

## Resumable Upload Protocol

### Server Endpoints

```
POST /jobs/upload/resumable/start
  Body:  {"filename": "video.mp4", "filepath": "/path/to/video.mp4", "total_size": 2147483648}
  Resp:  {"upload_id": "uuid", "chunk_size": 8388608, "expires_in": 86400}

PUT /jobs/upload/resumable/{upload_id}/chunk
  Header: Content-Range: bytes 0-8388607/2147483648
  Body:   <8 MiB binary chunk>
  Resp:   {"ok": true, "received": 8388608, "total": 2147483648}

POST /jobs/upload/resumable/{upload_id}/finish
  Resp:   {"job_id": "uuid", "priority_position": 42, "client_name": "...", "deduped": false}
```

### Client Behavior

1. **Start**: Call `POST /jobs/upload/resumable/start`. Save `upload_id` to local state.
2. **Chunk**: Read file in `chunk_size` blocks. For each block:
   - Set `Content-Range: bytes {start}-{end}/{total}`
   - Send with retry logic
   - On success, update `bytes_transferred` in local state
   - On network error, save state and pause
3. **Resume**: On app restart, read `upload_id` and `bytes_transferred` from state. Resume from next byte.
4. **Finish**: After all chunks sent, call `POST /jobs/upload/resumable/{upload_id}/finish`.
5. **Cleanup**: On success or permanent failure, delete local state and temp file.

### Android Implementation

```kotlin
class ResumableUploader(
    private val api: EnkoduApi,
    private val stateDao: TransferStateDao,
    private val chunkSize: Long = 8 * 1024 * 1024
) {
    suspend fun upload(file: File, onProgress: (Long, Long) -> Unit): Result<UploadResponse> {
        val totalSize = file.length()
        val existing = stateDao.getByFilePath(file.absolutePath)
        
        val uploadId = existing?.uploadId ?: run {
            val startResp = api.startResumableUpload(
                ResumableStartReq(file.name, file.absolutePath, totalSize)
            ).getOrThrow()
            stateDao.insert(TransferState(
                uploadId = startResp.uploadId,
                filePath = file.absolutePath,
                totalBytes = totalSize,
                status = TransferStatus.PENDING
            ))
            startResp.uploadId
        }
        
        val transferred = existing?.bytesTransferred ?: 0
        
        RandomAccessFile(file, "r").use { raf ->
            var offset = transferred
            while (offset < totalSize) {
                val end = min(offset + chunkSize - 1, totalSize - 1)
                val chunk = ByteArray((end - offset + 1).toInt())
                raf.seek(offset)
                raf.readFully(chunk)
                
                val result = retryWithBackoff {
                    api.uploadChunk(uploadId, offset, end, totalSize, chunk)
                }
                
                if (result.isFailure) {
                    stateDao.updateStatus(uploadId, TransferStatus.PAUSED, offset)
                    return Result.failure(result.exceptionOrNull()!!)
                }
                
                offset = end + 1
                stateDao.updateProgress(uploadId, offset)
                onProgress(offset, totalSize)
            }
        }
        
        val finishResp = api.finishResumableUpload(uploadId).getOrThrow()
        stateDao.delete(uploadId)
        return Result.success(finishResp)
    }
}
```

### iOS Implementation

```swift
class ResumableUploader {
    private let api: EnkoduAPI
    private let stateStore: TransferStateStore
    private let chunkSize: Int = 8 * 1024 * 1024
    
    func upload(file: URL, progress: @escaping (Int64, Int64) -> Void) async throws -> UploadResponse {
        let totalSize = try file.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        let existing = stateStore.state(for: file.path)
        
        let uploadId: String
        if let existing = existing, !existing.uploadId.isEmpty {
            uploadId = existing.uploadId
        } else {
            let start = try await api.startResumableUpload(
                filename: file.lastPathComponent,
                filepath: file.path,
                totalSize: totalSize
            )
            stateStore.save(TransferState(
                uploadId: start.uploadId,
                filePath: file.path,
                totalBytes: totalSize,
                status: .pending
            ))
            uploadId = start.uploadId
        }
        
        let handle = try FileHandle(forReadingFrom: file)
        defer { try? handle.close() }
        
        var offset = existing?.bytesTransferred ?? 0
        while offset < totalSize {
            let end = min(offset + chunkSize - 1, totalSize - 1)
            try handle.seek(toOffset: UInt64(offset))
            let chunk = handle.readData(ofLength: end - offset + 1)
            
            try await retryWithBackoff {
                try await api.uploadChunk(
                    uploadId: uploadId,
                    start: offset,
                    end: end,
                    total: totalSize,
                    data: chunk
                )
            }
            
            offset = end + 1
            stateStore.updateProgress(uploadId: uploadId, bytesTransferred: offset)
            progress(offset, totalSize)
        }
        
        let finish = try await api.finishResumableUpload(uploadId: uploadId)
        stateStore.delete(uploadId: uploadId)
        return finish
    }
}
```

## Resumable Download Protocol

### Server Support

The server now supports `Range` headers on `GET /jobs/{id}/output`:

```
GET /jobs/{id}/output
  Header: Range: bytes=0-8388607
  Resp:   206 Partial Content
  Header: Content-Range: bytes 0-8388607/2147483648
  Header: Accept-Ranges: bytes
```

### Client Behavior

1. **Start**: Check local state for existing download. If none, create new.
2. **Download chunks**: Use `Range` headers to download in chunks.
3. **Append**: Write each chunk to a temp file at the correct offset.
4. **Resume**: Read `bytes_transferred` from state. Request `Range: bytes={offset}-`.
5. **Verify**: After download, check file size and hash (if available).
6. **Save**: Move temp file to final location (MediaStore/Photos/Files).

### Android Implementation

```kotlin
class ResumableDownloader(
    private val api: EnkoduApi,
    private val stateDao: TransferStateDao,
    private val chunkSize: Long = 8 * 1024 * 1024
) {
    suspend fun download(
        jobId: String,
        outputFile: File,
        totalSize: Long,
        onProgress: (Long, Long) -> Unit
    ): Result<File> {
        val existing = stateDao.getByJobId(jobId)
        val tempFile = File(outputFile.parent, "${outputFile.name}.tmp")
        
        var offset = existing?.bytesTransferred ?: 0
        
        RandomAccessFile(tempFile, "rw").use { raf ->
            while (offset < totalSize) {
                val end = min(offset + chunkSize - 1, totalSize - 1)
                
                val result = retryWithBackoff {
                    api.downloadChunk(jobId, offset, end)
                }
                
                if (result.isFailure) {
                    stateDao.updateStatus(
                        jobId = jobId,
                        status = TransferStatus.PAUSED,
                        bytesTransferred = offset
                    )
                    return Result.failure(result.exceptionOrNull()!!)
                }
                
                val chunk = result.getOrThrow()
                raf.seek(offset)
                raf.write(chunk)
                
                offset = end + 1
                stateDao.updateProgress(jobId, offset)
                onProgress(offset, totalSize)
            }
        }
        
        tempFile.renameTo(outputFile)
        stateDao.delete(jobId)
        return Result.success(outputFile)
    }
}
```

## Retry Policy

### Error Classification

| Error | Kind | Retry? | Notes |
|---|---|---|---|
| `TimeoutException` | Network | Yes | Increase delay |
| `ConnectException` | Network | Yes | Increase delay |
| `SSLException` | Network | Yes | Retry up to 3x |
| `HTTP 408` | Server | Yes | Retry immediately |
| `HTTP 429` | Server | Yes | Respect `Retry-After` |
| `HTTP 502/503/504` | Server | Yes | Increase delay |
| `HTTP 500` | Server | Yes | Retry up to 3x |
| `HTTP 400/401/403/404/422` | Client | No | Permanent failure |
| `DiskFullException` | Local | No | Permanent failure |
| `AV1NotSupported` | Local | No | Permanent failure (gate) |

### Backoff Schedule

Attempt | Delay (mobile) | Delay (transfer)
---|---|---
0 | 500ms | 1s
1 | 750ms | 2s
2 | 1.1s | 4s
3 | 1.7s | 8s
4 | 2.5s | 16s
5 | 3.8s | 30s
6 | 5.7s | 60s
7 | 8.5s | 120s
8 | 12.8s | —

Jitter: add 0-30% random delay to each retry to avoid thundering herd.

### Max Retries

- **Upload**: 10 attempts (covers ~15 minutes of retrying)
- **Download**: 10 attempts
- **Poll**: 5 attempts (with 5s base delay)
- **Start/Finish**: 5 attempts

## State Persistence

### SQLite Schema

```sql
CREATE TABLE transfers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    upload_id TEXT,
    job_id TEXT,
    file_path TEXT NOT NULL,
    local_temp_path TEXT,
    total_bytes INTEGER NOT NULL,
    bytes_transferred INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending',
    transfer_type TEXT NOT NULL, -- 'upload' or 'download'
    last_error TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    network_type TEXT, -- 'wifi', 'cellular', 'unknown'
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(upload_id),
    UNIQUE(job_id)
);

CREATE INDEX idx_transfers_status ON transfers(status);
CREATE INDEX idx_transfers_upload_id ON transfers(upload_id);
CREATE INDEX idx_transfers_job_id ON transfers(job_id);
```

### State Updates

- **On chunk success**: Update `bytes_transferred` and `updated_at`
- **On pause**: Set `status = 'paused'`, save `bytes_transferred`
- **On failure**: Set `status = 'failed'`, save `last_error`, increment `retry_count`
- **On app background**: Pause active transfers, save state
- **On app foreground**: Resume paused transfers (if network is suitable)
- **On max retries**: Set `status = 'failed'`, notify user

## Network Constraints

### WiFi vs Cellular

| Constraint | Default | User Configurable |
|---|---|---|
| Upload on cellular | Blocked if > 100MB | Yes |
| Download on cellular | Blocked | Yes |
| Resume on cellular | Only if under limit | Yes |

### Battery Constraints

- Pause transfers if battery < 15%
- Pause transfers if thermal throttling detected
- Resume when battery > 20% and not throttling

### Background Constraints

**Android**:
- Upload: Use `WorkManager` with `setRequiresNetworkType(CONNECTED)`
- Download: Use `WorkManager` with `setRequiresNetworkType(UNMETERED)`
- For immediate transfers: Use foreground service with notification

**iOS**:
- Use `URLSession` with background configuration
- Register background task `BGProcessingTask` for large transfers
- Handle `URLSessionDelegate` callbacks for completion

## Cancellation

User can cancel a transfer at any time:

1. Set `status = 'cancelled'`
2. Abort in-flight HTTP request
3. Delete temp file
4. Delete state from database
5. If upload: call `DELETE /jobs/upload/resumable/{upload_id}` (server cleanup)

## Failure Handling

### Recoverable Failures

- **Network drop**: Pause, retry when network returns
- **App killed**: Resume from state on next launch
- **Server restart**: Retry with backoff
- **Chunk mismatch**: Re-send chunk (server verifies Content-Range)

### Permanent Failures

- **HTTP 4xx**: Mark as failed, notify user
- **Disk full**: Mark as failed, notify user
- **AV1 gate failure**: Mark as failed, disable upgrade flow
- **Max retries exceeded**: Mark as failed, notify user

## Testing Requirements

### Unit Tests

- Retry backoff calculation
- Chunk boundary calculation
- State persistence round-trip
- Error classification

### Integration Tests

- Upload 1GB file, pause at 50%, resume
- Download 1GB file, kill app at 50%, resume
- Network drop during transfer, auto-retry
- Server restart during transfer, resume after backoff
- Cancel upload, verify server cleanup

### Manual Tests

- iOS background transfer (app backgrounded for 5 minutes)
- Android Doze mode transfer
- Cellular data limit enforcement
- Battery low pause/resume

## Acceptance Criteria

- [ ] Upload can resume from exact byte after app restart
- [ ] Download can resume from exact byte after app restart
- [ ] Transfer survives network drop for < 30 seconds
- [ ] Transfer pauses on network drop > 30 seconds, resumes when network returns
- [ ] Transfer pauses when battery < 15%, resumes when battery > 20%
- [ ] Transfer does not start on cellular if over WiFi-only limit
- [ ] User sees progress bar with accurate percentage
- [ ] User can cancel transfer and see clean state
- [ ] Failed transfer shows specific error message (not generic "failed")
- [ ] No duplicate uploads or downloads after retry

## Open Questions

1. **Server-side cleanup**: Should server auto-expire resumable uploads after 24h? (Yes, recommended)
2. **Chunk size**: Is 8MB optimal for mobile? (Test on 3G/4G/5G/WiFi)
3. **Checksum verification**: Should client verify SHA-256 after upload? (Yes, add to Phase 5)
4. **Concurrent transfers**: Should mobile allow 2 uploads? (No, keep at 1)
5. **Push notifications**: Should server notify when job is done? (Yes, but out of scope for this PRD)
