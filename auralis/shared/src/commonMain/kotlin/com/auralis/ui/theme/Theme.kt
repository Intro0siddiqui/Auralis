package com.auralis.ui.theme

import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

val DefaultPrimaryLight = Color(0xFF6750A4)
val DefaultPrimaryDark = Color(0xFFD0BCFF)

val TealPrimary = Color(0xFF00BFA5)
val PurplePrimary = Color(0xFFBB86FC)

fun auralisLightColorScheme(primary: Color = DefaultPrimaryLight) = lightColorScheme(
    primary = primary,
    onPrimary = Color.White,
    primaryContainer = primary.copy(alpha = 0.12f),
    onPrimaryContainer = primary.copy(alpha = 0.9f),
    secondary = Color(0xFF625B71),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFE8DEF8),
    onSecondaryContainer = Color(0xFF1D192B),
    tertiary = Color(0xFF7D5260),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFFFFD8E4),
    onTertiaryContainer = Color(0xFF31111D),
    background = Color(0xFFFFFBFE),
    onBackground = Color(0xFF1C1B1F),
    surface = Color(0xFFFFFBFE),
    onSurface = Color(0xFF1C1B1F),
    surfaceVariant = Color(0xFFE7E0EC),
    onSurfaceVariant = Color(0xFF49454F),
    outline = Color(0xFF79747E),
    error = Color(0xFFB3261E),
    onError = Color.White,
    errorContainer = Color(0xFFF9DEDC),
    onErrorContainer = Color(0xFF410E0B),
    surfaceTint = primary,
)

fun auralisDarkColorScheme(primary: Color = DefaultPrimaryDark) = darkColorScheme(
    primary = primary,
    onPrimary = Color(0xFF381E72),
    primaryContainer = primary.copy(alpha = 0.12f),
    onPrimaryContainer = primary,
    secondary = Color(0xFFCCC2DC),
    onSecondary = Color(0xFF332D41),
    secondaryContainer = Color(0xFF4A4458),
    onSecondaryContainer = Color(0xFFE8DEF8),
    tertiary = Color(0xFFEFB8C8),
    onTertiary = Color(0xFF492532),
    tertiaryContainer = Color(0xFF633B48),
    onTertiaryContainer = Color(0xFFFFD8E4),
    background = Color(0xFF1C1B1F),
    onBackground = Color(0xFFE6E1E5),
    surface = Color(0xFF1C1B1F),
    onSurface = Color(0xFFE6E1E5),
    surfaceVariant = Color(0xFF49454F),
    onSurfaceVariant = Color(0xFFCAC4D0),
    outline = Color(0xFF938F99),
    error = Color(0xFFF2B8B5),
    onError = Color(0xFF601410),
    errorContainer = Color(0xFF8C1D18),
    onErrorContainer = Color(0xFFF9DEDC),
    surfaceTint = primary,
)

@Composable
fun AuralisTheme(
    isDark: Boolean = false,
    accentColor: Color? = null,
    content: @Composable () -> Unit
) {
    val primary = accentColor ?: if (isDark) DefaultPrimaryDark else DefaultPrimaryLight
    val colorScheme = if (isDark) auralisDarkColorScheme(primary) else auralisLightColorScheme(primary)

    MaterialTheme(
        colorScheme = colorScheme,
        content = content
    )
}
