package com.auralis.service

expect class FolderPicker() {
    suspend fun pickFolder(): String?
}
