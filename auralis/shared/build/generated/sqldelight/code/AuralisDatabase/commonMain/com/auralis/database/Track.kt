package com.auralis.database

import kotlin.Long
import kotlin.String

public data class Track(
  public val id: Long,
  public val title: String,
  public val artist: String?,
  public val album: String?,
  public val genre: String?,
  public val year: Long?,
  public val track_number: Long?,
  public val duration: Long,
  public val file_path: String,
  public val thumbnail_path: String?,
  public val format: String,
  public val size: Long,
  public val date_added: Long,
  public val source: String?,
  public val source_url: String?,
)
