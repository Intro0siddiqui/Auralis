package com.auralis.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.auralis.service.DownloadResult
import com.auralis.ui.icons.CheckCircleIcon
import com.auralis.ui.icons.ClearIcon
import com.auralis.ui.icons.DownloadIcon
import com.auralis.ui.icons.ErrorCircleIcon
import com.auralis.ui.icons.LinkIcon

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadScreen(
    downloads: List<DownloadResult> = emptyList(),
    onDownload: (String) -> Unit = {}
) {
    var url by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Download Music",
            style = MaterialTheme.typography.headlineMedium
        )
        Spacer(modifier = Modifier.height(16.dp))

        OutlinedTextField(
            value = url,
            onValueChange = { url = it },
            label = { Text("Paste URL from YouTube or Instagram") },
            modifier = Modifier.fillMaxWidth(),
            leadingIcon = { Icon(LinkIcon, contentDescription = null) },
            trailingIcon = {
                if (url.isNotEmpty()) {
                    IconButton(onClick = { url = "" }) {
                        Icon(ClearIcon, contentDescription = "Clear")
                    }
                }
            }
        )

        Spacer(modifier = Modifier.height(16.dp))

        Button(
            onClick = { onDownload(url) },
            modifier = Modifier.fillMaxWidth(),
            enabled = url.isNotBlank()
        ) {
            Icon(DownloadIcon, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Download")
        }

        Spacer(modifier = Modifier.height(24.dp))

        if (downloads.isNotEmpty()) {
            Text(
                text = "Recent Downloads",
                style = MaterialTheme.typography.titleMedium
            )
            Spacer(modifier = Modifier.height(8.dp))

            LazyColumn {
                items(downloads) { result ->
                    DownloadResultItem(result)
                }
            }
        }
    }
}

@Composable
fun DownloadResultItem(result: DownloadResult) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
    ) {
        Row(
            modifier = Modifier
                .padding(16.dp)
                .fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically
        ) {
            when (result) {
                is DownloadResult.Success -> {
                    Icon(
                        CheckCircleIcon,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text(result.metadata.title, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "Downloaded successfully",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
                is DownloadResult.Error -> {
                    Icon(
                        ErrorCircleIcon,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text("Download failed", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            result.message,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error
                        )
                    }
                }
                is DownloadResult.Progress -> {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp))
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text("Downloading...", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "${(result.percent * 100).toInt()}%",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        }
    }
}
