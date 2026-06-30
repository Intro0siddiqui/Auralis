package com.auralis.database

import app.cash.sqldelight.Query
import app.cash.sqldelight.TransacterImpl
import app.cash.sqldelight.db.QueryResult
import app.cash.sqldelight.db.SqlCursor
import app.cash.sqldelight.db.SqlDriver
import kotlin.Any
import kotlin.Long
import kotlin.String

public class PlaylistQueries(
  driver: SqlDriver,
) : TransacterImpl(driver) {
  public fun <T : Any> selectAll(mapper: (
    id: Long,
    name: String,
    description: String?,
    thumbnail_path: String?,
    date_created: Long,
    date_modified: Long,
  ) -> T): Query<T> = Query(942_694_062, arrayOf("Playlist"), driver, "Playlist.sq", "selectAll",
      "SELECT Playlist.id, Playlist.name, Playlist.description, Playlist.thumbnail_path, Playlist.date_created, Playlist.date_modified FROM Playlist ORDER BY date_created DESC") {
      cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2),
      cursor.getString(3),
      cursor.getLong(4)!!,
      cursor.getLong(5)!!
    )
  }

  public fun selectAll(): Query<Playlist> = selectAll { id, name, description, thumbnail_path,
      date_created, date_modified ->
    Playlist(
      id,
      name,
      description,
      thumbnail_path,
      date_created,
      date_modified
    )
  }

  public fun <T : Any> selectById(id: Long, mapper: (
    id: Long,
    name: String,
    description: String?,
    thumbnail_path: String?,
    date_created: Long,
    date_modified: Long,
  ) -> T): Query<T> = SelectByIdQuery(id) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2),
      cursor.getString(3),
      cursor.getLong(4)!!,
      cursor.getLong(5)!!
    )
  }

  public fun selectById(id: Long): Query<Playlist> = selectById(id) { id_, name, description,
      thumbnail_path, date_created, date_modified ->
    Playlist(
      id_,
      name,
      description,
      thumbnail_path,
      date_created,
      date_modified
    )
  }

  public fun insert(
    name: String,
    description: String?,
    thumbnail_path: String?,
    date_created: Long,
    date_modified: Long,
  ) {
    driver.execute(-1_498_282_064, """
        |INSERT INTO Playlist (name, description, thumbnail_path, date_created, date_modified)
        |VALUES (?, ?, ?, ?, ?)
        """.trimMargin(), 5) {
          bindString(0, name)
          bindString(1, description)
          bindString(2, thumbnail_path)
          bindLong(3, date_created)
          bindLong(4, date_modified)
        }
    notifyQueries(-1_498_282_064) { emit ->
      emit("Playlist")
    }
  }

  public fun update(
    name: String,
    description: String?,
    thumbnail_path: String?,
    date_modified: Long,
    id: Long,
  ) {
    driver.execute(-1_153_335_872, """
        |UPDATE Playlist
        |SET name = ?, description = ?, thumbnail_path = ?, date_modified = ?
        |WHERE id = ?
        """.trimMargin(), 5) {
          bindString(0, name)
          bindString(1, description)
          bindString(2, thumbnail_path)
          bindLong(3, date_modified)
          bindLong(4, id)
        }
    notifyQueries(-1_153_335_872) { emit ->
      emit("Playlist")
    }
  }

  public fun deleteById(id: Long) {
    driver.execute(-1_715_635_820, """DELETE FROM Playlist WHERE id = ?""", 1) {
          bindLong(0, id)
        }
    notifyQueries(-1_715_635_820) { emit ->
      emit("Playlist")
      emit("PlaylistTrack")
    }
  }

  private inner class SelectByIdQuery<out T : Any>(
    public val id: Long,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("Playlist", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("Playlist", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(-841_213_851,
        """SELECT Playlist.id, Playlist.name, Playlist.description, Playlist.thumbnail_path, Playlist.date_created, Playlist.date_modified FROM Playlist WHERE id = ?""",
        mapper, 1) {
      bindLong(0, id)
    }

    override fun toString(): String = "Playlist.sq:selectById"
  }
}
