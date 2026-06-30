package com.auralis.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.remember
import com.auralis.database.createDatabase
import com.auralis.ui.screens.MainScreen
import java.io.File

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val storageDir = File(filesDir, ".auralis").apply { mkdirs() }
        val database = createDatabase(applicationContext)

        setContent {
            MaterialTheme {
                MainScreen(
                    database = database,
                    storageDir = storageDir
                )
            }
        }
    }
}
