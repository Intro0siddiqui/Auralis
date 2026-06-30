package com.auralis.model

data class SyncAccount(
    val id: Long = 0,
    val username: String,
    val passwordHash: String,
    val salt: String,
    val dateCreated: Long = System.currentTimeMillis()
)
