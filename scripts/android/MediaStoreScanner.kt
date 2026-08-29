package com.auralis.v2

import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import org.json.JSONArray
import org.json.JSONObject

object MediaStoreScanner {

    @JvmStatic
    fun queryAllAudio(context: Context): String {
        val jsonArray = JSONArray()
        val resolver: ContentResolver = context.contentResolver

        val collection: Uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL)
        } else {
            MediaStore.Audio.Media.EXTERNAL_CONTENT_URI
        }

        val projection = arrayOf(
            MediaStore.Audio.Media._ID,
            MediaStore.Audio.Media.DATA,
            MediaStore.Audio.Media.TITLE,
            MediaStore.Audio.Media.ARTIST,
            MediaStore.Audio.Media.ALBUM,
            MediaStore.Audio.Media.DURATION,
            MediaStore.Audio.Media.TRACK,
            MediaStore.Audio.Media.YEAR,
            MediaStore.Audio.Media.SIZE,
            MediaStore.Audio.Media.MIME_TYPE,
            MediaStore.Audio.Media.ALBUM_ID
        )

        // Filter: only audio located in standard Music or Download directories, size > 10 KB
        val selection = "((${MediaStore.Audio.Media.IS_MUSIC} != 0) OR (${MediaStore.Audio.Media.IS_PODCAST} != 0) OR (${MediaStore.Audio.Media.IS_AUDIOBOOK} != 0)) " +
                "AND (${MediaStore.Audio.Media.SIZE} > 10240) " +
                "AND (${MediaStore.Audio.Media.DATA} LIKE '%/Music/%' OR ${MediaStore.Audio.Media.DATA} LIKE '%/Download/%' OR ${MediaStore.Audio.Media.DATA} LIKE '%/Downloads/%')"
        val sortOrder = "${MediaStore.Audio.Media.DATE_ADDED} DESC"

        try {
            resolver.query(collection, projection, selection, null, sortOrder)?.use { cursor ->
                val idCol = cursor.getColumnIndex(MediaStore.Audio.Media._ID)
                val dataCol = cursor.getColumnIndex(MediaStore.Audio.Media.DATA)
                val titleCol = cursor.getColumnIndex(MediaStore.Audio.Media.TITLE)
                val artistCol = cursor.getColumnIndex(MediaStore.Audio.Media.ARTIST)
                val albumCol = cursor.getColumnIndex(MediaStore.Audio.Media.ALBUM)
                val durCol = cursor.getColumnIndex(MediaStore.Audio.Media.DURATION)
                val trackCol = cursor.getColumnIndex(MediaStore.Audio.Media.TRACK)
                val yearCol = cursor.getColumnIndex(MediaStore.Audio.Media.YEAR)
                val sizeCol = cursor.getColumnIndex(MediaStore.Audio.Media.SIZE)
                val mimeCol = cursor.getColumnIndex(MediaStore.Audio.Media.MIME_TYPE)
                val albumIdCol = cursor.getColumnIndex(MediaStore.Audio.Media.ALBUM_ID)

                while (cursor.moveToNext()) {
                    val id = if (idCol != -1) cursor.getLong(idCol) else 0L
                    val path = if (dataCol != -1) cursor.getString(dataCol) else ""
                    val title = if (titleCol != -1) cursor.getString(titleCol) else ""
                    val artist = if (artistCol != -1) cursor.getString(artistCol) else ""
                    val album = if (albumCol != -1) cursor.getString(albumCol) else ""
                    val durationMs = if (durCol != -1) cursor.getLong(durCol) else 0L
                    val trackNum = if (trackCol != -1) cursor.getInt(trackCol) else 0
                    val year = if (yearCol != -1) cursor.getInt(yearCol) else 0
                    val size = if (sizeCol != -1) cursor.getLong(sizeCol) else 0L
                    val mimeType = if (mimeCol != -1) cursor.getString(mimeCol) else ""
                    val albumId = if (albumIdCol != -1) cursor.getLong(albumIdCol) else -1L

                    val artUri = if (albumId > 0) {
                        val albumArtUri = Uri.parse("content://media/external/audio/albumart")
                        ContentUris.withAppendedId(albumArtUri, albumId).toString()
                    } else ""

                    val obj = JSONObject().apply {
                        put("id", id)
                        put("path", path ?: "")
                        put("title", title ?: "")
                        put("artist", if (artist != "<unknown>") artist else "")
                        put("album", if (album != "<unknown>") album else "")
                        put("duration_ms", durationMs)
                        put("track_number", if (trackNum > 0) trackNum else JSONObject.NULL)
                        put("year", if (year > 0) year else JSONObject.NULL)
                        put("size", size)
                        put("mime_type", mimeType ?: "")
                        put("art_uri", artUri)
                    }
                    jsonArray.put(obj)
                }
            }
        } catch (e: Exception) {
            android.util.Log.e("MediaStoreScanner", "Error querying MediaStore audio", e)
        }

        return jsonArray.toString()
    }
}
