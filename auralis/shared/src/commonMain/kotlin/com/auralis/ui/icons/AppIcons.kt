package com.auralis.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathBuilder
import androidx.compose.ui.graphics.vector.group
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

private fun icon(pathBuilder: PathBuilder.() -> Unit): ImageVector {
    return ImageVector.Builder(
        name = "AppIcon",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f
    ).apply {
        group {
            path(fill = SolidColor(Color.Black), pathBuilder = pathBuilder)
        }
    }.build()
}

val PauseIcon by lazy {
    icon {
        moveTo(6f, 19f); horizontalLineTo(4f); verticalLineTo(5f); horizontalLineTo(6f); close()
        moveTo(19f, 5f); horizontalLineTo(8f); verticalLineTo(19f); horizontalLineTo(19f); close()
    }
}

val SkipPreviousIcon by lazy {
    icon {
        moveTo(6f, 6f); horizontalLineTo(8f); verticalLineTo(18f); horizontalLineTo(6f); close()
        moveTo(9.5f, 12f); lineTo(18f, 18f); verticalLineTo(6f); close()
    }
}

val SkipNextIcon by lazy {
    icon {
        moveTo(6f, 18f); lineTo(14.5f, 12f); lineTo(6f, 6f); close()
        moveTo(16f, 6f); horizontalLineTo(18f); verticalLineTo(18f); horizontalLineTo(16f); close()
    }
}

val DownloadIcon by lazy {
    icon {
        moveTo(19f, 9f); horizontalLineTo(14f); verticalLineTo(5f); horizontalLineTo(10f)
        verticalLineTo(9f); horizontalLineTo(19f); close()
        moveTo(5f, 18f); horizontalLineTo(19f); verticalLineTo(20f); horizontalLineTo(5f); verticalLineTo(18f); close()
        moveTo(12f, 15.5f); lineTo(7.5f, 11f); lineTo(9f, 9.5f); lineTo(12f, 12.5f)
        lineTo(15f, 9.5f); lineTo(16.5f, 11f); close()
    }
}

val LibraryMusicIcon by lazy {
    icon {
        moveTo(20f, 2f); horizontalLineTo(4f); verticalLineTo(12f); horizontalLineTo(20f); close()
        moveTo(4f, 22f); horizontalLineTo(20f); verticalLineTo(14f); horizontalLineTo(4f); close()
    }
}

val LinkIcon by lazy {
    icon {
        moveTo(3.9f, 12f)
        curveTo(3.9f, 10.18f, 5.4f, 8.68f, 7.22f, 8.68f)
        lineTo(9.04f, 8.68f); verticalLineTo(10.5f)
        curveTo(7.64f, 10.5f, 7.22f, 10.92f, 7.22f, 10.92f)
        curveTo(6.8f, 11.34f, 6.8f, 12.66f, 7.22f, 13.08f)
        curveTo(7.64f, 13.5f, 9.04f, 13.5f, 9.04f, 13.5f)
        lineTo(10.86f, 13.5f); verticalLineTo(15.32f)
        curveTo(12.26f, 15.32f, 12.68f, 14.9f, 12.68f, 14.9f)
        curveTo(13.1f, 14.48f, 13.1f, 13.16f, 12.68f, 12.74f)
        curveTo(12.26f, 12.32f, 10.86f, 12.32f, 10.86f, 12.32f)
        lineTo(9.04f, 12.32f); verticalLineTo(10.5f); lineTo(7.22f, 10.5f)
        curveTo(5.4f, 10.5f, 3.9f, 12f, 3.9f, 12f); close()
        moveTo(14.78f, 12f)
        curveTo(14.78f, 13.82f, 13.28f, 15.32f, 11.46f, 15.32f)
        lineTo(9.64f, 15.32f); verticalLineTo(13.5f)
        curveTo(11.04f, 13.5f, 11.46f, 13.08f, 11.46f, 13.08f)
        curveTo(11.88f, 12.66f, 11.88f, 11.34f, 11.46f, 10.92f)
        curveTo(11.04f, 10.5f, 9.64f, 10.5f, 9.64f, 10.5f)
        lineTo(7.82f, 10.5f); verticalLineTo(8.68f)
        curveTo(6.42f, 8.68f, 6f, 9.1f, 6f, 9.1f)
        curveTo(5.58f, 9.52f, 5.58f, 10.84f, 6f, 11.26f)
        curveTo(6.42f, 11.68f, 7.82f, 11.68f, 7.82f, 11.68f)
        lineTo(9.64f, 11.68f); verticalLineTo(13.5f); lineTo(11.46f, 13.5f)
        curveTo(13.28f, 13.5f, 14.78f, 12f, 14.78f, 12f); close()
    }
}

