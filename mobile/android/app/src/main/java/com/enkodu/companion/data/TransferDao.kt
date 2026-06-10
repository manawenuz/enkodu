package com.enkodu.companion.data

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Update

@Dao
interface TransferDao {
    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insert(state: TransferState)

    @Update
    suspend fun update(state: TransferState)

    @Query("SELECT * FROM transfers WHERE uploadId = :uploadId")
    suspend fun getByUploadId(uploadId: String): TransferState?

    @Query("SELECT * FROM transfers WHERE jobId = :jobId")
    suspend fun getByJobId(jobId: String): TransferState?

    @Query("SELECT * FROM transfers WHERE filePath = :filePath")
    suspend fun getByFilePath(filePath: String): TransferState?

    @Query("SELECT * FROM transfers WHERE status IN ('PENDING', 'PAUSED', 'FAILED')")
    suspend fun getPendingOrPaused(): List<TransferState>

    @Query("SELECT * FROM transfers WHERE status = 'ACTIVE'")
    suspend fun getActive(): List<TransferState>

    @Query("DELETE FROM transfers WHERE uploadId = :uploadId")
    suspend fun deleteByUploadId(uploadId: String)

    @Query("DELETE FROM transfers WHERE jobId = :jobId")
    suspend fun deleteByJobId(jobId: String)

    @Query("UPDATE transfers SET bytesTransferred = :bytes, status = :status, updatedAt = :now WHERE uploadId = :uploadId")
    suspend fun updateProgress(uploadId: String, bytes: Long, status: String, now: Long = System.currentTimeMillis())

    @Query("UPDATE transfers SET bytesTransferred = :bytes, status = :status, updatedAt = :now WHERE jobId = :jobId")
    suspend fun updateProgressByJobId(jobId: String, bytes: Long, status: String, now: Long = System.currentTimeMillis())
}
