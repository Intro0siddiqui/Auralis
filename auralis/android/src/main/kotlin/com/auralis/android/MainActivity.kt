package com.auralis.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import com.auralis.database.createDatabase
import com.auralis.service.FolderPicker
import com.auralis.ui.screens.MainScreen
import java.io.File

class MainActivity : ComponentActivity() {

    val folderPicker = FolderPicker()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val storageDir = File(filesDir, ".auralis").apply { mkdirs() }
        val database = createDatabase(applicationContext)

        folderPicker.pendingIntent = { intent ->
            @Suppress("DEPRECATION")
            startActivityForResult(intent, FolderPicker.FOLDER_PICKER_REQUEST)
        }

        setContent {
            MaterialTheme {
                MainScreen(
                    database = database,
                    storageDir = storageDir,
                    folderPicker = folderPicker
                )
            }
        }
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == FolderPicker.FOLDER_PICKER_REQUEST) {
            folderPicker.handleResult(resultCode, data)
        }
    }
}
