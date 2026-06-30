package com.auralis.sync

import android.util.Base64

actual object Base64Util {
    actual fun encode(bytes: ByteArray): String =
        Base64.encodeToString(bytes, Base64.NO_WRAP)

    actual fun decode(string: String): ByteArray =
        Base64.decode(string, Base64.NO_WRAP)
}
