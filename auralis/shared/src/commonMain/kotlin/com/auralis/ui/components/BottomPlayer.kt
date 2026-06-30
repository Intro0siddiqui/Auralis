package com.auralis.ui.components

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.auralis.ui.icons.PauseIcon
import com.auralis.ui.icons.PlayArrowIcon
import com.auralis.ui.icons.SkipNextIcon
import com.auralis.ui.icons.SkipPreviousIcon

@Composable
fun BottomPlayer(
    title: String = "No track playing",
    artist: String? = null,
    isPlaying: Boolean = false,
    progress: Float = 0f,
    onPlayPause: () -> Unit = {},
    onNext: () -> Unit = {},
    onPrevious: () -> Unit = {}
) {
    Surface(
        tonalElevation = 3.dp
    ) {
        Column {
            LinearProgressIndicator(
                progress = { progress },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(2.dp)
            )
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = title,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    if (artist != null) {
                        Text(
                            text = artist,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
                Row {
                    IconButton(onClick = onPrevious) {
                        Icon(SkipPreviousIcon, contentDescription = "Previous")
                    }
                    IconButton(onClick = onPlayPause) {
                        Icon(
                            if (isPlaying) PauseIcon else PlayArrowIcon,
                            contentDescription = if (isPlaying) "Pause" else "Play"
                        )
                    }
                    IconButton(onClick = onNext) {
                        Icon(SkipNextIcon, contentDescription = "Next")
                    }
                }
            }
        }
    }
}
