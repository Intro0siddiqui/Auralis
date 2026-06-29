package com.auralis.desktop

import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import androidx.compose.material3.MaterialTheme
import com.auralis.ui.screens.MainScreen

fun main() = application {
    Window(
        onCloseRequest = ::exitApplication,
        title = "Auralis",
        state = rememberWindowState()
    ) {
        MaterialTheme {
            MainScreen()
        }
    }
}