val MusicNoteIcon by lazy {
    icon {
        moveTo(12f, 3f); verticalLineTo(13.55f)
        curveTo(11.24f, 12.99f, 10.28f, 12.7f, 9.25f, 12.76f)
        curveTo(7.09f, 12.89f, 5.35f, 14.63f, 5.22f, 16.79f)
        curveTo(5.09f, 18.95f, 6.83f, 20.69f, 8.99f, 20.82f)
        curveTo(11.15f, 20.95f, 12.89f, 19.21f, 13.02f, 17.05f)
        verticalLineTo(7.46f); lineTo(18f, 5.56f); verticalLineTo(12.56f)
        curveTo(17.24f, 12f, 16.28f, 11.71f, 15.25f, 11.76f)
        curveTo(13.09f, 11.89f, 11.35f, 13.63f, 11.22f, 15.79f)
        curveTo(11.09f, 17.95f, 12.83f, 19.69f, 14.99f, 19.82f)
        curveTo(17.15f, 19.95f, 18.89f, 18.21f, 19.02f, 16.05f)
        verticalLineTo(3f); close()
    }
}

val QueueMusicIcon by lazy {
    icon {
        moveTo(15f, 6f); verticalLineTo(4f); horizontalLineTo(3f); verticalLineTo(20f)
        horizontalLineTo(15f); verticalLineTo(18f); horizontalLineTo(5f); verticalLineTo(6f); close()
        moveTo(15f, 12f); verticalLineTo(10f); horizontalLineTo(21f); verticalLineTo(12f); close()
        moveTo(18.5f, 15.5f); lineTo(21f, 13f); verticalLineTo(18f); close()
    }
}

val ErrorCircleIcon by lazy {
    icon {
        moveTo(12f, 2f)
        curveTo(6.48f, 2f, 2f, 6.48f, 2f, 12f)
        curveTo(2f, 17.52f, 6.48f, 22f, 12f, 22f)
        curveTo(17.52f, 22f, 22f, 17.52f, 22f, 12f)
        curveTo(22f, 6.48f, 17.52f, 2f, 12f, 2f); close()
        moveTo(13f, 17f); horizontalLineTo(11f); verticalLineTo(11f); horizontalLineTo(13f); close()
        moveTo(13f, 9f); horizontalLineTo(11f); verticalLineTo(7f); horizontalLineTo(13f); close()
    }
}

val PlayArrowIcon by lazy {
    icon {
        moveTo(8f, 5f); verticalLineTo(19f); lineTo(19f, 12f); close()
    }
}

