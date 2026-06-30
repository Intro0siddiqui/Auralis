package com.auralis.sync

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

class AccountManagerTest {

    @Test
    fun testPasswordHashing() {
        val salt1 = "abc123"
        val salt2 = "def456"
        val password = "mypassword"

        val hash1 = hashPassword(password, salt1)
        val hash2 = hashPassword(password, salt2)

        assertNotEquals(hash1, hash2)
        assertEquals(hash1, hashPassword(password, salt1))
    }

    @Test
    fun testDifferentPasswordDifferentHash() {
        val salt = "abc123"
        val hash1 = hashPassword("password1", salt)
        val hash2 = hashPassword("password2", salt)

        assertNotEquals(hash1, hash2)
    }

    @Test
    fun testHashLength() {
        val hash = hashPassword("test", "salt")
        assertEquals(44, hash.length) // SHA-256 Base64 encoded
    }

    private fun hashPassword(password: String, salt: String): String {
        val saltBytes = java.util.Base64.getDecoder().decode(salt)
        val passwordBytes = password.toByteArray(Charsets.UTF_8)
        val combined = ByteArray(saltBytes.size + passwordBytes.size)
        System.arraycopy(saltBytes, 0, combined, 0, saltBytes.size)
        System.arraycopy(passwordBytes, 0, combined, saltBytes.size, passwordBytes.size)

        val digest = java.security.MessageDigest.getInstance("SHA-256")
        val hash = digest.digest(combined)
        return java.util.Base64.getEncoder().encodeToString(hash)
    }
}
