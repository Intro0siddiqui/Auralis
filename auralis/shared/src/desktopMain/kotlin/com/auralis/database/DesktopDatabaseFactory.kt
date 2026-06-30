package com.auralis.database

import app.cash.sqldelight.driver.jdbc.sqlite.JdbcSqliteDriver
import java.io.File

actual fun createDatabase(): AuralisDatabase {
    val home = System.getProperty("user.home") ?: "."
    val storageDir = File(home, ".auralis")
    storageDir.mkdirs()
    val dbDir = File(storageDir, "database")
    dbDir.mkdirs()
    val driver = JdbcSqliteDriver("jdbc:sqlite:${dbDir.path}/auralis.db")
    AuralisDatabase.Schema.create(driver)
    return AuralisDatabase(driver)
}
