package com.auralis.database

import kotlin.Long
import kotlin.String

public data class SyncAccount(
  public val id: Long,
  public val username: String,
  public val password_hash: String,
  public val salt: String,
  public val date_created: Long,
)
