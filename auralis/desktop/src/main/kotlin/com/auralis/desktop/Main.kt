package com.auralis.desktop

import androidx.compose.runtime.remember
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import com.auralis.database.createDatabase
import com.auralis.desktop.theme.DesktopTheme
import com.auralis.ui.screens.MainScreen
import java.io.File

fun main() = application {
    val storageDir = remember {
        File(System.getProperty("user.home"), ".auralis").also { it.mkdirs() }
    }

    val database = remember { createDatabase() }

    Window(
        onCloseRequest = ::exitApplication,
        title = "Auralis",
        state = rememberWindowState()
    ) {
        DesktopTheme {
            MainScreen(
                database = database,
                storageDir = storageDir
            )
        }
    }
}
