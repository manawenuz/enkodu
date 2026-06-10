package com.enkodu.companion.transfer

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import com.enkodu.companion.api.ChunkResponse
import com.enkodu.companion.api.EnkoduApi
import com.enkodu.companion.api.ResumableStartRequest
import com.enkodu.companion.api.ResumableStartResponse
import com.enkodu.companion.api.UploadFinishResponse
import com.enkodu.companion.data.TransferDao
import com.enkodu.companion.data.TransferState
import com.enkodu.companion.data.TransferStatus
import com.enkodu.companion.data.TransferType
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import retrofit2.HttpException
import java.io.File
import java.io.RandomAccessFile
import java.io.IOException
import kotlin.math.min
import kotlin.math.pow
import kotlin.random.Random

class TransferManager(
    private val context: Context,
    private val api: EnkoduApi,
    private val dao: TransferDao,
    private val telemetry: com.enkodu.companion.TelemetryManager? = null
) {
    companion object {
        private const val TAG = "TransferManager"
        private const val CHUNK_SIZE = 8 * 1024 * 1024L // 8 MiB
        private const val MAX_RETRIES = 10
        private const val BASE_DELAY_MS = 500L
        private const val MAX_DELAY_MS = 30000L
        private const val BACKOFF_MULTIPLIER = 1.5
    }

    sealed class Result<out T> {
        data class Success<T>(val data: T) : Result<T>()
        data class Failure(val error: String, val retryable: Boolean = true) : Result<Nothing>()
    }

    // ── Upload ────────────────────────────────────────────────────────────────

    suspend fun uploadFile(
        file: File,
        onProgress: (bytesTransferred: Long, totalBytes: Long) -> Unit
    ): Result<UploadFinishResponse> = withContext(Dispatchers.IO) {
        val totalSize = file.length()
        val existing = dao.getByFilePath(file.absolutePath)
        val startTime = System.currentTimeMillis()

        val uploadId: String
        val chunkSize: Long

        if (existing != null && existing.uploadId != null) {
            uploadId = existing.uploadId
            chunkSize = CHUNK_SIZE
            Log.i(TAG, "Resuming upload for ${file.name} from ${existing.bytesTransferred} bytes")
        } else {
            val startResp = startUpload(file, totalSize)
            when (startResp) {
                is Result.Success -> {
                    uploadId = startResp.data.uploadId
                    chunkSize = startResp.data.chunkSize
                    dao.insert(
                        TransferState(
                            uploadId = uploadId,
                            filePath = file.absolutePath,
                            totalBytes = totalSize,
                            transferType = TransferType.UPLOAD.name,
                            status = TransferStatus.PENDING.name
                        )
                    )
                    telemetry?.trackUploadStart(uploadId)
                }
                is Result.Failure -> {
                    telemetry?.trackUploadFailure("Start failed: ${startResp.error}", durationMs = System.currentTimeMillis() - startTime)
                    return@withContext Result.Failure(startResp.error, startResp.retryable)
                }
            }
        }

        val transferred = existing?.bytesTransferred ?: 0
        dao.updateProgress(uploadId, transferred, TransferStatus.ACTIVE.name)

        RandomAccessFile(file, "r").use { raf ->
            var offset = transferred
            while (offset < totalSize) {
                val end = min(offset + chunkSize - 1, totalSize - 1)
                val chunkSizeActual = (end - offset + 1).toInt()
                val chunk = ByteArray(chunkSizeActual)
                raf.seek(offset)
                raf.readFully(chunk)

                val chunkResult = retryWithBackoff { attempt ->
                    val contentRange = "bytes $offset-$end/$totalSize"
                    val body = chunk.toRequestBody("application/octet-stream".toMediaType())
                    val resp = api.uploadChunk(uploadId, contentRange, body)
                    if (resp.isSuccessful) {
                        resp.body()?.let { Result.Success(it) }
                            ?: Result.Failure("Empty chunk response", true)
                    } else {
                        Result.Failure("Chunk failed: ${resp.code()}", isRetryable(resp.code()))
                    }
                }

                when (chunkResult) {
                    is Result.Success -> {
                        offset = end + 1
                        dao.updateProgress(uploadId, offset, TransferStatus.ACTIVE.name)
                        onProgress(offset, totalSize)
                    }
                    is Result.Failure -> {
                        val duration = System.currentTimeMillis() - startTime
                        when {
                            chunkResult.error.contains("401") -> {
                                dao.updateProgress(uploadId, offset, TransferStatus.PAUSED.name)
                                telemetry?.trackUploadFailure("auth_401 at $offset: ${chunkResult.error}", uploadId, duration)
                                return@withContext Result.Failure("auth_401: token missing or rejected", retryable = false)
                            }
                            chunkResult.error.contains("403") -> {
                                dao.updateProgress(uploadId, offset, TransferStatus.FAILED.name)
                                telemetry?.trackUploadFailure("auth_403 at $offset: ${chunkResult.error}", uploadId, duration)
                                return@withContext Result.Failure("auth_403: permission denied", retryable = false)
                            }
                            else -> {
                                dao.updateProgress(uploadId, offset, TransferStatus.PAUSED.name)
                                telemetry?.trackUploadFailure("Paused at $offset: ${chunkResult.error}", uploadId, duration)
                                return@withContext Result.Failure(
                                    "Upload paused at $offset: ${chunkResult.error}",
                                    chunkResult.retryable
                                )
                            }
                        }
                    }
                }
            }
        }

        // Finish upload
        val finishResult = retryWithBackoff { _ ->
            val resp = api.finishResumableUpload(uploadId)
            if (resp.isSuccessful) {
                resp.body()?.let { Result.Success(it) }
                    ?: Result.Failure("Empty finish response", true)
            } else {
                Result.Failure("Finish failed: ${resp.code()}", isRetryable(resp.code()))
            }
        }

        val duration = System.currentTimeMillis() - startTime
        when (finishResult) {
            is Result.Success -> {
                dao.deleteByUploadId(uploadId)
                telemetry?.trackUploadSuccess(uploadId, duration, totalSize)
                Result.Success(finishResult.data)
            }
            is Result.Failure -> {
                when {
                    finishResult.error.contains("401") -> {
                        dao.updateProgress(uploadId, totalSize, TransferStatus.PAUSED.name)
                        telemetry?.trackUploadFailure("Finish auth_401: ${finishResult.error}", uploadId, duration)
                        Result.Failure("auth_401: token missing or rejected", retryable = false)
                    }
                    finishResult.error.contains("403") -> {
                        dao.updateProgress(uploadId, totalSize, TransferStatus.FAILED.name)
                        telemetry?.trackUploadFailure("Finish auth_403: ${finishResult.error}", uploadId, duration)
                        Result.Failure("auth_403: permission denied", retryable = false)
                    }
                    else -> {
                        dao.updateProgress(uploadId, totalSize, TransferStatus.FAILED.name)
                        telemetry?.trackUploadFailure("Finish failed: ${finishResult.error}", uploadId, duration)
                        Result.Failure(finishResult.error, finishResult.retryable)
                    }
                }
            }
        }
    }

    private suspend fun startUpload(file: File, totalSize: Long): Result<ResumableStartResponse> {
        return retryWithBackoff { _ ->
            val resp = api.startResumableUpload(
                ResumableStartRequest(
                    filename = file.name,
                    filepath = file.absolutePath,
                    totalSize = totalSize
                )
            )
            if (resp.isSuccessful) {
                resp.body()?.let { Result.Success(it) }
                    ?: Result.Failure("Empty start response", true)
            } else {
                Result.Failure("Start failed: ${resp.code()}", isRetryable(resp.code()))
            }
        }
    }

    // ── Download ──────────────────────────────────────────────────────────────

    suspend fun downloadFile(
        jobId: String,
        outputFile: File,
        totalSize: Long,
        onProgress: (bytesTransferred: Long, totalBytes: Long) -> Unit
    ): Result<File> = withContext(Dispatchers.IO) {
        val tempFile = File(outputFile.parent, "${outputFile.name}.part")
        val existing = dao.getByJobId(jobId)
        var offset = existing?.bytesTransferred ?: 0
        val startTime = System.currentTimeMillis()

        if (tempFile.exists() && offset == 0L) {
            offset = tempFile.length()
        }

        Log.i(TAG, "Downloading $jobId to ${outputFile.name} from offset $offset")

        dao.insert(
            TransferState(
                jobId = jobId,
                filePath = outputFile.absolutePath,
                localTempPath = tempFile.absolutePath,
                totalBytes = totalSize,
                bytesTransferred = offset,
                transferType = TransferType.DOWNLOAD.name,
                status = TransferStatus.ACTIVE.name
            )
        )
        telemetry?.trackDownloadStart(jobId)

        RandomAccessFile(tempFile, "rw").use { raf ->
            raf.setLength(totalSize)
            while (offset < totalSize) {
                val end = min(offset + CHUNK_SIZE - 1, totalSize - 1)
                val range = "bytes=$offset-$end"

                val chunkResult = retryWithBackoff { _ ->
                    val resp = api.downloadOutput(jobId, range)
                    if (resp.isSuccessful) {
                        val body = resp.body()
                        if (body != null) {
                            val bytes = body.bytes()
                            Result.Success(bytes)
                        } else {
                            Result.Failure("Empty download body", true)
                        }
                    } else {
                        Result.Failure("Download failed: ${resp.code()}", isRetryable(resp.code()))
                    }
                }

                when (chunkResult) {
                    is Result.Success -> {
                        raf.seek(offset)
                        raf.write(chunkResult.data)
                        offset += chunkResult.data.size
                        dao.updateProgressByJobId(jobId, offset, TransferStatus.ACTIVE.name)
                        onProgress(offset, totalSize)
                    }
                    is Result.Failure -> {
                        val duration = System.currentTimeMillis() - startTime
                        when {
                            chunkResult.error.contains("401") -> {
                                dao.updateProgressByJobId(jobId, offset, TransferStatus.PAUSED.name)
                                telemetry?.trackDownloadFailure("auth_401 at $offset: ${chunkResult.error}", jobId, duration)
                                return@withContext Result.Failure("auth_401: token missing or rejected", retryable = false)
                            }
                            chunkResult.error.contains("403") -> {
                                dao.updateProgressByJobId(jobId, offset, TransferStatus.FAILED.name)
                                telemetry?.trackDownloadFailure("auth_403 at $offset: ${chunkResult.error}", jobId, duration)
                                return@withContext Result.Failure("auth_403: permission denied", retryable = false)
                            }
                            else -> {
                                dao.updateProgressByJobId(jobId, offset, TransferStatus.PAUSED.name)
                                telemetry?.trackDownloadFailure("Paused at $offset: ${chunkResult.error}", jobId, duration)
                                return@withContext Result.Failure(
                                    "Download paused at $offset: ${chunkResult.error}",
                                    chunkResult.retryable
                                )
                            }
                        }
                    }
                }
            }
        }

        // Guard: only save if job is verified
        val jobResp = try { api.getJob(jobId) } catch (e: Exception) { null }
        val job = jobResp?.body()
        if (job == null || job.status != "done" || job.verifyStatus != "pass") {
            tempFile.delete()
            val reason = when {
                job == null -> "job_status_unknown"
                job.status != "done" -> "job_not_done"
                else -> "verify_status_not_pass: ${job.verifyStatus}"
            }
            dao.updateProgressByJobId(jobId, 0L, TransferStatus.FAILED.name)
            telemetry?.trackDownloadFailure("Verification gate: $reason", jobId, System.currentTimeMillis() - startTime)
            return@withContext Result.Failure("verify_gate: $reason", retryable = false)
        }

        // Checksum verification
        val checksumResp = try { api.getChecksum(jobId) } catch (e: Exception) { null }
        val expectedSha = checksumResp?.body()?.outputSha256
        if (expectedSha != null) {
            val actualSha = computeSha256(tempFile)
            if (!actualSha.equals(expectedSha, ignoreCase = true)) {
                tempFile.delete()
                dao.updateProgressByJobId(jobId, 0L, TransferStatus.FAILED.name)
                telemetry?.trackDownloadFailure("checksum_mismatch", jobId, System.currentTimeMillis() - startTime)
                return@withContext Result.Failure("checksum_mismatch", retryable = false)
            }
        }

        tempFile.renameTo(outputFile)
        dao.deleteByJobId(jobId)
        val duration = System.currentTimeMillis() - startTime
        telemetry?.trackDownloadSuccess(jobId, duration, totalSize)
        Result.Success(outputFile)
    }

    // ── Retry logic ───────────────────────────────────────────────────────────

    private suspend fun <T> retryWithBackoff(
        operation: suspend (attempt: Int) -> Result<T>
    ): Result<T> {
        var lastError = ""
        for (attempt in 0..MAX_RETRIES) {
            when (val result = operation(attempt)) {
                is Result.Success -> return result
                is Result.Failure -> {
                    lastError = result.error
                    if (!result.retryable) return result
                    if (attempt < MAX_RETRIES) {
                        val delay = calculateDelay(attempt)
                        Log.d(TAG, "Retry $attempt/$MAX_RETRIES after ${delay}ms: $lastError")
                        delay(delay)
                    }
                }
            }
        }
        return Result.Failure("Max retries exceeded: $lastError", false)
    }

    private fun calculateDelay(attempt: Int): Long {
        val base = BASE_DELAY_MS * BACKOFF_MULTIPLIER.pow(attempt.toDouble())
        val jitter = base * 0.3 * Random.nextDouble()
        return min((base + jitter).toLong(), MAX_DELAY_MS)
    }

    private fun isRetryable(statusCode: Int): Boolean {
        return statusCode in listOf(408, 429, 500, 502, 503, 504)
    }

    private fun isAuthError(statusCode: Int): Boolean = statusCode == 401 || statusCode == 403

    // ── Constraints ─────────────────────────────────────────────────────────────

    fun isWifiConnected(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val capabilities = cm.getNetworkCapabilities(network) ?: return false
        return capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    }

    fun isCellularConnected(): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val network = cm.activeNetwork ?: return false
        val capabilities = cm.getNetworkCapabilities(network) ?: return false
        return capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
    }

    private fun computeSha256(file: File): String {
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(8192)
            var read: Int
            while (input.read(buffer).also { read = it } != -1) {
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }
}
