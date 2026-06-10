package com.enkodu.companion

import android.content.Context
import android.os.Build
import com.enkodu.companion.api.EnkoduApi
import com.enkodu.companion.api.TelemetryRequest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.UUID

class TelemetryManager(context: Context, private val api: EnkoduApi) {
    private val prefs = context.getSharedPreferences("enkodu_telemetry", Context.MODE_PRIVATE)
    private val clientId: String
        get() = prefs.getString("client_id", null) ?: run {
            val id = UUID.randomUUID().toString()
            prefs.edit().putString("client_id", id).apply()
            id
        }
    private val platform = "android-${Build.VERSION.RELEASE}"
    private val scope = CoroutineScope(Dispatchers.IO)

    fun trackUploadStart(jobId: String? = null) {
        track("upload_start", jobId = jobId)
    }

    fun trackUploadSuccess(jobId: String, durationMs: Long, bytesTransferred: Long) {
        track("upload_finish", jobId = jobId, success = true, durationMs = durationMs, bytesTransferred = bytesTransferred)
    }

    fun trackUploadFailure(error: String, jobId: String? = null, durationMs: Long? = null) {
        track("upload_finish", jobId = jobId, success = false, durationMs = durationMs, detail = error)
    }

    fun trackDownloadStart(jobId: String) {
        track("download_start", jobId = jobId)
    }

    fun trackDownloadSuccess(jobId: String, durationMs: Long, bytesTransferred: Long) {
        track("download_finish", jobId = jobId, success = true, durationMs = durationMs, bytesTransferred = bytesTransferred)
    }

    fun trackDownloadFailure(error: String, jobId: String, durationMs: Long? = null) {
        track("download_finish", jobId = jobId, success = false, durationMs = durationMs, detail = error)
    }

    fun trackAppLaunch() {
        track("app_launch")
    }

    fun trackAv1GateResult(supported: Boolean) {
        track("av1_gate", success = supported, detail = if (supported) "supported" else "unsupported")
    }

    fun trackError(errorType: String, detail: String? = null) {
        track("error", detail = "$errorType: ${detail ?: ""}")
    }

    private fun track(
        eventType: String,
        jobId: String? = null,
        success: Boolean = true,
        durationMs: Long? = null,
        bytesTransferred: Long? = null,
        detail: String? = null
    ) {
        scope.launch {
            try {
                val request = TelemetryRequest(
                    clientId = clientId,
                    eventType = eventType,
                    eventDetail = detail,
                    jobId = jobId,
                    platform = platform,
                    success = success,
                    durationMs = durationMs,
                    bytesTransferred = bytesTransferred
                )
                val resp = api.postTelemetry(request)
                if (!resp.isSuccessful) {
                    android.util.Log.w("Telemetry", "Failed to send: ${resp.code()}")
                }
            } catch (e: Exception) {
                android.util.Log.w("Telemetry", "Error sending: ${e.message}")
            }
        }
    }
}