val SearchIcon by lazy {
    icon {
        moveTo(15.5f, 14f)
        horizontalLineTo(14.71f)
        curveTo(14.88f, 13.38f, 14.93f, 12.71f, 14.93f, 12f)
        curveTo(14.93f, 9.24f, 12.69f, 7f, 9.93f, 7f)
        curveTo(7.17f, 7f, 4.93f, 9.24f, 4.93f, 12f)
        curveTo(4.93f, 14.76f, 7.17f, 17f, 9.93f, 17f)
        curveTo(11.19f, 17f, 12.34f, 16.52f, 13.22f, 15.72f)
        lineTo(14f, 16.51f)
        lineTo(14f, 17f)
        lineTo(19f, 22f)
        lineTo(20.5f, 20.5f)
        lineTo(15.5f, 14f)
        close()
        moveTo(9.93f, 15f)
        curveTo(8.27f, 15f, 6.93f, 13.66f, 6.93f, 12f)
        curveTo(6.93f, 10.34f, 8.27f, 9f, 9.93f, 9f)
        curveTo(11.59f, 9f, 12.93f, 10.34f, 12.93f, 12f)
        curveTo(12.93f, 13.66f, 11.59f, 15f, 9.93f, 15f)
        close()
    }
}

val CheckCircleIcon by lazy {
    icon {
        moveTo(12f, 2f)
        curveTo(6.48f, 2f, 2f, 6.48f, 2f, 12f)
        curveTo(2f, 17.52f, 6.48f, 22f, 12f, 22f)
        curveTo(17.52f, 22f, 22f, 17.52f, 22f, 12f)
        curveTo(22f, 6.48f, 17.52f, 2f, 12f, 2f)
        close()
        moveTo(10f, 17f)
        lineTo(5f, 12f)
        lineTo(6.41f, 10.59f)
        lineTo(10f, 14.17f)
        lineTo(17.59f, 6.58f)
        lineTo(19f, 8f)
        close()
    }
}

val ClearIcon by lazy {
    icon {
        moveTo(19f, 6.41f)
        lineTo(17.59f, 5f)
        lineTo(12f, 10.59f)
        lineTo(6.41f, 5f)
        lineTo(5f, 6.41f)
        lineTo(10.59f, 12f)
        lineTo(5f, 17.59f)
        lineTo(6.41f, 19f)
        lineTo(12f, 13.41f)
        lineTo(17.59f, 19f)
        lineTo(19f, 17.59f)
        lineTo(13.41f, 12f)
        close()
    }
}

val SyncIcon by lazy {
    icon {
        moveTo(12f, 4f)
        curveTo(7.58f, 4f, 4f, 7.58f, 4f, 12f)
        horizontalLineTo(1f)
        lineTo(5f, 16f)
        lineTo(9f, 12f)
        horizontalLineTo(6f)
        curveTo(6f, 8.69f, 8.69f, 6f, 12f, 6f)
        curveTo(15.31f, 6f, 18f, 8.69f, 18f, 12f)
        curveTo(18f, 15.31f, 15.31f, 18f, 12f, 18f)
        curveTo(10.47f, 18f, 9.09f, 17.43f, 8.01f, 16.51f)
        lineTo(6.6f, 17.92f)
        curveTo(7.98f, 19.23f, 9.89f, 20f, 12f, 20f)
        curveTo(16.42f, 20f, 20f, 16.42f, 20f, 12f)
        curveTo(20f, 7.58f, 16.42f, 4f, 12f, 4f)
        close()
    }
}

val PersonAddIcon by lazy {
    icon {
        moveTo(15f, 12f)
        curveTo(17.21f, 12f, 19f, 10.21f, 19f, 8f)
        curveTo(19f, 5.79f, 17.21f, 4f, 15f, 4f)
        curveTo(12.79f, 4f, 11f, 5.79f, 11f, 8f)
        curveTo(11f, 10.21f, 12.79f, 12f, 15f, 12f)
        close()
        moveTo(6f, 10f)
        verticalLineTo(8f)
        horizontalLineTo(4f)
        verticalLineTo(10f)
        horizontalLineTo(2f)
        verticalLineTo(12f)
        horizontalLineTo(4f)
        verticalLineTo(14f)
        horizontalLineTo(6f)
        verticalLineTo(12f)
        horizontalLineTo(8f)
        verticalLineTo(10f)
        horizontalLineTo(6f)
        close()
        moveTo(15f, 14f)
        curveTo(12.33f, 14f, 7f, 15.34f, 7f, 18f)
        verticalLineTo(20f)
        horizontalLineTo(23f)
        verticalLineTo(18f)
        curveTo(23f, 15.34f, 17.67f, 14f, 15f, 14f)
        close()
    }
}

