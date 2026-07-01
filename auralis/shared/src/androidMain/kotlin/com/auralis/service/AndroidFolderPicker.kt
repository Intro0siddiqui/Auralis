package com.auralis.service

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
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
            launchPicker()
        }
    }

    private fun launchPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        }
        pendingIntent?.invoke(intent)
    }

    fun handleResult(resultCode: Int, data: Intent?) {
        if (resultCode == Activity.RESULT_OK) {
            val treeUri: Uri = data?.data ?: run {
                pendingResult?.invoke(null)
                pendingResult = null
                return
            }
            val path = treeUriToPath(treeUri)
            pendingResult?.invoke(path)
        } else {
            pendingResult?.invoke(null)
        }
        pendingResult = null
    }

    private fun treeUriToPath(treeUri: Uri): String? {
        val docId = DocumentsContract.getTreeDocumentId(treeUri)
        if (docId == "primary:Android") return null

        if (docId.startsWith("primary:")) {
            return "/storage/emulated/0/${docId.removePrefix("primary:")}"
        }
        if (docId.contains(":")) {
            val parts = docId.split(":")
            if (parts.size >= 2) {
                return "/storage/${parts[0]}/${parts[1]}"
            }
        }
        return null
    }

    var pendingIntent: ((Intent) -> Unit)? = null

    companion object {
        const val FOLDER_PICKER_REQUEST = 1001
    }
}
