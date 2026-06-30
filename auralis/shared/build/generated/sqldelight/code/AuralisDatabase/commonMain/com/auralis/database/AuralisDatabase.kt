package com.auralis.database

import app.cash.sqldelight.Transacter
import app.cash.sqldelight.db.QueryResult
import app.cash.sqldelight.db.SqlDriver
import app.cash.sqldelight.db.SqlSchema
import com.auralis.database.shared.newInstance
import com.auralis.database.shared.schema
import kotlin.Unit

public interface AuralisDatabase : Transacter {
  public val accountQueries: AccountQueries

  public val playlistQueries: PlaylistQueries

  public val playlistTrackQueries: PlaylistTrackQueries

  public val trackQueries: TrackQueries

  public companion object {
    public val Schema: SqlSchema<QueryResult.Value<Unit>>
      get() = AuralisDatabase::class.schema

    public operator fun invoke(driver: SqlDriver): AuralisDatabase =
        AuralisDatabase::class.newInstance(driver)
  }
}
