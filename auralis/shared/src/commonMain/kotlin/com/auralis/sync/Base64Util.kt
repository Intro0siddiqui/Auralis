package com.auralis.sync

expect object Base64Util {
    fun encode(bytes: ByteArray): String
    fun decode(string: String): ByteArray
}
