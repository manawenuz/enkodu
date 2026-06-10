# Enkodu Android Companion

Native Android companion app for Enkodu. Submits videos to the Enkodu queue for AV1 transcoding, monitors progress, and downloads verified AV1 outputs.

**Important**: The upgrade flow is **disabled** on devices without hardware AV1 decode support. See [AV1 Hardware Decode Gate](#av1-hardware-decode-gate) below.

## Status

| Feature | Status | Notes |
|---|---|---|
| AV1 hardware decode detection | Planned | Phase 3 |
| Server setup screen | Planned | Phase 3 |
| Video picker (SAF/Photo Picker) | Planned | Phase 3 |
| Upload with progress | Planned | Phase 3 |
| Job status polling | Planned | Phase 3 |
| Download AV1 output | Planned | Phase 3 |
| Save to MediaStore | Planned | Phase 3 |
| Resumable uploads | Planned | Phase 5 |
| Resumable downloads | Planned | Phase 5 |
| Background transfer survival | Planned | Phase 5 |
| Local job history | Planned | Phase 3 |

See [PRD](../../docs/obsidian-vault/05-Product/Missing%20Companion%20Clients%20PRD.md) for full requirements.

## Prerequisites

- Android Studio (latest stable)
- Android SDK 34+
- Kotlin 2.0+
- Java 17+

## Project Structure

```
mobile/android/
├── app/
│   ├── build.gradle.kts
│   ├── src/
│   │   ├── main/
│   │   │   ├── AndroidManifest.xml
│   │   │   ├── kotlin/
│   │   │   │   └── com/
│   │   │   │       └── enkodu/
│   │   │   │           ├── MainActivity.kt
│   │   │   │           ├── app/
│   │   │   │           │   ├── EnkoduApp.kt
│   │   │   │           │   ├── navigation/
│   │   │   │           │   ├── theme/
│   │   │   │           │   └── Theme.kt
│   │   │   │           ├── data/
│   │   │   │           │   ├── api/
│   │   │   │           │   │   └── EnkoduApi.kt
│   │   │   │           │   ├── model/
│   │   │   │           │   ├── repository/
│   │   │   │           │   └── database/
│   │   │   │           ├── ui/
│   │   │   │           │   ├── screens/
│   │   │   │           │   │   ├── ServerSetupScreen.kt
│   │   │   │           │   │   ├── CapabilityScreen.kt
│   │   │   │           │   │   ├── HomeScreen.kt
│   │   │   │           │   │   ├── VideoPickerScreen.kt
│   │   │   │           │   │   ├── UploadScreen.kt
│   │   │   │           │   │   ├── QueueScreen.kt
│   │   │   │           │   │   ├── DownloadScreen.kt
│   │   │   │           │   │   └── HistoryScreen.kt
│   │   │   │           │   └── components/
│   │   │   │           └── service/
│   │   │   │               ├── TransferService.kt
│   │   │   │               └── TransferManager.kt
│   │   │   └── res/
│   │   │       ├── values/
│   │   │       └── layout/
│   │   └── test/
│   └── proguard-rules.pro
├── build.gradle.kts
├── settings.gradle.kts
├── gradle.properties
└── README.md (this file)
```

## AV1 Hardware Decode Gate

The app **must not** allow AV1 upgrade on devices without hardware AV1 decode support.

### Implementation

```kotlin
// In CapabilityChecker.kt
fun checkAv1HardwareDecode(context: Context): Boolean {
    val mediaCodecList = MediaCodecList(MediaCodecList.ALL_CODECS)
    return mediaCodecList.codecInfos.any { codecInfo ->
        codecInfo.isEncoder == false &&
        codecInfo.supportedTypes.any { type ->
            type.equals(MediaFormat.MIMETYPE_VIDEO_AV1, ignoreCase = true)
        } &&
        codecInfo.isHardwareAccelerated &&
        codecInfo.isSoftwareOnly == false
    }
}
```

### Behavior Matrix

| AV1 Hardware Decode | Upgrade Video Button | Queue/Status Viewing | Upload for AV1 | Download AV1 |
|---|---|---|---|---|
| Supported | Enabled | Enabled | Allowed | Allowed |
| Unsupported | Disabled | Enabled | **Blocked** | **Blocked** |

### User Messaging

**Unsupported devices see:**
> "This device cannot play AV1 efficiently. AV1 upgrade is disabled on this device. You can still view queue status, but videos cannot be submitted for AV1 conversion or downloaded as AV1 outputs."

## Configuration

Server URL and user preferences are stored in `SharedPreferences`.

```kotlin
data class AppConfig(
    val serverUrl: String,
    val userName: String? = null,
    val wifiOnlyUpload: Boolean = true,
    val wifiOnlyDownload: Boolean = true,
    val maxUploadSizeMb: Int = 2048, // 2GB default
    val batteryPauseThreshold: Int = 15, // %
    val batteryResumeThreshold: Int = 20  // %
)
```

## API Endpoints

| Endpoint | Method | Description |
|---|---|---|
| `/jobs/upload` | POST | Upload video for processing |
| `/jobs/{id}` | GET | Poll job status |
| `/jobs/{id}/output` | GET | Download AV1 output (supports Range headers) |
| `/status` | GET | Queue status |
| `/jobs/live` | GET | Live job updates (SSE) |

Future (Phase 5):
- `POST /jobs/upload/resumable/start`
- `PUT /jobs/upload/resumable/{upload_id}/chunk`
- `POST /jobs/upload/resumable/{upload_id}/finish`

## Transfer Management

### Upload Flow

1. Check AV1 hardware decode support (gate)
2. Pick video via SAF/Photo Picker
3. Get file size and verify read permission
4. Upload with progress reporting
5. On success: receive job_id, start polling
6. On failure: retry with exponential backoff or report error

### Download Flow

1. Wait for job status = "done" and verify_status = "pass"
2. Download AV1 output with progress
3. Verify file integrity
4. Save to MediaStore (or user-selected location)
5. Notify user of completion

### Retry Policy

| Error Type | Retry? | Max Retries | Base Delay | Multiplier |
|---|---|---|---|---|
| Network timeout | Yes | 10 | 500ms | 1.5x |
| HTTP 429/502/503/504 | Yes | 10 | 500ms | 1.5x |
| HTTP 400/401/403/404 | No | - | - | - |
| Disk full | No | - | - | - |

### Constraints

- **WiFi-only uploads**: Default block uploads > 100MB on cellular
- **WiFi-only downloads**: Default block all downloads on cellular
- **Battery pause**: Pause if battery < 15%, resume when > 20%
- **Thermal pause**: Pause if thermal throttling detected

## Background Handling

- Foreground service with notification for active transfers
- WorkManager for deferred/retry operations
- Transfer state persisted to Room SQLite database
- Survives app process death via persistent state

## Testing

### Manual Tests

- [ ] Unsupported device: verify upgrade button is disabled
- [ ] Supported device: verify fixture video flows end-to-end
- [ ] Network drop < 30s: verify auto-retry
- [ ] Network drop > 30s: verify pause and resume
- [ ] Battery < 15%: verify transfer pauses
- [ ] Battery > 20%: verify transfer resumes
- [ ] App killed during transfer: verify state recovery on restart

### Automated Tests

```kotlin
// AV1 decode capability test
class Av1CapabilityTest {
    @Test
    fun testAv1HardwareDecodeDetection() {
        // Mock MediaCodecList to return known supported/unsupported configurations
        // Verify checkAv1HardwareDecode returns correct result
    }
}

// Upload retry test
class UploadRetryTest {
    @Test
    fun testRetryOnNetworkError() {
        // Simulate network failure, verify retry with backoff
    }
}
```

## Building

```bash
# Build debug APK
./gradlew :app:assembleDebug

# Build release APK
./gradlew :app:assembleRelease

# Install to connected device
./gradlew :app:installDebug
```

## Recommended Dependencies

```kotlin
// build.gradle.kts (app)
dependencies {
    // Jetpack Compose
    implementation("androidx.activity:activity-compose:1.9.0")
    implementation("androidx.compose.ui:ui:1.6.0")
    implementation("androidx.compose.material3:material3:1.2.0")
    implementation("androidx.compose.ui:ui-tooling-preview:1.6.0")
    debugImplementation("androidx.compose.ui:ui-tooling:1.6.0")
    
    // Lifecycle
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.0")
    
    // Navigation
    implementation("androidx.navigation:navigation-compose:2.8.0")
    
    // Media3
    implementation("androidx.media3:media3-exoplayer:1.3.0")
    implementation("androidx.media3:media3-ui:1.3.0")
    
    // Storage
    implementation("androidx.room:room-runtime:2.7.0")
    kapt("androidx.room:room-compiler:2.7.0")
    implementation("androidx.datastore:datastore-preferences:1.1.0")
    
    // WorkManager
    implementation("androidx.work:work-runtime-ktx:2.9.0")
    
    // Network
    implementation("com.squareup.retrofit2:retrofit:2.11.0")
    implementation("com.squareup.retrofit2:converter-gson:2.11.0")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.squareup.okhttp3:logging-interceptor:4.12.0")
    
    // Coil for image loading
    implementation("io.coil-kt:coil-compose:2.6.0")
    
    // Accompanist for permissions
    implementation("com.google.accompanist:accompanist-permissions:0.34.0")
}
```

## References

- [PRD: Missing Companion Clients](../../docs/obsidian-vault/05-Product/Missing%20Companion%20Clients%20PRD.md)
- [Mobile Transfer Manager Design](../../docs/obsidian-vault/05-Product/Mobile%20Transfer%20Manager%20Design.md)
- [Android MediaCodecInfo Documentation](https://developer.android.com/reference/android/media/MediaCodecInfo)
- [Android MediaFormat Documentation](https://developer.android.com/reference/android/media/MediaFormat)
