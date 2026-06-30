package com.auralis.sync

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.github.sarxos.webcam.Webcam
import com.google.zxing.BinaryBitmap
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import kotlinx.coroutines.*
import java.awt.image.BufferedImage

@Composable
actual fun QrScannerView(
    onQrScanned: (String) -> Unit,
    onDismiss: () -> Unit
) {
    var error by remember { mutableStateOf<String?>(null) }
    var scanning by remember { mutableStateOf(true) }
    val reader = remember { MultiFormatReader() }

    LaunchedEffect(scanning) {
        if (!scanning) return@LaunchedEffect

        withContext(Dispatchers.IO) {
            try {
                val webcam = Webcam.getDefault()
                if (webcam == null) {
                    error = "No webcam found"
                    scanning = false
                    return@withContext
                }

                webcam.open()

                try {
                    while (scanning && isActive) {
                        val image: BufferedImage? = webcam.image
                        if (image != null) {
                            val result = decodeQrFromImage(image, reader)
                            if (result != null) {
                                withContext(Dispatchers.Main) {
                                    onQrScanned(result)
                                    scanning = false
                                }
                                break
                            }
                        }
                        delay(100)
                    }
                } finally {
                    webcam.close()
                }
            } catch (e: Exception) {
                error = e.message ?: "Camera error"
                scanning = false
            }
        }
    }

    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        if (error != null) {
            Text(
                text = error!!,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = "Make sure your camera is available.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        } else if (scanning) {
            Text(
                text = "Point your camera at the QR code on the host device",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center
            )
            Spacer(modifier = Modifier.height(16.dp))
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(200.dp)
                    .background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center
            ) {
                CircularProgressIndicator()
            }
        }
    }
}

private fun decodeQrFromImage(image: BufferedImage, reader: MultiFormatReader): String? {
    return try {
        val width = image.width
        val height = image.height
        val pixels = IntArray(width * height)
        image.getRGB(0, 0, width, height, pixels, 0, width)

        val yuv = ByteArray(width * height)
        for (i in pixels.indices) {
            val r = (pixels[i] shr 16) and 0xFF
            val g = (pixels[i] shr 8) and 0xFF
            val b = pixels[i] and 0xFF
            yuv[i] = ((0.299 * r + 0.587 * g + 0.114 * b).toInt()).toByte()
        }

        val source = PlanarYUVLuminanceSource(
            yuv, width, height, 0, 0, width, height, false
        )
        val binaryBitmap = BinaryBitmap(HybridBinarizer(source))
        val result = reader.decode(binaryBitmap)
        result.text
    } catch (_: Exception) {
        null
    }
}
