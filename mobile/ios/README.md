# Enkodu iOS Companion

Native iOS companion app for Enkodu. Submits videos to the Enkodu queue for AV1 transcoding, monitors progress, and downloads verified AV1 outputs.

**Important**: The upgrade flow is **disabled** on devices without hardware AV1 decode support. See [AV1 Hardware Decode Gate](#av1-hardware-decode-gate) below.

## Status

| Feature | Status | Notes |
|---|---|---|
| AV1 hardware decode detection | Planned | Phase 4 |
| Server setup screen | Planned | Phase 4 |
| Video picker (Photos/Files) | Planned | Phase 4 |
| Upload with progress | Planned | Phase 4 |
| Job status polling | Planned | Phase 4 |
| Download AV1 output | Planned | Phase 4 |
| Save/share output | Planned | Phase 4 |
| Resumable uploads | Planned | Phase 5 |
| Resumable downloads | Planned | Phase 5 |
| Background transfer survival | Planned | Phase 5 |
| Local job history | Planned | Phase 4 |

See [PRD](../../docs/obsidian-vault/05-Product/Missing%20Companion%20Clients%20PRD.md) for full requirements.

## Prerequisites

- Xcode 16+
- iOS 17+ deployment target
- Swift 6+ (or Swift 5.10+)
- Apple Developer account (for device testing)

## Project Structure

```
mobile/ios/
├── Enkodu/
│   ├── EnkoduApp.swift
│   ├── Assets.xcassets/
│   ├── Models/
│   │   ├── Job.swift
│   │   ├── TransferState.swift
│   │   └── AppConfig.swift
│   ├── Services/
│   │   ├── EnkoduAPI.swift
│   │   ├── CapabilityChecker.swift
│   │   ├── TransferManager.swift
│   │   └── NotificationService.swift
│   ├── ViewModels/
│   │   ├── ServerSetupViewModel.swift
│   │   ├── HomeViewModel.swift
│   │   ├── UploadViewModel.swift
│   │   └── QueueViewModel.swift
│   └── Views/
│       ├── ServerSetupView.swift
│       ├── CapabilityView.swift
│       ├── HomeView.swift
│       ├── VideoPickerView.swift
│       ├── UploadView.swift
│       ├── QueueView.swift
│       ├── DownloadView.swift
│       ├── HistoryView.swift
│       └── Components/
│           ├── ProgressView.swift
│           ├── JobRow.swift
│           └── StatusBadge.swift
├── EnkoduTests/
│   ├── CapabilityCheckerTests.swift
│   ├── TransferManagerTests.swift
│   └── APITests.swift
├── EnkoduUITests/
├── Info.plist
├── Enkodu.entitlements
└── README.md (this file)
```

## AV1 Hardware Decode Gate

The app **must not** allow AV1 upgrade on devices without hardware AV1 decode support.

### Implementation

```swift
// In CapabilityChecker.swift
import VideoToolbox

func checkAV1HardwareDecodeSupport() -> Bool {
    // AV1 codec type for VideoToolbox
    // Note: kCMVideoCodecType_AV1 is available on iOS 17+
    
    #if os(iOS)
    if #available(iOS 17.0, *) {
        return VTIsHardwareDecodeSupported(kCMVideoCodecType_AV1)
    } else {
        // iOS versions before 17.0 do not support AV1 hardware decode
        return false
    }
    #elseif os(tvOS)
    // tvOS may have different availability
    if #available(tvOS 17.0, *) {
        return VTIsHardwareDecodeSupported(kCMVideoCodecType_AV1)
    } else {
        return false
    }
    #else
    return false
    #endif
}
```

### Fallback for Older iOS Versions

If the AV1 codec type constant is unavailable at compile time, treat as unsupported:

```swift
// Check if AV1 codec type exists at runtime
func canCheckAV1Support() -> Bool {
    #if os(iOS)
    if #available(iOS 17.0, *) {
        return true
    }
    return false
    #else
    return false
    #endif
}
```

### Behavior Matrix

| AV1 Hardware Decode | Upgrade Video Button | Queue/Status Viewing | Upload for AV1 | Download AV1 |
|---|---|---|---|---|
| Supported | Enabled | Enabled | Allowed | Allowed |
| Unsupported | Disabled | Enabled | **Blocked** | **Blocked** |
| iOS < 17.0 | Disabled | Enabled | **Blocked** | **Blocked** |

### User Messaging

**Unsupported devices see:**
> "This iPhone/iPad cannot play AV1 efficiently. AV1 upgrade is disabled on this device. You can still view queue status, but videos cannot be submitted for AV1 conversion or downloaded as AV1 outputs."

**iOS < 17.0 devices see:**
> "AV1 hardware decode requires iOS 17 or later. AV1 upgrade is disabled on this device."

## Configuration

Server URL and user preferences are stored in `UserDefaults` or AppStorage.

```swift
struct AppConfig: Codable {
    var serverUrl: String
    var userName: String?
    var wifiOnlyUpload: Bool
    var wifiOnlyDownload: Bool
    var maxUploadSizeMB: Int  // Default: 2048 (2GB)
    var batteryPauseThreshold: Int  // Default: 15 (%)
    var batteryResumeThreshold: Int  // Default: 20 (%)
}

class ConfigStore {
    static let shared = ConfigStore()
    
    @AppStorage("enkodu.config")
    private var configData: Data?
    
    var config: AppConfig {
        get {
            guard let data = configData,
                  let decoded = try? JSONDecoder().decode(AppConfig.self, from: data)
            else {
                return AppConfig(
                    serverUrl: "",
                    userName: nil,
                    wifiOnlyUpload: true,
                    wifiOnlyDownload: true,
                    maxUploadSizeMB: 2048,
                    batteryPauseThreshold: 15,
                    batteryResumeThreshold: 20
                )
            }
            return decoded
        }
        set {
            configData = try? JSONEncoder().encode(newValue)
        }
    }
}
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
2. Pick video via PhotosPicker or DocumentPicker
3. Get file size and verify read permission
4. Upload with progress reporting via URLSession
5. On success: receive job_id, start polling
6. On failure: retry with exponential backoff or report error

### Download Flow

1. Wait for job status = "done" and verify_status = "pass"
2. Download AV1 output with progress via URLSession
3. Verify file integrity
4. Save to Files app or Photos library
5. Present share sheet for user to save/export
6. Notify user of completion

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

- URLSession with background configuration for transfers
- BGProcessingTask for large transfer processing
- Transfer state persisted to Core Data or FileManager
- Survives app suspension via background session delegates

```swift
// In AppDelegate.swift or EnkoduApp.swift
class EnkoduApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
        .backgroundTask(.appRefresh("com.enkodu.refresh")) {
            // Handle background refresh
        }
    }
}

