package com.auralis.sync

import androidx.compose.runtime.Composable

expect @Composable fun QrScannerView(
    onQrScanned: (String) -> Unit,
    onDismiss: () -> Unit
)
