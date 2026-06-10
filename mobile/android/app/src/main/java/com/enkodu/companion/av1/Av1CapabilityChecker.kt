package com.enkodu.companion.av1

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.util.Log

object Av1CapabilityChecker {

    private const val TAG = "Av1Capability"
    private const val AV1_MIME = "video/av01"
    private const val AV1_MIME_ALT = "video/av1"

    data class Result(
        val supported: Boolean,
        val decoderName: String? = null,
        val isHardware: Boolean = false,
        val reason: String = ""
    )

    fun check(): Result {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            // MediaCodecInfo.isHardwareAccelerated() requires API 29+
            return Result(
                supported = false,
                reason = "Requires Android 10+ for hardware acceleration detection"
            )
        }

        val codecList = MediaCodecList(MediaCodecList.REGULAR_CODECS)
        for (codecInfo in codecList.codecInfos) {
            if (codecInfo.isEncoder) continue

            val supportedTypes = codecInfo.supportedTypes
            val hasAv1 = supportedTypes.contains(AV1_MIME) ||
                    supportedTypes.contains(AV1_MIME_ALT) ||
                    supportedTypes.contains(MediaFormat.MIMETYPE_VIDEO_AV1)

            if (!hasAv1) continue

            val isHardware = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                codecInfo.isHardwareAccelerated
            } else false

            val isSoftwareOnly = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                codecInfo.isSoftwareOnly
            } else true

            Log.i(TAG, "Found AV1 decoder: ${codecInfo.name} hardware=$isHardware software=$isSoftwareOnly")

            if (isHardware && !isSoftwareOnly) {
                return Result(
                    supported = true,
                    decoderName = codecInfo.name,
                    isHardware = true,
                    reason = "Hardware decoder: ${codecInfo.name}"
                )
            }
        }

        return Result(
            supported = false,
            reason = "No hardware AV1 decoder found on this device"
        )
    }

    fun isSupported(): Boolean = check().supported
}