// URLSession with background support
let backgroundSessionConfig = URLSessionConfiguration.background(withIdentifier: "com.enkodu.transfer")
backgroundSessionConfig.sessionSendsLaunchEvents = true
backgroundSessionConfig.isDiscretionary = true
let backgroundSession = URLSession(configuration: backgroundSessionConfig, delegate: TransferDelegate(), delegateQueue: nil)
```

## Entitlements

For background transfers and other capabilities:

```xml
<!-- Enkodu.entitlements -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.developer.usbserialnumbers</key>
    <array/>
    <key>com.apple.developer.background-modes</key>
    <array>
        <string>background-fetch</string>
        <string>remote-notification</string>
    </array>
    <key>com.apple.developer-networking.background-transfer</key>
    <true/>
</dict>
</plist>
```

## Info.plist Requirements

```xml
<key>NSPhotoLibraryUsageDescription</key>
<string>Enkodu needs access to your photos to let you select videos for AV1 upgrade.</string>

<key>NSPhotoLibraryAddUsageDescription</key>
<string>Enkodu needs access to save AV1 videos to your Photos library.</string>

<key>NSDocumentsFolderUsageDescription</key>
<string>Enkodu needs access to files to let you select and save videos.</string>

<key>NSAppTransportSecurity</key>
<dict>
    <key>NSAllowsArbitraryLoads</key>
    <false/>
    <key>NSExceptionDomains</key>
    <dict>
        <key>your-server-domain.com</key>
        <dict>
            <key>NSExceptionAllowsInsecureHTTPLoads</key>
            <false/>
            <key>NSIncludesSubdomains</key>
            <true/>
        </dict>
    </dict>
