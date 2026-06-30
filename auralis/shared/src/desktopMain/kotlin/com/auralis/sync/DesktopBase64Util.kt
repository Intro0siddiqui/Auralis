package com.auralis.sync

actual object Base64Util {
    actual fun encode(bytes: ByteArray): String =
        java.util.Base64.getEncoder().encodeToString(bytes)

    actual fun decode(string: String): ByteArray =
        java.util.Base64.getDecoder().decode(string)
}
