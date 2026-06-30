package com.auralis.database

import kotlin.Long

public data class PlaylistTrack(
  public val playlist_id: Long,
  public val track_id: Long,
  public val position: Long,
  public val date_added: Long,
)
