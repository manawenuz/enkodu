package com.enkodu.companion

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.ComponentActivity
import androidx.core.content.FileProvider
import java.io.File
import java.io.FileOutputStream

class VideoPicker(private val activity: ComponentActivity) {

    private var onVideoPicked: ((File?) -> Unit)? = null

    private val launcher: ActivityResultLauncher<Intent> = activity.registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            val uri = result.data?.data
            if (uri != null) {
                val file = copyUriToCache(uri)
                onVideoPicked?.invoke(file)
            } else {
                onVideoPicked?.invoke(null)
            }
        } else {
            onVideoPicked?.invoke(null)
        }
    }

    fun pickVideo(onResult: (File?) -> Unit) {
        onVideoPicked = onResult
        val intent = Intent(Intent.ACTION_PICK, MediaStore.Video.Media.EXTERNAL_CONTENT_URI).apply {
            type = "video/*"
        }
        launcher.launch(intent)
    }

    fun pickVideoModern(onResult: (File?) -> Unit) {
        onVideoPicked = onResult
        val intent = Intent(MediaStore.ACTION_PICK_IMAGES).apply {
            type = "video/*"
        }
        launcher.launch(intent)
    }

    private fun copyUriToCache(uri: Uri): File? {
        return try {
            val inputStream = activity.contentResolver.openInputStream(uri) ?: return null
            val fileName = "picked_${System.currentTimeMillis()}.mp4"
            val outFile = File(activity.cacheDir, fileName)
            FileOutputStream(outFile).use { output ->
                inputStream.copyTo(output)
            }
            inputStream.close()
            outFile
        } catch (e: Exception) {
            null
        }
    }
}
