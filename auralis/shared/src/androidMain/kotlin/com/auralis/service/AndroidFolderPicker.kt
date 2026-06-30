package com.auralis.service

import android.app.Activity
import android.content.Intent
import android.net.Uri
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlin.coroutines.resume

actual class FolderPicker {
    private var pendingResult: ((String?) -> Unit)? = null

    actual suspend fun pickFolder(): String? = withContext(Dispatchers.IO) {
        suspendCancellableCoroutine { continuation ->
            pendingResult = { path ->
                continuation.resume(path)
            }
        }
    }

    fun handleResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == FOLDER_PICKER_REQUEST && resultCode == Activity.RESULT_OK) {
            val uri: Uri? = data?.data
            pendingResult?.invoke(uri?.path)
        } else {
            pendingResult?.invoke(null)
        }
        pendingResult = null
    }

    companion object {
        const val FOLDER_PICKER_REQUEST = 1001
    }
}
