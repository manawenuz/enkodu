package com.enkodu.companion.data

import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey

@Entity(
    tableName = "transfers",
    indices = [
        Index(value = ["uploadId"], unique = true),
        Index(value = ["jobId"], unique = true),
        Index(value = ["status"])
    ]
)
data class TransferState(
    @PrimaryKey(autoGenerate = true)
    val id: Long = 0,
    val uploadId: String? = null,
    val jobId: String? = null,
    val filePath: String,
    val localTempPath: String? = null,
    val totalBytes: Long = 0,
    val bytesTransferred: Long = 0,
    val status: String = TransferStatus.PENDING.name,
    val transferType: String = TransferType.UPLOAD.name,
    val lastError: String? = null,
    val retryCount: Int = 0,
    val networkType: String? = null,
    val createdAt: Long = System.currentTimeMillis(),
    val updatedAt: Long = System.currentTimeMillis()
)

enum class TransferStatus {
    PENDING, ACTIVE, PAUSED, FAILED, DONE, CANCELLED
}

enum class TransferType {
    UPLOAD, DOWNLOAD
}