</dict>

<key>UIBackgroundModes</key>
<array>
    <string>fetch</string>
    <string>remote-notification</string>
</array>
```

## Testing

### Manual Tests

- [ ] Unsupported device (iOS < 17.0): verify upgrade button is disabled
- [ ] Unsupported device (iOS 17+ without AV1): verify upgrade button is disabled
- [ ] Supported device: verify fixture video flows end-to-end
- [ ] Network drop < 30s: verify auto-retry
- [ ] Network drop > 30s: verify pause and resume
- [ ] App backgrounded during transfer: verify continuation via background session
- [ ] App killed during transfer: verify state recovery on restart

### Automated Tests (Unit/UI)

```swift
// AV1 decode capability test
import XCTest
import VideoToolbox

class CapabilityCheckerTests: XCTestCase {
    func testAV1SupportDetection() {
        if #available(iOS 17.0, *) {
            let supported = CapabilityChecker.checkAV1HardwareDecodeSupport()
            // Verify result is deterministic for the test device
            XCTAssertNotNil(supported)
        } else {
            // Should return false on older iOS
            XCTAssertFalse(CapabilityChecker.checkAV1HardwareDecodeSupport())
        }
    }
}

// Upload retry test
class TransferManagerTests: XCTestCase {
    func testRetryOnNetworkError() {
        // Mock URLSession to simulate network failure
        // Verify retry with exponential backoff
    }
}
```

## Building

```bash
# Build for simulator
xcodebuild -scheme Enkodu -destination 'platform=iOS Simulator,name=iPhone 15' build

# Build for device (requires code signing)
xcodebuild -scheme Enkodu -configuration Release archive

# Export archive for distribution
xcodebuild -exportArchive -archivePath Enkodu.xcarchive -exportPath ./build -exportOptionsPlist ExportOptions.plist
```

## Recommended Dependencies

Add to your `Package.swift` or via Swift Package Manager in Xcode:

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/Alamofire/Alamofire.git", from: "5.8.0"),
    .package(url: "https://github.com/groue/CombineExpectations.git", from: "4.0.0"),
    .package(url: "https://github.com/pointfreeco/swift-composable-architecture.git", from: "1.8.0"),
    .package(url: "https://github.com/SnapKit/Record.git", from: "1.0.0"),
    .package(url: "https://github.com/dkk/WrappingHStack.git", from: "1.0.0"),
],
targets: [
    .target(
        name: "Enkodu",
        dependencies: [
            .product(name: "Alamofire", package: "Alamofire"),
            .product(name: "ComposableArchitecture", package: "swift-composable-architecture"),
        ]
    ),
]
```

## References

- [PRD: Missing Companion Clients](../../docs/obsidian-vault/05-Product/Missing%20Companion%20Clients%20PRD.md)
- [Mobile Transfer Manager Design](../../docs/obsidian-vault/05-Product/Mobile%20Transfer%20Manager%20Design.md)
- [Apple VTIsHardwareDecodeSupported Documentation](https://developer.apple.com/documentation/videotoolbox/vtishardwaredecodesupported%28_%3A%29)
- [Apple VideoToolbox Overview](https://developer.apple.com/documentation/videotoolbox)
- [Apple Background Transfer Documentation](https://developer.apple.com/documentation/foundation/urlsession/transferring_files_in_the_background)
