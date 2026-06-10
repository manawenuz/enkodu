package com.enkodu.companion.transfer

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.enkodu.companion.MainActivity
import com.enkodu.companion.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

class TransferService : Service() {

    companion object {
        private const val TAG = "TransferService"
        private const val CHANNEL_ID = "enkodu_transfers"
        private const val NOTIFICATION_ID = 1
        const val ACTION_START_UPLOAD = "com.enkodu.companion.START_UPLOAD"
        const val ACTION_START_DOWNLOAD = "com.enkodu.companion.START_DOWNLOAD"
        const val EXTRA_FILE_PATH = "file_path"
        const val EXTRA_JOB_ID = "job_id"
        const val EXTRA_TOTAL_SIZE = "total_size"
        const val EXTRA_SERVER_URL = "server_url"
    }

    private val serviceScope = CoroutineScope(Dispatchers.IO + Job())
    private lateinit var notificationManager: NotificationManager

    override fun onCreate() {
        super.onCreate()
        notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START_UPLOAD -> {
                val filePath = intent.getStringExtra(EXTRA_FILE_PATH) ?: return START_NOT_STICKY
                val serverUrl = intent.getStringExtra(EXTRA_SERVER_URL) ?: return START_NOT_STICKY
                startForeground()
                startUpload(filePath, serverUrl)
            }
            ACTION_START_DOWNLOAD -> {
                val jobId = intent.getStringExtra(EXTRA_JOB_ID) ?: return START_NOT_STICKY
                val serverUrl = intent.getStringExtra(EXTRA_SERVER_URL) ?: return START_NOT_STICKY
                val totalSize = intent.getLongExtra(EXTRA_TOTAL_SIZE, 0)
                startForeground()
                startDownload(jobId, serverUrl, totalSize)
            }
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        super.onDestroy()
        serviceScope.cancel()
    }

    private fun startUpload(filePath: String, serverUrl: String) {
        serviceScope.launch {
            val app = application as com.enkodu.companion.EnkoduApp
            val api = com.enkodu.companion.api.EnkoduApi.create(serverUrl, app.authStore)
            val manager = TransferManager(applicationContext, api, app.database.transferDao())

            val file = java.io.File(filePath)
            val result = manager.uploadFile(file) { bytes, total ->
                updateProgressNotification(bytes, total, "Uploading ${file.name}")
            }

            when (result) {
                is TransferManager.Result.Success -> {
                    showCompletionNotification("Upload complete", "${file.name} queued for processing")
                }
                is TransferManager.Result.Failure -> {
                    showCompletionNotification("Upload failed", result.error, true)
                }
            }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun startDownload(jobId: String, serverUrl: String, totalSize: Long) {
        serviceScope.launch {
            val app = application as com.enkodu.companion.EnkoduApp
            val api = com.enkodu.companion.api.EnkoduApi.create(serverUrl, app.authStore)
            val manager = TransferManager(applicationContext, api, app.database.transferDao())

            val outputDir = applicationContext.getExternalFilesDir(android.os.Environment.DIRECTORY_MOVIES)
                ?: applicationContext.filesDir
            val outputFile = java.io.File(outputDir, "$jobId.av1.mp4")

            val result = manager.downloadFile(jobId, outputFile, totalSize) { bytes, total ->
                updateProgressNotification(bytes, total, "Downloading output")
            }

            when (result) {
                is TransferManager.Result.Success -> {
                    showCompletionNotification("Download complete", "Saved to ${outputFile.name}")
                }
                is TransferManager.Result.Failure -> {
                    showCompletionNotification("Download failed", result.error, true)
                }
            }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Enkodu Transfers",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Shows progress for video uploads and downloads"
            }
            notificationManager.createNotificationChannel(channel)
        }
    }

    private fun startForeground() {
        val notification = buildNotification("Starting transfer...", 0, 0)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(content: String, progress: Int, max: Int): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Enkodu")
            .setContentText(content)
            .setSmallIcon(R.drawable.ic_notification)
            .setProgress(max, progress, max == 0)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
    }

    private fun updateProgressNotification(bytes: Long, total: Long, title: String) {
        val percent = if (total > 0) ((bytes * 100) / total).toInt() else 0
        val text = "$percent% — ${formatBytes(bytes)} / ${formatBytes(total)}"
        val notification = buildNotification("$title: $text", percent, 100)
        notificationManager.notify(NOTIFICATION_ID, notification)
    }

    private fun showCompletionNotification(title: String, text: String, isError: Boolean = false) {
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )

        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(text)
            .setSmallIcon(if (isError) R.drawable.ic_notification_error else R.drawable.ic_notification)
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .build()

        notificationManager.notify(System.currentTimeMillis().toInt(), notification)
    }

    private fun formatBytes(bytes: Long): String {
        return when {
            bytes >= 1_000_000_000 -> "%.2f GB".format(bytes / 1_000_000_000.0)
            bytes >= 1_000_000 -> "%.1f MB".format(bytes / 1_000_000.0)
            bytes >= 1_000 -> "%.0f KB".format(bytes / 1_000.0)
            else -> "$bytes B"
        }
    }
}
