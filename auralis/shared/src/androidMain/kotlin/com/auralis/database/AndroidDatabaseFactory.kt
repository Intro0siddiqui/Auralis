package com.auralis.database

import android.content.Context
import app.cash.sqldelight.driver.android.AndroidSqliteDriver

actual fun createDatabase(): AuralisDatabase {
    throw UnsupportedOperationException("Android database requires Context - use createDatabase(context) instead")
}

fun createDatabase(context: Context): AuralisDatabase {
    val driver = AndroidSqliteDriver(
        schema = AuralisDatabase.Schema,
        context = context,
        name = "auralis.db"
    )
    return AuralisDatabase(driver)
}
