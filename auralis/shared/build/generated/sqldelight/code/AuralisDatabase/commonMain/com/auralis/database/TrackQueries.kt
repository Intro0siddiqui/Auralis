package com.auralis.database

import app.cash.sqldelight.Query
import app.cash.sqldelight.TransacterImpl
import app.cash.sqldelight.db.QueryResult
import app.cash.sqldelight.db.SqlCursor
import app.cash.sqldelight.db.SqlDriver
import kotlin.Any
import kotlin.Long
import kotlin.String

public class TrackQueries(
  driver: SqlDriver,
) : TransacterImpl(driver) {
  public fun <T : Any> selectAll(mapper: (
    id: Long,
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Long?,
    track_number: Long?,
    duration: Long,
    file_path: String,
    thumbnail_path: String?,
    format: String,
    size: Long,
    date_added: Long,
    source: String?,
    source_url: String?,
  ) -> T): Query<T> = Query(768_052_771, arrayOf("Track"), driver, "Track.sq", "selectAll",
      "SELECT Track.id, Track.title, Track.artist, Track.album, Track.genre, Track.year, Track.track_number, Track.duration, Track.file_path, Track.thumbnail_path, Track.format, Track.size, Track.date_added, Track.source, Track.source_url FROM Track ORDER BY date_added DESC") {
      cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2),
      cursor.getString(3),
      cursor.getString(4),
      cursor.getLong(5),
      cursor.getLong(6),
      cursor.getLong(7)!!,
      cursor.getString(8)!!,
      cursor.getString(9),
      cursor.getString(10)!!,
      cursor.getLong(11)!!,
      cursor.getLong(12)!!,
      cursor.getString(13),
      cursor.getString(14)
    )
  }

  public fun selectAll(): Query<Track> = selectAll { id, title, artist, album, genre, year,
      track_number, duration, file_path, thumbnail_path, format, size, date_added, source,
      source_url ->
    Track(
      id,
      title,
      artist,
      album,
      genre,
      year,
      track_number,
      duration,
      file_path,
      thumbnail_path,
      format,
      size,
      date_added,
      source,
      source_url
    )
  }

  public fun <T : Any> selectById(id: Long, mapper: (
    id: Long,
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Long?,
    track_number: Long?,
    duration: Long,
    file_path: String,
    thumbnail_path: String?,
    format: String,
    size: Long,
    date_added: Long,
    source: String?,
    source_url: String?,
  ) -> T): Query<T> = SelectByIdQuery(id) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2),
      cursor.getString(3),
      cursor.getString(4),
      cursor.getLong(5),
      cursor.getLong(6),
      cursor.getLong(7)!!,
      cursor.getString(8)!!,
      cursor.getString(9),
      cursor.getString(10)!!,
      cursor.getLong(11)!!,
      cursor.getLong(12)!!,
      cursor.getString(13),
      cursor.getString(14)
    )
  }

  public fun selectById(id: Long): Query<Track> = selectById(id) { id_, title, artist, album, genre,
      year, track_number, duration, file_path, thumbnail_path, format, size, date_added, source,
      source_url ->
    Track(
      id_,
      title,
      artist,
      album,
      genre,
      year,
      track_number,
      duration,
      file_path,
      thumbnail_path,
      format,
      size,
      date_added,
      source,
      source_url
    )
  }

  public fun <T : Any> selectByTitle(title: String, mapper: (
    id: Long,
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Long?,
    track_number: Long?,
    duration: Long,
    file_path: String,
    thumbnail_path: String?,
    format: String,
    size: Long,
    date_added: Long,
    source: String?,
    source_url: String?,
  ) -> T): Query<T> = SelectByTitleQuery(title) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2),
      cursor.getString(3),
      cursor.getString(4),
      cursor.getLong(5),
      cursor.getLong(6),
      cursor.getLong(7)!!,
      cursor.getString(8)!!,
      cursor.getString(9),
      cursor.getString(10)!!,
      cursor.getLong(11)!!,
      cursor.getLong(12)!!,
      cursor.getString(13),
      cursor.getString(14)
    )
  }

  public fun selectByTitle(title: String): Query<Track> = selectByTitle(title) { id, title_, artist,
      album, genre, year, track_number, duration, file_path, thumbnail_path, format, size,
      date_added, source, source_url ->
    Track(
      id,
      title_,
      artist,
      album,
      genre,
      year,
      track_number,
      duration,
      file_path,
      thumbnail_path,
      format,
      size,
      date_added,
      source,
      source_url
    )
  }

  public fun <T : Any> selectBySource(source: String?, mapper: (
    id: Long,
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Long?,
    track_number: Long?,
    duration: Long,
    file_path: String,
    thumbnail_path: String?,
    format: String,
    size: Long,
    date_added: Long,
    source: String?,
    source_url: String?,
  ) -> T): Query<T> = SelectBySourceQuery(source) { cursor ->
    mapper(
      cursor.getLong(0)!!,
      cursor.getString(1)!!,
      cursor.getString(2),
      cursor.getString(3),
      cursor.getString(4),
      cursor.getLong(5),
      cursor.getLong(6),
      cursor.getLong(7)!!,
      cursor.getString(8)!!,
      cursor.getString(9),
      cursor.getString(10)!!,
      cursor.getLong(11)!!,
      cursor.getLong(12)!!,
      cursor.getString(13),
      cursor.getString(14)
    )
  }

  public fun selectBySource(source: String?): Query<Track> = selectBySource(source) { id, title,
      artist, album, genre, year, track_number, duration, file_path, thumbnail_path, format, size,
      date_added, source_, source_url ->
    Track(
      id,
      title,
      artist,
      album,
      genre,
      year,
      track_number,
      duration,
      file_path,
      thumbnail_path,
      format,
      size,
      date_added,
      source_,
      source_url
    )
  }

  public fun insert(
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Long?,
    track_number: Long?,
    duration: Long,
    file_path: String,
    thumbnail_path: String?,
    format: String,
    size: Long,
    date_added: Long,
    source: String?,
    source_url: String?,
  ) {
    driver.execute(-822_274_981, """
        |INSERT INTO Track (title, artist, album, genre, year, track_number, duration, file_path, thumbnail_path, format, size, date_added, source, source_url)
        |VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """.trimMargin(), 14) {
          bindString(0, title)
          bindString(1, artist)
          bindString(2, album)
          bindString(3, genre)
          bindLong(4, year)
          bindLong(5, track_number)
          bindLong(6, duration)
          bindString(7, file_path)
          bindString(8, thumbnail_path)
          bindString(9, format)
          bindLong(10, size)
          bindLong(11, date_added)
          bindString(12, source)
          bindString(13, source_url)
        }
    notifyQueries(-822_274_981) { emit ->
      emit("Track")
    }
  }

  public fun update(
    title: String,
    artist: String?,
    album: String?,
    genre: String?,
    year: Long?,
    track_number: Long?,
    id: Long,
  ) {
    driver.execute(-477_328_789, """
        |UPDATE Track
        |SET title = ?, artist = ?, album = ?, genre = ?, year = ?, track_number = ?
        |WHERE id = ?
        """.trimMargin(), 7) {
          bindString(0, title)
          bindString(1, artist)
          bindString(2, album)
          bindString(3, genre)
          bindLong(4, year)
          bindLong(5, track_number)
          bindLong(6, id)
        }
    notifyQueries(-477_328_789) { emit ->
      emit("Track")
    }
  }

  public fun deleteById(id: Long) {
    driver.execute(1_460_418_751, """DELETE FROM Track WHERE id = ?""", 1) {
          bindLong(0, id)
        }
    notifyQueries(1_460_418_751) { emit ->
      emit("PlaylistTrack")
      emit("Track")
    }
  }

  public fun createTitleIndex() {
    driver.execute(766_996_120, """CREATE INDEX Track_title ON Track(title)""", 0)
  }

  public fun createArtistIndex() {
    driver.execute(119_624_781, """CREATE INDEX Track_artist ON Track(artist)""", 0)
  }

  public fun createAlbumIndex() {
    driver.execute(1_694_173_409, """CREATE INDEX Track_album ON Track(album)""", 0)
  }

  private inner class SelectByIdQuery<out T : Any>(
    public val id: Long,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("Track", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("Track", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(-1_960_126_576,
        """SELECT Track.id, Track.title, Track.artist, Track.album, Track.genre, Track.year, Track.track_number, Track.duration, Track.file_path, Track.thumbnail_path, Track.format, Track.size, Track.date_added, Track.source, Track.source_url FROM Track WHERE id = ?""",
        mapper, 1) {
      bindLong(0, id)
    }

    override fun toString(): String = "Track.sq:selectById"
  }

  private inner class SelectByTitleQuery<out T : Any>(
    public val title: String,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("Track", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("Track", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(254_953_411,
        """SELECT Track.id, Track.title, Track.artist, Track.album, Track.genre, Track.year, Track.track_number, Track.duration, Track.file_path, Track.thumbnail_path, Track.format, Track.size, Track.date_added, Track.source, Track.source_url FROM Track WHERE title LIKE ?""",
        mapper, 1) {
      bindString(0, title)
    }

    override fun toString(): String = "Track.sq:selectByTitle"
  }

  private inner class SelectBySourceQuery<out T : Any>(
    public val source: String?,
    mapper: (SqlCursor) -> T,
  ) : Query<T>(mapper) {
    override fun addListener(listener: Query.Listener) {
      driver.addListener("Track", listener = listener)
    }

    override fun removeListener(listener: Query.Listener) {
      driver.removeListener("Track", listener = listener)
    }

    override fun <R> execute(mapper: (SqlCursor) -> QueryResult<R>): QueryResult<R> =
        driver.executeQuery(null,
        """SELECT Track.id, Track.title, Track.artist, Track.album, Track.genre, Track.year, Track.track_number, Track.duration, Track.file_path, Track.thumbnail_path, Track.format, Track.size, Track.date_added, Track.source, Track.source_url FROM Track WHERE source ${ if (source == null) "IS" else "=" } ?""",
        mapper, 1) {
      bindString(0, source)
    }

    override fun toString(): String = "Track.sq:selectBySource"
  }
}
