package com.auralis.sync

import com.auralis.database.AuralisDatabase
import com.auralis.model.SyncAccount
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64

class AccountManager(private val database: AuralisDatabase) {
    private val queries = database.accountQueries

    suspend fun createAccount(username: String, password: String): Result<SyncAccount> =
        withContext(Dispatchers.Default) {
            try {
                if (username.isBlank() || password.isBlank()) {
                    return@withContext Result.failure(IllegalArgumentException("Username and password cannot be empty"))
                }
                if (username.length < 3) {
                    return@withContext Result.failure(IllegalArgumentException("Username must be at least 3 characters"))
                }
                if (password.length < 6) {
                    return@withContext Result.failure(IllegalArgumentException("Password must be at least 6 characters"))
                }

                val existing = queries.selectByUsername(username).executeAsOneOrNull()
                if (existing != null) {
                    return@withContext Result.failure(IllegalStateException("Username already exists"))
                }

                val salt = generateSalt()
                val hash = hashPassword(password, salt)

                queries.insert(
                    username = username,
                    password_hash = hash,
                    salt = salt,
                    date_created = System.currentTimeMillis()
                )

                val account = queries.selectByUsername(username).executeAsOne().toDomain()
                Result.success(account)
            } catch (e: Exception) {
                Result.failure(e)
            }
        }

    suspend fun login(username: String, password: String): Result<SyncAccount> =
        withContext(Dispatchers.Default) {
            try {
                val account = queries.selectByUsername(username).executeAsOneOrNull()
                    ?: return@withContext Result.failure(IllegalArgumentException("Invalid username or password"))

                val hash = hashPassword(password, account.salt)
                if (hash != account.password_hash) {
                    return@withContext Result.failure(IllegalArgumentException("Invalid username or password"))
                }

                Result.success(account.toDomain())
            } catch (e: Exception) {
                Result.failure(e)
            }
        }

    suspend fun getAccount(username: String): SyncAccount? = withContext(Dispatchers.Default) {
        queries.selectByUsername(username).executeAsOneOrNull()?.toDomain()
    }

    suspend fun getAllAccounts(): List<SyncAccount> = withContext(Dispatchers.Default) {
        queries.selectAll().executeAsList().map { it.toDomain() }
    }

    suspend fun deleteAccount(id: Long) = withContext(Dispatchers.Default) {
        queries.deleteById(id)
    }

    suspend fun deleteAccount(username: String) = withContext(Dispatchers.Default) {
        queries.deleteByUsername(username)
    }

    private fun generateSalt(): String {
        val salt = ByteArray(16)
        SecureRandom().nextBytes(salt)
        return Base64.getEncoder().encodeToString(salt)
    }

    private fun hashPassword(password: String, salt: String): String {
        val saltBytes = Base64.getDecoder().decode(salt)
        val passwordBytes = password.toByteArray(Charsets.UTF_8)
        val combined = ByteArray(saltBytes.size + passwordBytes.size)
        System.arraycopy(saltBytes, 0, combined, 0, saltBytes.size)
        System.arraycopy(passwordBytes, 0, combined, saltBytes.size, passwordBytes.size)

        val digest = MessageDigest.getInstance("SHA-256")
        val hash = digest.digest(combined)
        return Base64.getEncoder().encodeToString(hash)
    }

    private fun com.auralis.database.SyncAccount.toDomain() = SyncAccount(
        id = id,
        username = username,
        passwordHash = password_hash,
        salt = salt,
        dateCreated = date_created
    )
}
