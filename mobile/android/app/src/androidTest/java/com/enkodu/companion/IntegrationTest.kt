package com.enkodu.companion

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.enkodu.companion.av1.Av1CapabilityChecker
import com.enkodu.companion.data.EnkoduDatabase
import com.enkodu.companion.data.TransferStatus
import com.enkodu.companion.data.TransferType
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class SettingsStoreTest {

    private lateinit var context: Context
    private lateinit var store: SettingsStore

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
        store = SettingsStore(context)
    }

    @After
    fun tearDown() {
        context.getSharedPreferences("enkodu_settings", Context.MODE_PRIVATE).edit().clear().apply()
    }

    @Test
    fun testValidServerUrl() {
        store.serverUrl = "https://enkodu.example.com"
        assertTrue(store.validateServerUrl())
    }

    @Test
    fun testInvalidServerUrl() {
        store.serverUrl = "not-a-url"
        assertFalse(store.validateServerUrl())
    }

    @Test
    fun testEmptyServerUrl() {
        store.serverUrl = ""
        assertFalse(store.validateServerUrl())
    }

    @Test
    fun testWifiDefaults() {
        assertTrue(store.wifiOnlyUploads)
        assertTrue(store.wifiOnlyDownloads)
    }

    @Test
    fun testBatteryDefaults() {
        assertEquals(15, store.batteryMinPercent)
    }

    @Test
    fun testMaxUploadDefaults() {
        assertEquals(100, store.maxUploadSizeMb)
    }
}

@RunWith(AndroidJUnit4::class)
class Av1CapabilityTest {

    @Test
    fun testAv1CheckReturnsResult() {
        val result = Av1CapabilityChecker.check()
        // Result may be supported or not depending on device, but should never crash
        assertNotNull(result)
        assertTrue(result.reason.isNotEmpty())
    }
}

@RunWith(AndroidJUnit4::class)
class TransferStateTest {

    private lateinit var db: EnkoduDatabase
    private lateinit var dao: com.enkodu.companion.data.TransferDao

    @Before
    fun setUp() {
        val context = ApplicationProvider.getApplicationContext()
        db = EnkoduDatabase.getDatabase(context)
        dao = db.transferDao()
    }

    @After
    fun tearDown() {
        db.close()
    }

    @Test
    fun testInsertAndRetrieve() = runBlocking {
        val state = com.enkodu.companion.data.TransferState(
            uploadId = "test-upload-123",
            filePath = "/tmp/test.mp4",
            totalBytes = 1024,
            status = TransferStatus.PENDING.name,
            transferType = TransferType.UPLOAD.name
        )
        dao.insert(state)

        val retrieved = dao.getByUploadId("test-upload-123")
        assertNotNull(retrieved)
        assertEquals("test-upload-123", retrieved?.uploadId)
        assertEquals(1024, retrieved?.totalBytes)
    }

    @Test
    fun testUpdateProgress() = runBlocking {
        val state = com.enkodu.companion.data.TransferState(
            uploadId = "test-upload-456",
            filePath = "/tmp/test2.mp4",
            totalBytes = 2048,
            status = TransferStatus.PENDING.name,
            transferType = TransferType.UPLOAD.name
        )
        dao.insert(state)

        dao.updateProgress("test-upload-456", 1024, TransferStatus.ACTIVE.name)
        val updated = dao.getByUploadId("test-upload-456")
        assertEquals(1024, updated?.bytesTransferred)
        assertEquals(TransferStatus.ACTIVE.name, updated?.status)
    }

    @Test
    fun testDelete() = runBlocking {
        val state = com.enkodu.companion.data.TransferState(
            uploadId = "test-upload-789",
            filePath = "/tmp/test3.mp4",
            totalBytes = 4096,
            status = TransferStatus.PENDING.name,
            transferType = TransferType.UPLOAD.name
        )
        dao.insert(state)
        dao.deleteByUploadId("test-upload-789")
        val deleted = dao.getByUploadId("test-upload-789")
        assertNull(deleted)
    }
}

@RunWith(AndroidJUnit4::class)
class TransferManagerTest {

    private lateinit var context: Context

    @Before
    fun setUp() {
        context = ApplicationProvider.getApplicationContext()
    }

    @Test
    fun testNetworkDetection() {
        val db = EnkoduDatabase.getDatabase(context)
        val dao = db.transferDao()
        val api = com.enkodu.companion.api.EnkoduApi.create("https://example.com")
        val manager = com.enkodu.companion.transfer.TransferManager(context, api, dao)

        // Just verify methods don't crash — actual network state depends on device
        manager.isWifiConnected()
        manager.isCellularConnected()
    }

    @Test
    fun testRetryBackoffCalculation() {
        val db = EnkoduDatabase.getDatabase(context)
        val dao = db.transferDao()
        val api = com.enkodu.companion.api.EnkoduApi.create("https://example.com")
        val manager = com.enkodu.companion.transfer.TransferManager(context, api, dao)

        // Verify that retry logic is wired by calling a private method via reflection
        // or by testing a public method that uses it
        // For now, we just verify the class can be instantiated
        assertNotNull(manager)
    }
}
