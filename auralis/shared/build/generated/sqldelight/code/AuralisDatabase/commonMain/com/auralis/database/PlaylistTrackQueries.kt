package com.auralis.database

import app.cash.sqldelight.Query
import app.cash.sqldelight.TransacterImpl
import app.cash.sqldelight.db.QueryResult
import app.cash.sqldelight.db.SqlCursor
import app.cash.sqldelight.db.SqlDriver
import kotlin.Any
import kotlin.Long
import kotlin.String

public class PlaylistTrackQueries(
  driver: SqlDriver,
) : TransacterImpl(driver) {
  public fun <T : Any> selectByPlaylist(playlist_id: Long, mapper: (
    playlist_id: Long,
    track_id: Long,
    position: Long,
    date_added: Long,
  ) -> T): Query<T> = SelectByPlaylistQuery(playlist_id) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getLong(1)!!,
      cursor.getLong(2)!!,
      cursor.getLong(3)!!
    )
  }

  public fun selectByPlaylist(playlist_id: Long): Query<PlaylistTrack> =
      selectByPlaylist(playlist_id) { playlist_id_, track_id, position, date_added ->
    PlaylistTrack(
      playlist_id_,
      track_id,
      position,
      date_added
    )
  }

  public fun insert(
    playlist_id: Long,
    track_id: Long,
    position: Long,
    date_added: Long,
  ) {
    driver.execute(-169_659_607, """
        |INSERT INTO PlaylistTrack (playlist_id, track_id, position, date_added)
        |VALUES (?, ?, ?, ?)
        """.trimMargin(), 4) {
          bindLong(0, playlist_id)
          bindLong(1, track_id)
          bindLong(2, position)
          bindLong(3, date_added)
        }
    notifyQueries(-169_659_607) { emit ->
      emit("PlaylistTrack")
    }
  }

  public fun updatePosition(
    position: Long,
    playlist_id: Long,
    track_id: Long,
  ) {
    driver.execute(-647_799_422,
        """UPDATE PlaylistTrack SET position = ? WHERE playlist_id = ? AND track_id = ?""", 3) {
          bindLong(0, position)
          bindLong(1, playlist_id)
          bindLong(2, track_id)
        }
    notifyQueries(-647_799_422) { emit ->
      emit("PlaylistTrack")
    }
  }

  public fun deleteFromPlaylist(playlist_id: Long, track_id: Long) {
    driver.execute(1_647_357_271,
        """DELETE FROM PlaylistTrack WHERE playlist_id = ? AND track_id = ?""", 2) {
          bindLong(0, playlist_id)
          bindLong(1, track_id)
        }
    notifyQueries(1_647_357_271) { emit ->
      emit("PlaylistTrack")
    }
  }

  public fun deleteAllFromPlaylist(playlist_id: Long) {
    driver.execute(1_620_279_362, """DELETE FROM PlaylistTrack WHERE playlist_id = ?""", 1) {
          bindLong(0, playlist_id)
        }
    notifyQueries(1_620_279_362) { emit ->
      emit("PlaylistTrack")
    }
  }

  private inner class SelectByPlaylistQuery<out T : Any>(
    public val playlist_id: Long,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("PlaylistTrack", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("PlaylistTrack", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(671_831_509,
        """SELECT PlaylistTrack.playlist_id, PlaylistTrack.track_id, PlaylistTrack.position, PlaylistTrack.date_added FROM PlaylistTrack WHERE playlist_id = ? ORDER BY position""",
        mapper, 1) {
      bindLong(0, playlist_id)
    }

    override fun toString(): String = "PlaylistTrack.sq:selectByPlaylist"
  }
}