val QrCodeIcon by lazy {
    icon {
        moveTo(3f, 3f)
        verticalLineTo(9f)
        horizontalLineTo(9f)
        verticalLineTo(3f)
        horizontalLineTo(3f)
        close()
        moveTo(5f, 7f)
        verticalLineTo(5f)
        horizontalLineTo(7f)
        verticalLineTo(7f)
        horizontalLineTo(5f)
        close()
        moveTo(15f, 3f)
        verticalLineTo(9f)
        horizontalLineTo(21f)
        verticalLineTo(3f)
        horizontalLineTo(15f)
        close()
        moveTo(17f, 7f)
        verticalLineTo(5f)
        horizontalLineTo(19f)
        verticalLineTo(7f)
        horizontalLineTo(17f)
        close()
        moveTo(3f, 15f)
        verticalLineTo(21f)
        horizontalLineTo(9f)
        verticalLineTo(15f)
        horizontalLineTo(3f)
        close()
        moveTo(5f, 19f)
        verticalLineTo(17f)
        horizontalLineTo(7f)
        verticalLineTo(19f)
        horizontalLineTo(5f)
        close()
        moveTo(15f, 15f)
        verticalLineTo(17f)
        horizontalLineTo(17f)
        verticalLineTo(15f)
        horizontalLineTo(15f)
        close()
        moveTo(19f, 15f)
        verticalLineTo(17f)
        horizontalLineTo(21f)
        verticalLineTo(19f)
        horizontalLineTo(19f)
        verticalLineTo(21f)
        horizontalLineTo(17f)
        verticalLineTo(19f)
        horizontalLineTo(19f)
        verticalLineTo(15f)
        close()
        moveTo(15f, 19f)
        verticalLineTo(21f)
        horizontalLineTo(17f)
        verticalLineTo(19f)
        horizontalLineTo(15f)
        close()
    }
}

val WifiIcon by lazy {
    icon {
        moveTo(1f, 9f)
        lineTo(3f, 9f)
        curveTo(7.97f, 4.03f, 16.03f, 4.03f, 21f, 9f)
        lineTo(23f, 9f)
        curveTo(17.05f, 3.05f, 6.95f, 3.05f, 1f, 9f)
        close()
        moveTo(5f, 13f)
        lineTo(7f, 13f)
        curveTo(9.76f, 10.24f, 14.24f, 10.24f, 17f, 13f)
        lineTo(19f, 13f)
        curveTo(15.14f, 9.14f, 8.87f, 9.14f, 5f, 13f)
        close()
        moveTo(9f, 17f)
        lineTo(11f, 17f)
        curveTo(11.55f, 16.45f, 12.45f, 16.45f, 13f, 17f)
        lineTo(15f, 17f)
        curveTo(13.87f, 15.87f, 10.12f, 15.87f, 9f, 17f)
        close()
    }
}

val FolderOpenIcon by lazy {
    icon {
        moveTo(20f, 6f)
        verticalLineTo(18f)
        curveTo(20f, 19.1f, 19.1f, 20f, 18f, 20f)
        horizontalLineTo(6f)
        curveTo(4.9f, 20f, 4f, 19.1f, 4f, 18f)
        verticalLineTo(8f)
        curveTo(4f, 6.9f, 4.9f, 6f, 6f, 6f)
        horizontalLineTo(9.17f)
        lineTo(11f, 4f)
        horizontalLineTo(18f)
        curveTo(19.1f, 4f, 20f, 4.9f, 20f, 6f)
        close()
    }
}

