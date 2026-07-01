package com.auralis.database

import app.cash.sqldelight.driver.jdbc.sqlite.JdbcSqliteDriver
import java.io.File

actual fun createDatabase(): AuralisDatabase {
    val home = System.getProperty("user.home") ?: "."
    val storageDir = File(home, ".auralis")
    storageDir.mkdirs()
    val dbDir = File(storageDir, "database")
    dbDir.mkdirs()
    val dbFile = File(dbDir, "auralis.db")
    val driver = JdbcSqliteDriver("jdbc:sqlite:${dbFile.path}")
    if (dbFile.exists() && dbFile.length() > 0) {
        AuralisDatabase.Schema.migrate(driver, AuralisDatabase.Schema.version, AuralisDatabase.Schema.version)
    } else {
        AuralisDatabase.Schema.create(driver)
    }
    return AuralisDatabase(driver)
}
