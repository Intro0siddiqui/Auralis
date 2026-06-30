package com.auralis.desktop.theme

import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import com.auralis.ui.theme.AuralisTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import javax.swing.UIManager

@Composable
fun DesktopTheme(content: @Composable () -> Unit) {
    val systemDark = isSystemInDarkTheme()
    var isDark by remember { mutableStateOf(systemDark) }
    var accentColor by remember { mutableStateOf<Color?>(null) }

    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            isDark = detectDarkMode()
            accentColor = detectAccentColor()
        }
    }

    val gradientColors = if (isDark) {
        listOf(
            Color(0xFF0F0C29).copy(alpha = 0.95f),
            Color(0xFF302B63).copy(alpha = 0.85f),
            Color(0xFF24243E).copy(alpha = 0.9f)
        )
    } else {
        listOf(
            Color(0xFFF5F7FA).copy(alpha = 0.95f),
            Color(0xFFE4E9F2).copy(alpha = 0.85f),
            Color(0xFFF0F4F8).copy(alpha = 0.9f)
        )
    }

    val accent = accentColor ?: if (isDark) Color(0xFFD0BCFF) else Color(0xFF6750A4)

    AuralisTheme(
        isDark = isDark,
        accentColor = accent
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(
                    Brush.verticalGradient(
                        colors = gradientColors
                    )
                )
        ) {
            content()
        }
    }
}

private fun detectDarkMode(): Boolean {
    val gsettings = try {
        val result = ProcessBuilder("gsettings", "get", "org.gnome.desktop.interface", "color-scheme")
            .redirectErrorStream(true)
            .start()
            .inputStream.bufferedReader().readText().trim()
        result.contains("prefer-dark") || result.contains("dark")
    } catch (_: Exception) { false }
    if (gsettings) return true

    val gtkTheme = try {
        val result = ProcessBuilder("gsettings", "get", "org.gnome.desktop.interface", "gtk-theme")
            .redirectErrorStream(true)
            .start()
            .inputStream.bufferedReader().readText().trim()
        result.lowercase().contains("dark")
    } catch (_: Exception) { false }
    if (gtkTheme) return true

    val gtkThemeVar = System.getenv("GTK_THEME")
    if (gtkThemeVar != null && gtkThemeVar.lowercase().contains("dark")) return true

    val colorScheme = try {
        val result = ProcessBuilder("dbus-send", "--session", "--type=method_call", "--dest=org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings.Read",
            "string:org.freedesktop.appearance", "string:color-scheme")
            .redirectErrorStream(true)
            .start()
            .inputStream.bufferedReader().readText()
        result.contains("uint32 1")
    } catch (_: Exception) { false }
    if (colorScheme) return true

    val kdeScheme = try {
        val home = System.getProperty("user.home") ?: ""
        val configFile = File("$home/.config/kdeglobals")
        if (configFile.exists()) {
            val content = configFile.readText()
            content.contains("ColorScheme") && content.contains("Dark")
        } else false
    } catch (_: Exception) { false }
    if (kdeScheme) return true

    try {
        val lafName = UIManager.getLookAndFeel().name.lowercase()
        if (lafName.contains("dark")) return true
        val bg = UIManager.getColor("Panel.background")
        if (bg != null) {
            val brightness = (bg.red * 0.299 + bg.green * 0.587 + bg.blue * 0.114) / 255.0
            if (brightness < 0.4) return true
        }
    } catch (_: Exception) {}

    return false
}

private fun detectAccentColor(): Color? {
    return tryGnomeAccent()
        ?: tryKdeAccent()
        ?: tryGtkAccent()
        ?: null
}

private fun tryGnomeAccent(): Color? {
    val result = try {
        ProcessBuilder("gsettings", "get", "org.gnome.desktop.interface", "accent-color")
            .redirectErrorStream(true)
            .start()
            .inputStream.bufferedReader().readText().trim()
    } catch (_: Exception) { return null }

    if (result == "nothing" || result == "'nothing'") return null

    val name = result.trim('\'').lowercase()
    return when (name) {
        "blue" -> Color(0xFF3584E4)
        "teal" -> Color(0xFF2190A4)
        "green" -> Color(0xFF3A944A)
        "yellow" -> Color(0xFFC88800)
        "orange" -> Color(0xFFED5B00)
        "red" -> Color(0xFFE62D42)
        "pink" -> Color(0xFFD56199)
        "purple" -> Color(0xFF9141AC)
        "slate" -> Color(0xFF6F8396)
        else -> null
    }
}

private fun tryKdeAccent(): Color? {
    val home = System.getProperty("user.home") ?: return null
    val configFile = File("$home/.config/kdeglobals")

    val content = try {
        if (configFile.exists()) configFile.readText() else return null
    } catch (_: Exception) { return null }

    val activeEffect = Regex("""\[Colors:Selection\]\s*\nActiveForeground=(#[0-9A-Fa-f]{6})""", RegexOption.MULTILINE)
        .find(content)?.groupValues?.get(1) ?: return null

    return parseHexColor(activeEffect)
}

private fun tryGtkAccent(): Color? {
    val cssFile = File(System.getProperty("user.home") + "/.config/gtk-4.0/gtk.css")
    val cssFile3 = File(System.getProperty("user.home") + "/.config/gtk-3.0/gtk.css")

    for (file in listOf(cssFile, cssFile3)) {
        if (!file.exists()) continue
        val css = try { file.readText() } catch (_: Exception) { continue }
        val match = Regex("""@define-color accent_color\s+#([0-9A-Fa-f]{6})""").find(css)
        if (match != null) return parseHexColor("#${match.groupValues[1]}")
    }
    return null
}

private fun parseHexColor(hex: String): Color? {
    val cleaned = hex.removePrefix("#")
    if (cleaned.length != 6) return null
    return try {
        val r = cleaned.substring(0, 2).toInt(16)
        val g = cleaned.substring(2, 4).toInt(16)
        val b = cleaned.substring(4, 6).toInt(16)
        Color(r, g, b)
    } catch (_: Exception) { null }
}