val ScanIcon by lazy {
    icon {
        moveTo(12f, 4f)
        curveTo(7.58f, 4f, 4f, 7.58f, 4f, 12f)
        curveTo(4f, 16.42f, 7.58f, 20f, 12f, 20f)
        curveTo(16.42f, 20f, 20f, 16.42f, 20f, 12f)
        curveTo(20f, 7.58f, 16.42f, 4f, 12f, 4f)
        close()
        moveTo(12f, 18f)
        curveTo(8.69f, 18f, 6f, 15.31f, 6f, 12f)
        curveTo(6f, 8.69f, 8.69f, 6f, 12f, 6f)
        curveTo(15.31f, 6f, 18f, 8.69f, 18f, 12f)
        curveTo(18f, 15.31f, 15.31f, 18f, 12f, 18f)
        close()
        moveTo(12f, 8f)
        curveTo(9.79f, 8f, 8f, 9.79f, 8f, 12f)
        curveTo(8f, 14.21f, 9.79f, 16f, 12f, 16f)
        curveTo(14.21f, 16f, 16f, 14.21f, 16f, 12f)
        curveTo(16f, 9.79f, 14.21f, 8f, 12f, 8f)
        close()
        moveTo(12f, 10f)
        curveTo(13.1f, 10f, 14f, 10.9f, 14f, 12f)
        curveTo(14f, 13.1f, 13.1f, 14f, 12f, 14f)
        curveTo(10.9f, 14f, 10f, 13.1f, 10f, 12f)
        curveTo(10f, 10.9f, 10.9f, 10f, 12f, 10f)
        close()
    }
}

val SettingsIcon by lazy {
    icon {
        moveTo(19.14f, 12.94f)
        curveTo(19.18f, 12.64f, 19.2f, 12.33f, 19.2f, 12f)
        curveTo(19.2f, 11.68f, 19.18f, 11.36f, 19.13f, 11.06f)
        lineTo(21.16f, 9.48f)
        lineTo(21.16f, 6.56f)
        lineTo(19.54f, 5.62f)
        curveTo(19.54f, 5.62f, 18.18f, 6.29f, 17.39f, 6.71f)
        lineTo(16.53f, 5.23f)
        lineTo(13.22f, 3.74f)
        lineTo(12.62f, 5.71f)
        curveTo(11.82f, 5.51f, 10.99f, 5.39f, 10.15f, 5.37f)
        lineTo(9.49f, 3.4f)
        lineTo(6.56f, 3.4f)
        lineTo(5.62f, 5.02f)
        curveTo(4.69f, 5.59f, 3.92f, 6.35f, 3.34f, 7.23f)
        lineTo(5.27f, 8.81f)
        lineTo(5.27f, 11.73f)
        lineTo(3.34f, 13.31f)
        curveTo(3.92f, 14.19f, 4.69f, 14.95f, 5.62f, 15.52f)
        lineTo(6.56f, 17.14f)
        lineTo(9.49f, 17.14f)
        lineTo(10.15f, 15.17f)
        curveTo(10.99f, 15.19f, 11.82f, 15.31f, 12.62f, 15.51f)
        lineTo(13.22f, 17.48f)
        lineTo(16.53f, 15.99f)
        lineTo(17.39f, 14.51f)
        curveTo(18.18f, 14.93f, 19.54f, 15.6f, 19.54f, 15.6f)
        lineTo(21.16f, 14.66f)
        lineTo(21.16f, 11.74f)
        close()
        moveTo(12.05f, 15.6f)
        curveTo(9.86f, 15.6f, 8.09f, 13.83f, 8.09f, 11.64f)
        curveTo(8.09f, 9.45f, 9.86f, 7.68f, 12.05f, 7.68f)
        curveTo(14.24f, 7.68f, 16.01f, 9.45f, 16.01f, 11.64f)
        curveTo(16.01f, 13.83f, 14.24f, 15.6f, 12.05f, 15.6f)
        close()
    }
}
