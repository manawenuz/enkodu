package com.enkodu.companion.transfer

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import com.enkodu.companion.EnkoduApp
import com.enkodu.companion.api.EnkoduApi
import com.enkodu.companion.av1.Av1CapabilityChecker
import com.enkodu.companion.data.EnkoduDatabase
import com.enkodu.companion.TelemetryManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

class UploadWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    companion object {
        private const val KEY_FILE_PATH = "file_path"
        private const val KEY_SERVER_URL = "server_url"
        private const val KEY_UPLOAD_ID = "upload_id"
        private const val KEY_REQUIRES_WIFI = "requires_wifi"

        fun schedule(context: Context, filePath: String, serverUrl: String, requiresWifi: Boolean = true) {
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(if (requiresWifi) NetworkType.UNMETERED else NetworkType.CONNECTED)
                .setRequiresBatteryNotLow(true)
                .build()

            val inputData = workDataOf(
                KEY_FILE_PATH to filePath,
                KEY_SERVER_URL to serverUrl,
                KEY_REQUIRES_WIFI to requiresWifi
            )

            val request = OneTimeWorkRequestBuilder<UploadWorker>()
                .setConstraints(constraints)
                .setInputData(inputData)
                .addTag("enkodu_upload")
                .build()

            WorkManager.getInstance(context).enqueue(request)
        }

        fun scheduleResume(context: Context, uploadId: String, serverUrl: String) {
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .setRequiresBatteryNotLow(true)
                .build()

            val inputData = workDataOf(
                KEY_UPLOAD_ID to uploadId,
                KEY_SERVER_URL to serverUrl,
                KEY_FILE_PATH to "" // Will be fetched from DB
            )

            val request = OneTimeWorkRequestBuilder<UploadWorker>()
                .setConstraints(constraints)
                .setInputData(inputData)
                .addTag("enkodu_upload")
                .build()

            WorkManager.getInstance(context).enqueue(request)
        }
    }

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val filePath = inputData.getString(KEY_FILE_PATH) ?: return@withContext Result.failure()
        val serverUrl = inputData.getString(KEY_SERVER_URL) ?: return@withContext Result.failure()
        val uploadId = inputData.getString(KEY_UPLOAD_ID)

        val app = applicationContext as EnkoduApp
        val api = EnkoduApi.create(serverUrl, app.authStore)
        val telemetry = TelemetryManager(applicationContext, api)
        val manager = TransferManager(
            applicationContext,
            api,
            app.database.transferDao(),
            telemetry
        )

        val file = File(filePath)
        if (!file.exists()) {
            return@withContext Result.failure(
                workDataOf("error" to "File not found: $filePath")
            )
        }

        // If resuming from an existing upload, pass the uploadId
        // The TransferManager handles resume automatically from DB state
        val result = manager.uploadFile(file) { bytes, total ->
            // Progress is tracked via the database, but we could also report here
            setProgressAsync(workDataOf("bytes" to bytes, "total" to total))
        }

        when (result) {
            is TransferManager.Result.Success -> {
                Result.success(
                    workDataOf(
                        "job_id" to result.data.jobId,
                        "priority_position" to result.data.priorityPosition
                    )
                )
            }
            is TransferManager.Result.Failure -> {
                if (result.retryable) {
                    Result.retry()
                } else {
                    Result.failure(workDataOf("error" to result.error))
                }
            }
        }
    }
}

class DownloadWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    companion object {
        private const val KEY_JOB_ID = "job_id"
        private const val KEY_SERVER_URL = "server_url"
        private const val KEY_OUTPUT_PATH = "output_path"
        private const val KEY_TOTAL_SIZE = "total_size"
        private const val KEY_REQUIRES_WIFI = "requires_wifi"

        fun schedule(
            context: Context,
            jobId: String,
            serverUrl: String,
            outputPath: String,
            totalSize: Long,
            requiresWifi: Boolean = true
        ) {
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(if (requiresWifi) NetworkType.UNMETERED else NetworkType.CONNECTED)
                .setRequiresBatteryNotLow(true)
                .build()

            val inputData = workDataOf(
                KEY_JOB_ID to jobId,
                KEY_SERVER_URL to serverUrl,
                KEY_OUTPUT_PATH to outputPath,
                KEY_TOTAL_SIZE to totalSize,
                KEY_REQUIRES_WIFI to requiresWifi
            )

            val request = OneTimeWorkRequestBuilder<DownloadWorker>()
                .setConstraints(constraints)
                .setInputData(inputData)
                .addTag("enkodu_download")
                .build()

            WorkManager.getInstance(context).enqueue(request)
        }
    }

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        val jobId = inputData.getString(KEY_JOB_ID) ?: return@withContext Result.failure()
        val serverUrl = inputData.getString(KEY_SERVER_URL) ?: return@withContext Result.failure()
        val outputPath = inputData.getString(KEY_OUTPUT_PATH) ?: return@withContext Result.failure()
        val totalSize = inputData.getLong(KEY_TOTAL_SIZE, 0)

        val app = applicationContext as EnkoduApp
        val api = EnkoduApi.create(serverUrl, app.authStore)
        val telemetry = TelemetryManager(applicationContext, api)
        val manager = TransferManager(
            applicationContext,
            api,
            app.database.transferDao(),
            telemetry
        )

        if (!Av1CapabilityChecker.isSupported()) {
            return@withContext Result.failure(
                workDataOf("error" to "av1_unsupported: This device cannot play AV1 efficiently")
            )
        }

        val outputFile = File(outputPath)
        val result = manager.downloadFile(jobId, outputFile, totalSize) { bytes, total ->
            setProgressAsync(workDataOf("bytes" to bytes, "total" to total))
        }

        when (result) {
            is TransferManager.Result.Success -> {
                val saved = saveToMediaStore(result.data)
                if (saved) {
                    result.data.delete() // remove temp copy after MediaStore insert
                    Result.success()
                } else {
                    // MediaStore failed but file is on disk - still count as success
                    Result.success(workDataOf("warning" to "mediastore_save_failed"))
                }
            }
            is TransferManager.Result.Failure -> {
                if (result.retryable) {
                    Result.retry()
                } else {
                    Result.failure(workDataOf("error" to result.error))
                }
            }
        }
    }

    private fun saveToMediaStore(file: File): Boolean {
        return try {
            val stem = file.nameWithoutExtension.removeSuffix("_av1")
            val displayName = "${stem}_av1.mp4"
            val values = android.content.ContentValues().apply {
                put(android.provider.MediaStore.Video.Media.DISPLAY_NAME, displayName)
                put(android.provider.MediaStore.Video.Media.MIME_TYPE, "video/mp4")
                put(android.provider.MediaStore.Video.Media.RELATIVE_PATH, "Movies/Enkodu")
                put(android.provider.MediaStore.Video.Media.IS_PENDING, 1)
            }
            val resolver = applicationContext.contentResolver
            val uri = resolver.insert(android.provider.MediaStore.Video.Media.EXTERNAL_CONTENT_URI, values)
                ?: return false
            resolver.openOutputStream(uri)?.use { out ->
                file.inputStream().copyTo(out)
            }
            values.clear()
            values.put(android.provider.MediaStore.Video.Media.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
            true
        } catch (e: Exception) {
            android.util.Log.e("DownloadWorker", "MediaStore save failed: ${e.message}")
            false
        }
    }
}
