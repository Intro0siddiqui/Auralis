package com.auralis.database

import app.cash.sqldelight.Query
import app.cash.sqldelight.TransacterImpl
import app.cash.sqldelight.db.QueryResult
import app.cash.sqldelight.db.SqlCursor
import app.cash.sqldelight.db.SqlDriver
import kotlin.Any
import kotlin.Long
import kotlin.String

public class AccountQueries(
  driver: SqlDriver,
) : TransacterImpl(driver) {
  public fun <T : Any> selectByUsername(username: String, mapper: (
    id: Long,
    username: String,
    password_hash: String,
    salt: String,
    date_created: Long,
  ) -> T): Query<T> = SelectByUsernameQuery(username) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2)!!,
      cursor.getString(3)!!,
      cursor.getLong(4)!!
    )
  }

  public fun selectByUsername(username: String): Query<SyncAccount> = selectByUsername(username) {
      id, username_, password_hash, salt, date_created ->
    SyncAccount(
      id,
      username_,
      password_hash,
      salt,
      date_created
    )
  }

  public fun <T : Any> selectById(id: Long, mapper: (
    id: Long,
    username: String,
    password_hash: String,
    salt: String,
    date_created: Long,
  ) -> T): Query<T> = SelectByIdQuery(id) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2)!!,
      cursor.getString(3)!!,
      cursor.getLong(4)!!
    )
  }

  public fun selectById(id: Long): Query<SyncAccount> = selectById(id) { id_, username,
      password_hash, salt, date_created ->
    SyncAccount(
      id_,
      username,
      password_hash,
      salt,
      date_created
    )
  }

  public fun <T : Any> selectAll(mapper: (
    id: Long,
    username: String,
    password_hash: String,
    salt: String,
    date_created: Long,
  ) -> T): Query<T> = Query(488_129, arrayOf("SyncAccount"), driver, "Account.sq", "selectAll",
      "SELECT SyncAccount.id, SyncAccount.username, SyncAccount.password_hash, SyncAccount.salt, SyncAccount.date_created FROM SyncAccount") {
      cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2)!!,
      cursor.getString(3)!!,
      cursor.getLong(4)!!
    )
  }

  public fun selectAll(): Query<SyncAccount> = selectAll { id, username, password_hash, salt,
      date_created ->
    SyncAccount(
      id,
      username,
      password_hash,
      salt,
      date_created
    )
  }

  public fun insert(
    username: String,
    password_hash: String,
    salt: String,
    date_created: Long,
  ) {
    driver.execute(1_881_318_525, """
        |INSERT INTO SyncAccount (username, password_hash, salt, date_created)
        |VALUES (?, ?, ?, ?)
        """.trimMargin(), 4) {
          bindString(0, username)
          bindString(1, password_hash)
          bindString(2, salt)
          bindLong(3, date_created)
        }
    notifyQueries(1_881_318_525) { emit ->
      emit("SyncAccount")
    }
  }

  public fun deleteById(id: Long) {
    driver.execute(-859_248_671, """DELETE FROM SyncAccount WHERE id = ?""", 1) {
          bindLong(0, id)
        }
    notifyQueries(-859_248_671) { emit ->
      emit("SyncAccount")
    }
  }

  public fun deleteByUsername(username: String) {
    driver.execute(-8_866_020, """DELETE FROM SyncAccount WHERE username = ?""", 1) {
          bindString(0, username)
        }
    notifyQueries(-8_866_020) { emit ->
      emit("SyncAccount")
    }
  }

  private inner class SelectByUsernameQuery<out T : Any>(
    public val username: String,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("SyncAccount", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("SyncAccount", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(1_839_150_381,
        """SELECT SyncAccount.id, SyncAccount.username, SyncAccount.password_hash, SyncAccount.salt, SyncAccount.date_created FROM SyncAccount WHERE username = ?""",
        mapper, 1) {
      bindString(0, username)
    }

    override fun toString(): String = "Account.sq:selectByUsername"
  }

  private inner class SelectByIdQuery<out T : Any>(
    public val id: Long,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("SyncAccount", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("SyncAccount", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(15_173_298,
        """SELECT SyncAccount.id, SyncAccount.username, SyncAccount.password_hash, SyncAccount.salt, SyncAccount.date_created FROM SyncAccount WHERE id = ?""",
        mapper, 1) {
      bindLong(0, id)
    }

    override fun toString(): String = "Account.sq:selectById"
  }
}
