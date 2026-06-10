package com.enkodu.companion.api

import com.enkodu.companion.auth.AuthHeaderInterceptor
import com.enkodu.companion.auth.AuthTokenProvider
import com.google.gson.annotations.SerializedName
import okhttp3.OkHttpClient
import okhttp3.RequestBody
import okhttp3.logging.HttpLoggingInterceptor
import retrofit2.Response
import retrofit2.Retrofit
import retrofit2.converter.gson.GsonConverterFactory
import retrofit2.http.Body
import retrofit2.http.GET
import retrofit2.http.Header
import retrofit2.http.POST
import retrofit2.http.PUT
import retrofit2.http.Path
import java.util.concurrent.TimeUnit

interface EnkoduApi {

    companion object {
        fun create(baseUrl: String, tokenProvider: AuthTokenProvider? = null): EnkoduApi {
            val clientBuilder = OkHttpClient.Builder()
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(300, TimeUnit.SECONDS)
                .writeTimeout(300, TimeUnit.SECONDS)
            tokenProvider?.let { clientBuilder.addInterceptor(AuthHeaderInterceptor(it)) }
            clientBuilder.addInterceptor(HttpLoggingInterceptor().apply {
                level = HttpLoggingInterceptor.Level.BASIC
            })
            val client = clientBuilder.build()

            return Retrofit.Builder()
                .baseUrl(baseUrl)
                .client(client)
                .addConverterFactory(GsonConverterFactory.create())
                .build()
                .create(EnkoduApi::class.java)
        }
    }

    // ── Server status ─────────────────────────────────────────────────────────

    @GET("status")
    suspend fun getStatus(): Response<StatusResponse>

    @GET("version")
    suspend fun getVersion(): Response<VersionResponse>

    @GET("healthz")
    suspend fun getHealthz(): Response<HealthzResponse>

    // ── Jobs ──────────────────────────────────────────────────────────────────

    @GET("jobs/{jobId}")
    suspend fun getJob(@Path("jobId") jobId: String): Response<JobResponse>

    @GET("jobs/{jobId}/output")
    suspend fun downloadOutput(
        @Path("jobId") jobId: String,
        @Header("Range") range: String? = null
    ): Response<okhttp3.ResponseBody>

    @GET("jobs/{jobId}/checksum")
    suspend fun getChecksum(@Path("jobId") jobId: String): Response<ChecksumResponse>

    // ── Resumable upload ───────────────────────────────────────────────────────

    @POST("jobs/upload/resumable/start")
    suspend fun startResumableUpload(
        @Body request: ResumableStartRequest
    ): Response<ResumableStartResponse>

    @PUT("jobs/upload/resumable/{uploadId}/chunk")
    suspend fun uploadChunk(
        @Path("uploadId") uploadId: String,
        @Header("Content-Range") contentRange: String,
        @Body body: RequestBody
    ): Response<ChunkResponse>

    @POST("jobs/upload/resumable/{uploadId}/finish")
    suspend fun finishResumableUpload(
        @Path("uploadId") uploadId: String
    ): Response<UploadFinishResponse>

    // ── Control ───────────────────────────────────────────────────────────────

    @GET("control")
    suspend fun getControl(): Response<ControlResponse>

    @POST("control/{cmd}")
    suspend fun setControl(@Path("cmd") cmd: String): Response<Unit>

    @POST("telemetry")
    suspend fun postTelemetry(@Body request: TelemetryRequest): Response<Unit>
}

// ── Response models ─────────────────────────────────────────────────────────

data class StatusResponse(
    val ok: Boolean,
    val pending: Long,
    val active: Long,
    val done: Long,
    val failed: Long
)

data class VersionResponse(
    val version: String
)

data class HealthzResponse(
    val ok: Boolean
)

data class JobResponse(
    val id: String,
    val status: String,
    val percent: Double?,
    val fps: Double?,
    val speed: String?,
    val worker: String?,
    val error: String?,
    val outputSize: Long?,
    val sourceSize: Long?,
    val verifyStatus: String?,
    val verifyDetail: String?
)

data class ChecksumResponse(
    val jobId: String,
    val status: String,
    val sourceSha256: String?,
    val outputSha256: String?
)

data class ResumableStartRequest(
    val filename: String,
    val filepath: String?,
    val totalSize: Long
)

data class ResumableStartResponse(
    val uploadId: String,
    val chunkSize: Long,
    val expiresIn: Long
)

data class ChunkResponse(
    val ok: Boolean,
    val received: Long,
    val total: Long
)

data class UploadFinishResponse(
    val jobId: String,
    val priorityPosition: Long,
    val clientName: String,
    val deduped: Boolean = false
)

data class ControlResponse(
    val command: String
)

data class TelemetryRequest(
    val clientId: String?,
    val eventType: String,
    val eventDetail: String?,
    val jobId: String?,
    val platform: String?,
    val success: Boolean,
    val durationMs: Long?,
    val bytesTransferred: Long?
)
