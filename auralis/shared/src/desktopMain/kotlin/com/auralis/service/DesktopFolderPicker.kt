package com.auralis.service

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import javax.swing.JFileChooser
import javax.swing.filechooser.FileSystemView

actual class FolderPicker {
    actual suspend fun pickFolder(): String? = withContext(Dispatchers.IO) {
        try {
            val chooser = JFileChooser().apply {
                fileSelectionMode = JFileChooser.DIRECTORIES_ONLY
                dialogTitle = "Select Music Folder"
                currentDirectory = FileSystemView.getFileSystemView().defaultDirectory
            }
            val result = chooser.showOpenDialog(null)
            if (result == JFileChooser.APPROVE_OPTION) {
                chooser.selectedFile.absolutePath
            } else {
                null
            }
        } catch (e: Exception) {
            null
        }
    }
}
