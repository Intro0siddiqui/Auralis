package com.auralis.sync

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.geometry.Rect

object QrCodeManager {

    data class ConnectionInfo(
        val host: String,
        val port: Int,
        val token: String
    ) {
        fun toJson(): String = """{"host":"$host","port":$port,"token":"$token"}"""

        companion object {
            fun fromJson(json: String): ConnectionInfo? {
                return try {
                    val host = json.substringAfter("\"host\":\"").substringBefore("\"")
                    val port = json.substringAfter("\"port\":").substringBefore(",").toInt()
                    val token = json.substringAfter("\"token\":\"").substringBefore("\"")
                    ConnectionInfo(host, port, token)
                } catch (_: Exception) {
                    null
                }
            }
        }
    }

    fun generateQrMatrix(content: String, size: Int = 25): List<List<Boolean>> {
        val matrix = Array(size) { BooleanArray(size) }

        val binary = content.toByteArray(Charsets.UTF_8)
        val bits = mutableListOf<Boolean>()
        for (byte in binary) {
            for (i in 7 downTo 0) {
                bits.add((byte.toInt() shr i) and 1 == 1)
            }
        }

        addFinderPattern(matrix, 0, 0, size)
        addFinderPattern(matrix, size - 7, 0, size)
        addFinderPattern(matrix, 0, size - 7, size)

        addTimingPatterns(matrix, size)

        var bitIndex = 0
        for (row in 0 until size) {
            for (col in 0 until size) {
                if (isPatternArea(row, col, size)) continue
                if (bitIndex < bits.size) {
                    matrix[row][col] = bits[bitIndex]
                    bitIndex++
                }
            }
        }

        return matrix.map { row -> row.toList() }
    }

    private fun addFinderPattern(matrix: Array<BooleanArray>, startRow: Int, startCol: Int, size: Int) {
        for (r in 0..6) {
            for (c in 0..6) {
                val row = startRow + r
                val col = startCol + c
                if (row in 0 until size && col in 0 until size) {
                    matrix[row][col] = r == 0 || r == 6 || c == 0 || c == 6 ||
                            (r in 2..4 && c in 2..4)
                }
            }
        }
    }

    private fun addTimingPatterns(matrix: Array<BooleanArray>, size: Int) {
        for (i in 8 until size - 8) {
            if (i % 2 == 0) {
                matrix[6][i] = true
                matrix[i][6] = true
            }
        }
    }

    private fun isPatternArea(row: Int, col: Int, size: Int): Boolean {
        if (row < 9 && col < 9) return true
        if (row < 9 && col >= size - 8) return true
        if (row >= size - 8 && col < 9) return true
        if (row == 6 || col == 6) return true
        return false
    }

    fun generateQrBitmap(content: String, pixelSize: Int = 300): ImageBitmap {
        val matrix = generateQrMatrix(content)
        val size = matrix.size
        val moduleSize = pixelSize / size

        val bitmap = ImageBitmap(pixelSize, pixelSize)
        val canvas = Canvas(bitmap)

        val bgPaint = Paint().apply { color = Color.White }
        val fgPaint = Paint().apply { color = Color.Black }

        canvas.drawRect(Rect(0f, 0f, pixelSize.toFloat(), pixelSize.toFloat()), bgPaint)

        for (row in matrix.indices) {
            for (col in matrix[row].indices) {
                if (matrix[row][col]) {
                    canvas.drawRect(
                        Rect(
                            col * moduleSize.toFloat(),
                            row * moduleSize.toFloat(),
                            (col + 1) * moduleSize.toFloat(),
                            (row + 1) * moduleSize.toFloat()
                        ),
                        fgPaint
                    )
                }
            }
        }

        return bitmap
    }

    fun generateConnectionInfo(
        ip: String,
        port: Int,
        token: String
    ): ConnectionInfo {
        return ConnectionInfo(ip, port, token)
    }

    private val TEMP_CODE_CHARS = "0123456789"

    fun generateTempCode(): String {
        val random = java.security.SecureRandom()
        return (1..6).map { TEMP_CODE_CHARS[random.nextInt(TEMP_CODE_CHARS.length)] }.joinToString("")
    }
}
