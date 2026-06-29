package com.auralis.model

data class Playlist(
    val id: Long = 0,
    val name: String,
    val description: String? = null,
    val thumbnailPath: String? = null,
    val dateCreated: Long = System.currentTimeMillis(),
    val dateModified: Long = System.currentTimeMillis(),
    val trackCount: Int = 0
)
