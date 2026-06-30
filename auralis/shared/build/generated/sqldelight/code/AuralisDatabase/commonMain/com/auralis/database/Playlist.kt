package com.auralis.database

import kotlin.Long
import kotlin.String

public data class Playlist(
  public val id: Long,
  public val name: String,
  public val description: String?,
  public val thumbnail_path: String?,
  public val date_created: Long,
  public val date_modified: Long,
)
