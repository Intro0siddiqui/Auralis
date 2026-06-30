package com.auralis.database.shared

import app.cash.sqldelight.TransacterImpl
import app.cash.sqldelight.db.AfterVersion
import app.cash.sqldelight.db.QueryResult
import app.cash.sqldelight.db.SqlDriver
import app.cash.sqldelight.db.SqlSchema
import com.auralis.database.AccountQueries
import com.auralis.database.AuralisDatabase
import com.auralis.database.PlaylistQueries
import com.auralis.database.PlaylistTrackQueries
import com.auralis.database.TrackQueries
import kotlin.Long
import kotlin.Unit
import kotlin.reflect.KClass

internal val KClass<AuralisDatabase>.schema: SqlSchema<QueryResult.Value<Unit>>
  get() = AuralisDatabaseImpl.Schema

internal fun KClass<AuralisDatabase>.newInstance(driver: SqlDriver): AuralisDatabase =
    AuralisDatabaseImpl(driver)

private class AuralisDatabaseImpl(
  driver: SqlDriver,
) : TransacterImpl(driver), AuralisDatabase {
  override val accountQueries: AccountQueries = AccountQueries(driver)

  override val playlistQueries: PlaylistQueries = PlaylistQueries(driver)

  override val playlistTrackQueries: PlaylistTrackQueries = PlaylistTrackQueries(driver)

  override val trackQueries: TrackQueries = TrackQueries(driver)

  public object Schema : SqlSchema<QueryResult.Value<Unit>> {
    override val version: Long
      get() = 1

    override fun create(driver: SqlDriver): QueryResult.Value<Unit> {
      driver.execute(null, """
          |CREATE TABLE SyncAccount (
          |    id INTEGER PRIMARY KEY AUTOINCREMENT,
          |    username TEXT NOT NULL UNIQUE,
          |    password_hash TEXT NOT NULL,
          |    salt TEXT NOT NULL,
          |    date_created INTEGER NOT NULL DEFAULT 0
          |)
          """.trimMargin(), 0)
      driver.execute(null, """
          |CREATE TABLE Playlist (
          |    id INTEGER PRIMARY KEY AUTOINCREMENT,
          |    name TEXT NOT NULL,
          |    description TEXT,
          |    thumbnail_path TEXT,
          |    date_created INTEGER NOT NULL DEFAULT 0,
          |    date_modified INTEGER NOT NULL DEFAULT 0
          |)
          """.trimMargin(), 0)
      driver.execute(null, """
          |CREATE TABLE PlaylistTrack (
          |    playlist_id INTEGER NOT NULL,
          |    track_id INTEGER NOT NULL,
          |    position INTEGER NOT NULL DEFAULT 0,
          |    date_added INTEGER NOT NULL DEFAULT 0,
          |    PRIMARY KEY (playlist_id, track_id),
          |    FOREIGN KEY (playlist_id) REFERENCES Playlist(id) ON DELETE CASCADE,
          |    FOREIGN KEY (track_id) REFERENCES Track(id) ON DELETE CASCADE
          |)
          """.trimMargin(), 0)
      driver.execute(null, """
          |CREATE TABLE Track (
          |    id INTEGER PRIMARY KEY AUTOINCREMENT,
          |    title TEXT NOT NULL,
          |    artist TEXT,
          |    album TEXT,
          |    genre TEXT,
          |    year INTEGER,
          |    track_number INTEGER,
          |    duration INTEGER NOT NULL DEFAULT 0,
          |    file_path TEXT NOT NULL UNIQUE,
          |    thumbnail_path TEXT,
          |    format TEXT NOT NULL DEFAULT 'mp3',
          |    size INTEGER NOT NULL DEFAULT 0,
          |    date_added INTEGER NOT NULL DEFAULT 0,
          |    source TEXT,
          |    source_url TEXT
          |)
          """.trimMargin(), 0)
      return QueryResult.Unit
    }

    override fun migrate(
      driver: SqlDriver,
      oldVersion: Long,
      newVersion: Long,
      vararg callbacks: AfterVersion,
    ): QueryResult.Value<Unit> = QueryResult.Unit
  }
}
