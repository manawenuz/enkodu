import Foundation
import VideoToolbox

struct Av1CapabilityResult {
    let supported: Bool
    let reason: String
}

enum Av1CapabilityChecker {
    static func check() async -> Av1CapabilityResult {
        // Check if VTIsHardwareDecodeSupported is available (iOS 16.0+)
        if #available(iOS 16.0, *) {
            let av1Supported = VTIsHardwareDecodeSupported(kCMVideoCodecType_AV1)
            if av1Supported {
                return Av1CapabilityResult(
                    supported: true,
                    reason: "Hardware AV1 decode supported via VideoToolbox"
                )
            } else {
                return Av1CapabilityResult(
                    supported: false,
                    reason: "VTIsHardwareDecodeSupported returned false for AV1"
                )
            }
        } else {
            return Av1CapabilityResult(
                supported: false,
                reason: "Requires iOS 16+ for AV1 hardware decode detection"
            )
        }
    }
}

// CMVideoCodecType for AV1 (kCMVideoCodecType_AV1 = 'av01')
private let kCMVideoCodecType_AV1: CMVideoCodecType = 0x61763031
