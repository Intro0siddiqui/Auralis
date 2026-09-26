//! Audio Player
//!
//! Audio playback using rodio (0.22+).

use rodio::{mixer::Mixer, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::opus::OpusSource;
use crate::domain::models::{RepeatMode, Track};
use rand::seq::{IndexedRandom, SliceRandom};
use rand::RngExt;

/// Holds the lazily-opened audio output stream.
struct OutputStreamHolder {
    // We keep the MixerDeviceSink alive here. Dropping it stops the audio output.
    stream: Option<MixerDeviceSink>,
}

// SAFETY: `MixerDeviceSink` wraps an `Arc` to the cpal/ALSA/CoreAudio output
// stream. rodio's sink is `Send` but not `Sync`; we wrap it in a
// `std::sync::Mutex` and document the following invariants:
//
// 1. Every access to the inner `MixerDeviceSink` is through a short,
//    synchronous `Mutex::lock()` critical section that never holds the guard
//    across an `.await` point (see `output_stream_sync`). This prevents the
//    non-`Sync` interior from being shared concurrently.
// 2. The sink is opened lazily on first `play()` and then never moved between
//    threads except via the `Arc<Mutex<_>>` — the underlying OS handle is
//    thread-safe for the operations we perform (`mixer().clone()` only).
// 3. `OutputStreamHolder` is only `Send + Sync` because `Mutex<T>` is `Sync`
//    when `T: Send`; the `MixerDeviceSink` itself is `Send`.
//
// If rodio ever makes `MixerDeviceSink: !Send`, this impl must be removed and
// audio I/O confined to a dedicated thread via a channel.
unsafe impl Send for OutputStreamHolder {}
unsafe impl Sync for OutputStreamHolder {}

/// Audio player using rodio
#[derive(Clone)]
/// Test-only slot holding the start observer. See the field's doc comment.
#[cfg(test)]
type StartObserverSlot = Arc<std::sync::Mutex<Option<Arc<dyn Fn(Option<usize>) + Send + Sync>>>>;

pub struct AudioPlayer {
    output: Arc<std::sync::Mutex<OutputStreamHolder>>,
    sink: Arc<RwLock<Option<Player>>>,
    volume: Arc<RwLock<f32>>,
    current_track: Arc<RwLock<Option<Track>>>,
    queue: Arc<RwLock<Vec<Track>>>,
    current_index: Arc<RwLock<Option<usize>>>,
    repeat_mode: Arc<RwLock<RepeatMode>>,
    shuffle_enabled: Arc<RwLock<bool>>,
    /// History stack for shuffle playback (visited indices in order, last is current).
    /// Used to implement exhaust-on-RepeatOff and previous-with-history.
    shuffle_history: Arc<RwLock<Vec<usize>>>,
    /// Playback time accumulated before the current play session.
    played: Arc<RwLock<Duration>>,
    /// `Some` while the sink is actively playing.
    play_anchor: Arc<RwLock<Option<Instant>>>,
    /// When the current playback session started.
    play_started_at: Arc<RwLock<Option<Instant>>>,
    track_duration: Arc<RwLock<Duration>>,
    /// Test-only observer, called with the queue index in effect at the moment a
    /// start is attempted. `next` / `previous` have to move the index *before*
    /// starting so the commit's duration repair lands on the incoming track (see
    /// `start_at_index`), and that ordering is otherwise unobservable headlessly:
    /// a successful start needs an audio output device, and a failed one leaves
    /// no stamp behind to inspect.
    // Behind a `Mutex` rather than a bare `Option` so a test can install it
    // through `&self` — `AudioPlayer` is only ever used behind a shared
    // reference (Tauri's `State<'_, AudioPlayer>`), so a seam needing `&mut`
    // would not be reachable from a realistic test setup.
    #[cfg(test)]
    start_observer: StartObserverSlot,
}

// SAFETY: `AudioPlayer` is a bag of `Arc<RwLock<_>>` / `Arc<Mutex<_>>`
// around `Send` primitives (`Duration`, `Track`, `Player`, `bool`, etc.)
// and the `OutputStreamHolder` above, which is itself documented as
// `Send + Sync` under the invariants noted there. All interior state is
// behind `Arc` + synchronization primitives, so sharing `&AudioPlayer`
// across threads (as Tauri's `State` requires) is sound. No `&mut self`
// aliasing is exposed.
//
// The one non-`Arc` field, `start_observer`, is `#[cfg(test)]` and so absent
// from every build this impl actually governs; it is `Send + Sync` by its own
// trait bounds regardless. Keep that true if it is ever promoted to a real field.
unsafe impl Send for AudioPlayer {}
unsafe impl Sync for AudioPlayer {}

/// Reconcile the library's recorded duration with what a decoder reports.
///
/// The decoder's number is a **lower bound, not an authority**. rodio's
/// `total_duration()` reads a container header, and for the MP4s YouTube serves
/// (fragmented, and muxed 360p progressives among them) it can stop at the first
/// fragment it manages to parse: a 4:26 track came back as 1:32. The library
/// value comes from the container's own sample table and is the same number the
/// download gate verifies before saving a file, so when the two disagree the
/// container wins and the decoder is only allowed to add information by
/// claiming *more*.
///
/// Before this, a >5s disagreement overwrote the library value with the
/// decoder's, which shortened the progress bar, capped seeking at the wrong
/// point and made the player report a long track as over.
fn reconcile_duration(db: Duration, decoded: Option<Duration>) -> Duration {
    match decoded {
        Some(dec) if !db.is_zero() => db.max(dec),
        Some(dec) => dec,
        None => db,
    }
}

/// A start that rodio has accepted: a sink exists and holds the decoded source.
///
/// Produced by [`AudioPlayer::start_sink`] (which can fail) and turned into
/// player state by [`AudioPlayer::commit_start`] (which cannot). Keeping the
/// two apart is what makes starting a track transactional: the identity of what
/// is playing is published only after audio is actually running, so a missing
/// file or an unavailable output device no longer leaves the player describing a
/// track that never started.
struct EstablishedStart {
    /// Length to publish for the new track: the library's claim reconciled with
    /// the decoder's (see [`reconcile_duration`]).
    duration: Duration,
    /// The decoder's own claim, kept so the commit can tell "the library said so"
    /// from "the decoder had something to say". The mirrored copies are re-stamped
    /// whenever the decoder spoke at all — see [`AudioPlayer::commit_start`],
    /// which documents why that is not the same thing as "the decoder corrected
    /// it", and why the distinction does not change the published value.
    decoded: Option<Duration>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, PlayerError> {
        info!("Initializing audio player (output stream opened lazily)");
        Ok(Self {
            output: Arc::new(std::sync::Mutex::new(OutputStreamHolder { stream: None })),
            sink: Arc::new(RwLock::new(None)),
            volume: Arc::new(RwLock::new(0.8)),
            current_track: Arc::new(RwLock::new(None)),
            queue: Arc::new(RwLock::new(Vec::new())),
            current_index: Arc::new(RwLock::new(None)),
            repeat_mode: Arc::new(RwLock::new(RepeatMode::Off)),
            shuffle_enabled: Arc::new(RwLock::new(false)),
            shuffle_history: Arc::new(RwLock::new(Vec::new())),
            played: Arc::new(RwLock::new(Duration::ZERO)),
            play_anchor: Arc::new(RwLock::new(None)),
            play_started_at: Arc::new(RwLock::new(None)),
            track_duration: Arc::new(RwLock::new(Duration::ZERO)),
            #[cfg(test)]
            start_observer: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    fn output_stream_sync(&self) -> Result<Mixer, PlayerError> {
        let mut guard = self.output.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(device_sink) = guard.stream.as_ref() {
            // Clone the Mixer reference so it can be sent across async boundaries
            return Ok(device_sink.mixer().clone());
        }
        let device_sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| PlayerError::InitError(e.to_string()))?;
        let mixer = device_sink.mixer().clone();
        guard.stream = Some(device_sink);
        Ok(mixer)
    }

    async fn output_stream_handle(&self) -> Result<Mixer, PlayerError> {
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY: Duration = Duration::from_millis(500);
        for attempt in 1..=MAX_ATTEMPTS {
            match self.output_stream_sync() {
                Ok(mixer) => return Ok(mixer),
                Err(e) if attempt == MAX_ATTEMPTS => return Err(e),
                Err(e) => {
                    warn!(attempt, error = %e, "Audio output unavailable; retrying");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
        Err(PlayerError::InitError("audio output unavailable".into()))
    }

    /// Start a bare file path, publishing the new duration only once playback is
    /// established. Prefer [`AudioPlayer::play_track`], which also publishes the
    /// track identity.
    ///
    /// With no `Track` of its own, the length the decoder is reconciled against
    /// is whatever the player already published — the caller is expected to have
    /// set it (or to use `play_track`, which does).
    pub async fn play(&self, path: &str) -> Result<(), PlayerError> {
        let library_duration = *self.track_duration.read().await;
        let start = self.start_sink(path, library_duration).await?;
        self.commit_start(None, &start).await;
        Ok(())
    }

    /// Do the fallible half of a start: stop, open, decode, and hand the source
    /// to a fresh rodio player. Returns only once playback is established.
    ///
    /// Nothing here publishes track identity. Every rejection this can produce —
    /// missing file, undecodable file, no audio output device — happens before
    /// `current_track` / `track_duration` / the queue are touched, so the
    /// caller can commit or, on `Err`, leave the player exactly as it was.
    ///
    /// `library_duration` is passed in rather than read from `track_duration`
    /// because at this point `track_duration` still describes the *previous*
    /// track: reconciling the incoming file's decoder against that is how a
    /// 1:32 file would inherit a 4:26 track's length.
    async fn start_sink(
        &self,
        path: &str,
        library_duration: Duration,
    ) -> Result<EstablishedStart, PlayerError> {
        info!(path = %path, "Starting playback");

        // Tearing the previous sink down first is deliberate and long-standing:
        // the playback watcher's end-of-track detection is `sink.empty()`, so a
        // sink that outlived a failed start would keep auto-advance firing
        // against the old track. `stop()` clears playback position only — it
        // never touches track identity — so a failure below still leaves the
        // player describing the previous track.
        self.stop().await?;

        let vol = *self.volume.read().await;
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                // Android scoped-storage fallback: try to resolve a MediaStore
                // Download/Auralis entry via ContentResolver into a cache copy.
                #[cfg(target_os = "android")]
                {
                    if let Some(cached) =
                        crate::infrastructure::media::android_downloads::cached_copy_for_path(path)
                    {
                        File::open(&cached).map_err(|ce| {
                            PlayerError::FileError(format!("{path}: {e} (cached {cached:?}: {ce})"))
                        })?
                    } else {
                        return Err(PlayerError::FileError(format!("{path}: {e}")));
                    }
                }
                #[cfg(not(target_os = "android"))]
                {
                    return Err(PlayerError::FileError(format!("{path}: {e}")));
                }
            }
        };
        let source = create_decoder(file, path)?;

        // Reconcile the duration the decoder reports with the one the library
        // recorded. The decoder may only ever *raise* it, never lower it: see
        // `reconcile_duration` for why.
        let decoded = source.total_duration();
        let duration = reconcile_duration(library_duration, decoded);
        if let Some(dec_dur) = decoded {
            let db_secs = library_duration.as_secs();
            let dec_secs = dec_dur.as_secs();
            if (dec_secs as i64 - db_secs as i64).unsigned_abs() > 5 {
                warn!(
                    path = %path,
                    db_secs = db_secs,
                    dec_secs = dec_secs,
                    kept_secs = duration.as_secs(),
                    "Decoder and library disagree on the track length; keeping the container's claim. \
                     rodio's total_duration() under-reports some MP4s (it stops at the first \
                     fragment it can parse), and trusting it here is what made a 4:26 track \
                     display as 1:32 and become unseekable past that point"
                );
            }
        }

        let mixer = self.output_stream_handle().await?;
        let player = Player::connect_new(&mixer);

        player.set_volume(vol);
        player.append(source);

        *self.sink.write().await = Some(player);
        self.mark_playing().await;

        debug!(path = %path, "Playback started");
        Ok(EstablishedStart { duration, decoded })
    }

    /// The commit point of a start: publish a [`EstablishedStart`] that
    /// `start_sink` has already proven, and only that.
    ///
    /// `track` is `None` for a bare `play(path)`, which has no identity to
    /// publish and so may only refresh the length of what is already current.
    ///
    /// The mirrored copies (`current_track.duration_secs` and the queue entry at
    /// `current_index`) are re-stamped whenever the decoder had *any* opinion.
    /// That is deliberately not "only when the decoder corrected the library":
    /// without an opinion `duration` is the library's value, which is exactly
    /// what both copies already hold — `play_track` receives the queue entry as
    /// its `Track`, and `reconcile_duration` never lowers a value — so the extra
    /// guard would change no byte. With one, stamping is the whole point: the
    /// decoder is the only thing that can raise a placeholder duration, and the
    /// queue is a copy that has to follow or the UI disagrees with the player
    /// bar.
    ///
    /// The queue entry is re-stamped at whatever `current_index` names *at this
    /// moment*, which is why every caller moves the index before starting the
    /// track: [`AudioPlayer::start_at_index`] for `next` / `previous` and
    /// `commands::playback::play` for a direct play. Pointing at the outgoing
    /// track instead re-stamped the track just left with the incoming track's
    /// length, and left the track now playing without its own repair.
    async fn commit_start(&self, track: Option<Track>, start: &EstablishedStart) {
        let mirror_secs = start.decoded.map(|_| start.duration.as_secs() as u32);

        *self.track_duration.write().await = start.duration;

        if let Some(mut track) = track {
            if let Some(secs) = mirror_secs {
                track.duration_secs = secs;
            }
            *self.current_track.write().await = Some(track);
        } else if let Some(secs) = mirror_secs {
            // Guards are scoped to their block: no `.await` runs while one is
            // held.
            let mut current_track = self.current_track.write().await;
            if let Some(current) = current_track.as_mut() {
                current.duration_secs = secs;
            }
        }

        if let Some(secs) = mirror_secs {
            // Read the index into a local first: a temporary guard in an
            // `if let` scrutinee would live to the end of the block, holding it
            // across the `queue` write below.
            let index = *self.current_index.read().await;
            if let Some(index) = index {
                let mut queue = self.queue.write().await;
                if let Some(entry) = queue.get_mut(index) {
                    entry.duration_secs = secs;
                }
            }
        }
    }

    /// Start `track`, publishing the new state only once rodio has accepted it.
    ///
    /// The ordering is the point: the fallible work runs to completion first, so
    /// a missing or undecodable file returns `Err` with `current_track`,
    /// `track_duration` and the queue still describing the previous track, rather
    /// than leaving the player bar pointing at a song that never played.
    ///
    /// The queue index is not written here. Its caller owns it, and it owns it
    /// *before* calling: the commit inside this method re-stamps the entry at
    /// `current_index`, so that index has to name the incoming track by then. See
    /// `start_at_index`, which is how `next` / `previous` do it.
    pub async fn play_track(&self, track: Track) -> Result<(), PlayerError> {
        info!(track_id = %track.id, title = %track.title, "Starting track playback");
        let library_duration = Duration::from_secs(track.duration_secs as u64);
        let start = self.start_sink(&track.file_path, library_duration).await?;
        self.commit_start(Some(track), &start).await;
        Ok(())
    }

    /// Point the queue at `idx` and start `track` there. Used by `next` and
    /// `previous`, which is where a transition's index is decided.
    ///
    /// The index moves **before** the start, because the commit that closes a
    /// successful start re-stamps a decoder-repaired duration onto the entry at
    /// `current_index`, and that must be the incoming track. Setting it
    /// afterwards meant the stamp landed on the outgoing track's entry on every
    /// ordinary transition (auto-advance included): the track just left was
    /// re-stamped with the new track's length, and the track now playing never
    /// received its own repair in the queue at all.
    ///
    /// `previous_index` is the caller's already-read copy of the outgoing index,
    /// passed in rather than re-read here so the rollback cannot pick up a value
    /// somebody else wrote while the start was in flight. `play_track` is
    /// transactional — on `Err` it has published nothing, so `current_track` still
    /// describes the previous track and leaving the index forward would highlight
    /// one entry in the queue while the player bar shows another. Both the
    /// forward and the wrapping case (`next` off the end, `previous` off the
    /// start) take this same path, so the rollback is the only thing that
    /// distinguishes them.
    async fn start_at_index(
        &self,
        idx: usize,
        previous_index: Option<usize>,
        track: &Track,
    ) -> Result<(), PlayerError> {
        *self.current_index.write().await = Some(idx);

        // Test-only: the index as the start sees it, immediately before
        // `play_track` — see the `start_observer` field.
        #[cfg(test)]
        {
            let index = *self.current_index.read().await;
            // Clone the observer out and drop the guard before calling it: the
            // closure locks its own `Mutex`, and holding two locks across a
            // callback is how a test deadlocks instead of failing.
            let observe = self
                .start_observer
                .lock()
                .ok()
                .and_then(|guard| guard.clone());
            if let Some(observe) = observe {
                observe(index);
            }
        }

        if let Err(e) = self.play_track(track.clone()).await {
            *self.current_index.write().await = previous_index;
            return Err(e);
        }
        Ok(())
    }

    pub async fn pause(&self) -> Result<(), PlayerError> {
        debug!("Pausing playback");
        if let Some(s) = self.sink.read().await.as_ref() {
            s.pause();
        }
        let elapsed = self
            .play_anchor
            .read()
            .await
            .map(|a| a.elapsed())
            .unwrap_or_default();
        if !elapsed.is_zero() {
            *self.played.write().await += elapsed;
        }
        *self.play_anchor.write().await = None;
        Ok(())
    }

    /// Un-pause playback, or report that there is nothing left to resume.
    ///
    /// rodio's `Player::play()` is documented as "resumes playback of a paused
    /// player. No effect if not paused", so a resume can silently do nothing.
    /// Two states reach that branch in practice:
    ///
    /// * there is no sink at all — `stop()` takes it out of `self.sink` but
    ///   leaves `current_track` set, so the frontend still believes a track is
    ///   loaded and keeps asking to resume; and
    /// * the sink is drained — a source that played to the end leaves
    ///   `Player::empty()` true forever, and `play()` cannot re-queue it.
    ///
    /// Both used to return `Ok(())`. The caller then treated the resume as
    /// done, drew a pause button over silence, and every later press took the
    /// same dead path until the app was restarted. Report the failure so the
    /// caller replays the track instead.
    pub async fn resume(&self) -> Result<(), PlayerError> {
        debug!("Resuming playback");
        // Read the anchor before taking the sink guard so no lock guard is held
        // across an `.await`.
        let anchor_some = self.play_anchor.read().await.is_some();
        {
            let sink_guard = self.sink.read().await;
            let s = match sink_guard.as_ref() {
                Some(s) => s,
                None => {
                    return Err(PlayerError::StateError(
                        "nothing to resume: playback is not active".into(),
                    ));
                }
            };
            if s.empty() {
                return Err(PlayerError::StateError(
                    "nothing to resume: the track already finished".into(),
                ));
            }
            // Guard against double resume: if already playing (anchor Some && !is_paused), no-op.
            if anchor_some && !s.is_paused() {
                return Ok(());
            }
            s.play();
        }
        // Discard elapsed while paused — do not fold stale anchor into `played`.
        *self.play_anchor.write().await = Some(Instant::now());
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), PlayerError> {
        debug!("Stopping playback");
        if let Some(s) = self.sink.write().await.take() {
            s.stop();
        }
        *self.played.write().await = Duration::ZERO;
        *self.play_anchor.write().await = None;
        *self.play_started_at.write().await = None;
        Ok(())
    }

    /// Seek using rodio's native `try_seek` with fallback for unseekable MP3 bitstreams.
    pub async fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        debug!(?position, "Seeking to position");

        let total = *self.track_duration.read().await;
        if !total.is_zero() && position > total {
            return Err(PlayerError::StateError(
                "Seek position exceeds track duration".into(),
            ));
        }

        let current_track = self.current_track.read().await.clone();

        let seek_res = {
            let sink_guard = self.sink.read().await;
            if let Some(player) = sink_guard.as_ref() {
                let was_paused = player.is_paused();
                let res = player.try_seek(position);
                (res, was_paused)
            } else {
                return Err(PlayerError::StateError("No active playback".into()));
            }
        };

        let (res, was_paused) = seek_res;

        if let Err(e) = res {
            warn!(
                ?position,
                error = %e,
                "Native player seek failed (e.g. unseekable VBR MP3); attempting fallback seek"
            );
            if let Some(track) = current_track {
                if let Err(fallback_err) = self.seek_fallback(&track, position, was_paused).await {
                    warn!(
                        error = %fallback_err,
                        "Fallback seek failed; preserving playback state"
                    );
                    return Err(PlayerError::StateError(format!(
                        "Seek failed: {e} (fallback error: {fallback_err})"
                    )));
                }
                info!(?position, "Fallback seek completed successfully");
                return Ok(());
            } else {
                return Err(PlayerError::StateError(format!("Seek failed: {e}")));
            }
        }

        *self.played.write().await = position;
        if was_paused {
            *self.play_anchor.write().await = None;
        } else {
            *self.play_anchor.write().await = Some(Instant::now());
        }

        info!(?position, "Seek completed");
        Ok(())
    }

    /// Fallback seeking when Symphonia cannot perform frame-accurate timestamp seeks directly on the active decoder.
    /// Re-opens the track, creates a fresh decoder, attempts `skip_duration` or `try_seek`, and replaces the active player.
    async fn seek_fallback(
        &self,
        track: &Track,
        position: Duration,
        was_paused: bool,
    ) -> Result<(), PlayerError> {
        let file_path = &track.file_path;
        let file = match File::open(file_path) {
            Ok(f) => f,
            Err(e) => {
                #[cfg(target_os = "android")]
                {
                    if let Some(cached) =
                        crate::infrastructure::media::android_downloads::cached_copy_for_path(
                            file_path,
                        )
                    {
                        File::open(&cached).map_err(|ce| {
                            PlayerError::FileError(format!(
                                "{file_path}: {e} (cached {cached:?}: {ce})"
                            ))
                        })?
                    } else {
                        return Err(PlayerError::FileError(format!("{file_path}: {e}")));
                    }
                }
                #[cfg(not(target_os = "android"))]
                {
                    return Err(PlayerError::FileError(format!("{file_path}: {e}")));
                }
            }
        };

        let source = create_decoder(file, file_path)?;

        let skipped_source = source.skip_duration(position);

        let vol = *self.volume.read().await;
        let mixer = self.output_stream_handle().await?;
        let player = Player::connect_new(&mixer);
        player.set_volume(vol);
        player.append(skipped_source);

        if was_paused {
            player.pause();
        }

        if let Some(old_sink) = self.sink.write().await.replace(player) {
            old_sink.stop();
        }

        *self.played.write().await = position;
        if was_paused {
            *self.play_anchor.write().await = None;
        } else {
            *self.play_anchor.write().await = Some(Instant::now());
        }

        Ok(())
    }

    pub async fn set_volume(&self, volume: f32) -> Result<(), PlayerError> {
        let volume = volume.clamp(0.0, 1.0);
        *self.volume.write().await = volume;
        if let Some(s) = self.sink.read().await.as_ref() {
            s.set_volume(volume);
        }
        Ok(())
    }

    pub async fn next(&self) -> Result<Option<Track>, PlayerError> {
        self.next_internal(false).await
    }

    /// Auto-advance variant used by the playback watcher on natural track end.
    /// Honors `RepeatOne` by repeating the current track; manual `next()` always advances.
    pub async fn next_for_auto_advance(&self) -> Result<Option<Track>, PlayerError> {
        self.next_internal(true).await
    }

    /// Calculates the next track index when shuffle mode is enabled.
    async fn next_shuffle_index(
        &self,
        queue_len: usize,
        current_idx: Option<usize>,
        repeat: RepeatMode,
    ) -> Option<usize> {
        if queue_len == 1 {
            return match (current_idx, repeat) {
                (Some(_), RepeatMode::Off) => None,
                _ => Some(0),
            };
        }

        if repeat == RepeatMode::Off {
            // Shuffle + RepeatOff: exhaust when all tracks have been visited.
            let history = self.shuffle_history.read().await;
            let mut visited_set: std::collections::HashSet<usize> =
                history.iter().copied().collect();
            if let Some(ci) = current_idx {
                visited_set.insert(ci);
            }

            if visited_set.len() >= queue_len {
                return None;
            }

            let mut rng = rand::rng();
            let unvisited: Vec<usize> = (0..queue_len)
                .filter(|i| !visited_set.contains(i))
                .collect();

            if let Some(&idx) = unvisited.choose(&mut rng) {
                Some(idx)
            } else {
                let other_candidates: Vec<usize> =
                    (0..queue_len).filter(|&i| Some(i) != current_idx).collect();
                Some(
                    other_candidates
                        .choose(&mut rng)
                        .copied()
                        .unwrap_or_else(|| rng.random_range(0..queue_len)),
                )
            }
        } else {
            // Shuffle + RepeatAll (or RepeatOne manual): pick random != current
            let mut rng = rand::rng();
            let candidates: Vec<usize> =
                (0..queue_len).filter(|&i| Some(i) != current_idx).collect();
            Some(
                candidates
                    .choose(&mut rng)
                    .copied()
                    .unwrap_or_else(|| rng.random_range(0..queue_len)),
            )
        }
    }

    /// Maintains shuffle history stack (visited track indices) for previous and exhaustion checking.
    async fn record_shuffle_history(&self, current_idx: Option<usize>, next_idx: usize) {
        let mut hist = self.shuffle_history.write().await;
        if let Some(ci) = current_idx {
            if !hist.contains(&ci) {
                hist.push(ci);
            }
        }
        if hist.last().copied() != Some(next_idx) {
            hist.push(next_idx);
        }
    }

    async fn next_internal(&self, for_auto_advance: bool) -> Result<Option<Track>, PlayerError> {
        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat = *self.repeat_mode.read().await;
        let shuffle = *self.shuffle_enabled.read().await;

        let next_index = if for_auto_advance && repeat == RepeatMode::One {
            current_idx.or(Some(0))
        } else if shuffle {
            self.next_shuffle_index(queue.len(), current_idx, repeat)
                .await
        } else {
            match (current_idx, repeat) {
                (Some(idx), RepeatMode::All) if idx + 1 >= queue.len() => Some(0),
                (Some(idx), RepeatMode::Off) if idx + 1 >= queue.len() => None,
                (Some(idx), RepeatMode::One) if idx + 1 >= queue.len() => None,
                (Some(idx), _) => Some(idx + 1),
                (None, _) => Some(0),
            }
        };

        match next_index {
            Some(idx) => {
                let track = queue[idx].clone();
                drop(queue);
                // Index first, rolled back on a failed start — the commit's
                // duration repair has to land on this track's entry.
                self.start_at_index(idx, current_idx, &track).await?;
                // Maintain shuffle history (visited stack) for exhaust/previous
                if shuffle {
                    self.record_shuffle_history(current_idx, idx).await;
                }
                Ok(Some(track))
            }
            None => {
                drop(queue);
                // Clear shuffle history on exhaustion (session complete)
                if shuffle {
                    self.shuffle_history.write().await.clear();
                }
                self.stop().await?;
                Ok(None)
            }
        }
    }

    pub async fn previous(&self) -> Result<Option<Track>, PlayerError> {
        let queue = self.queue.read().await;
        if queue.is_empty() {
            return Ok(None);
        }

        let current_idx = *self.current_index.read().await;
        let repeat = *self.repeat_mode.read().await;
        let shuffle = *self.shuffle_enabled.read().await;

        let prev_index = if repeat == RepeatMode::One {
            current_idx
        } else if shuffle {
            if queue.len() == 1 {
                Some(0)
            } else {
                // Use history stack to go back: pop current, return previous.
                let mut hist = self.shuffle_history.write().await;
                if hist.len() > 1 {
                    // Last is current, pop it and return new last
                    hist.pop();
                    let prev = hist.last().copied();
                    // Keep hist pointing at prev (do not push again)
                    prev
                } else if hist.len() == 1 && hist[0] != current_idx.unwrap_or(usize::MAX) {
                    // History has single entry not matching current (edge), use it
                    let prev = hist[0];
                    // hist already at prev
                    Some(prev)
                } else {
                    // No history to pop — fallback to random distinct
                    drop(hist);
                    let mut rng = rand::rng();
                    let mut candidates: Vec<usize> = (0..queue.len()).collect();
                    candidates.shuffle(&mut rng);
                    let idx = candidates
                        .into_iter()
                        .find(|&i| Some(i) != current_idx)
                        .unwrap_or_else(|| rng.random_range(0..queue.len()));
                    Some(idx)
                }
            }
        } else {
            match (current_idx, repeat) {
                (Some(0), RepeatMode::All) => Some(queue.len() - 1),
                (Some(0), RepeatMode::Off) => Some(0),
                (Some(idx), _) => Some(idx - 1),
                (None, _) => Some(0),
            }
        };

        match prev_index {
            Some(idx) => {
                let track = queue[idx].clone();
                drop(queue);
                // Same ordering as `next`: index first so the commit's duration
                // repair lands on this track's entry, rolled back if it fails.
                self.start_at_index(idx, current_idx, &track).await?;
                Ok(Some(track))
            }
            None => Ok(None),
        }
    }

    pub async fn is_playing(&self) -> bool {
        self.sink
            .read()
            .await
            .as_ref()
            .map(|s| !s.is_paused() && !s.empty())
            .unwrap_or(false)
    }

    pub async fn current_position(&self) -> Duration {
        let played = *self.played.read().await;
        let live = self
            .play_anchor
            .read()
            .await
            .map(|a| a.elapsed())
            .unwrap_or_default();
        let total = played + live;
        let duration = *self.track_duration.read().await;
        if duration.is_zero() {
            total
        } else {
            total.min(duration)
        }
    }

    pub async fn is_sink_empty(&self) -> bool {
        self.sink
            .read()
            .await
            .as_ref()
            .map(|s| s.empty())
            .unwrap_or(true)
    }

    /// Atomically snapshot `(is_playing, is_empty)` under a single `RwLock`
    /// read guard to avoid the TOCTOU race between separate
    /// `is_playing()` + `is_sink_empty()` calls (the sink could transition
    /// between the two awaits).
    pub async fn sink_snapshot(&self) -> (bool, bool) {
        let guard = self.sink.read().await;
        match guard.as_ref() {
            Some(s) => (!s.is_paused() && !s.empty(), s.empty()),
            None => (false, true),
        }
    }

    pub async fn play_started_elapsed(&self) -> Option<Duration> {
        self.play_started_at.read().await.map(|s| s.elapsed())
    }

    async fn mark_playing(&self) {
        let now = Instant::now();
        *self.played.write().await = Duration::ZERO;
        *self.play_anchor.write().await = Some(now);
        *self.play_started_at.write().await = Some(now);
    }

    pub async fn duration(&self) -> Duration {
        *self.track_duration.read().await
    }
    pub async fn get_volume(&self) -> f32 {
        *self.volume.read().await
    }
    pub async fn get_current_track(&self) -> Option<Track> {
        self.current_track.read().await.clone()
    }
    pub async fn get_queue(&self) -> Vec<Track> {
        self.queue.read().await.clone()
    }
    pub async fn set_queue(&self, tracks: Vec<Track>) {
        *self.queue.write().await = tracks;
        // Shuffle session invalidated by queue change
        self.shuffle_history.write().await.clear();
    }
    pub async fn add_to_queue(&self, track: Track) {
        let new_len = {
            let mut q = self.queue.write().await;
            q.push(track);
            q.len()
        };
        // Retain shuffle session; only invalidate out-of-bounds indices.
        self.shuffle_history.write().await.retain(|i| *i < new_len);
    }

    pub async fn play_next(&self, track: Track) {
        let current_idx = *self.current_index.read().await;
        let new_len = {
            let mut q = self.queue.write().await;
            if q.is_empty() {
                q.push(track);
                1
            } else {
                let insert_idx = current_idx.map(|i| i + 1).unwrap_or(0).min(q.len());
                q.insert(insert_idx, track);
                q.len()
            }
        };
        self.shuffle_history.write().await.retain(|i| *i < new_len);
    }

    pub async fn remove_from_queue(&self, index: usize) -> Result<Track, PlayerError> {
        let mut queue = self.queue.write().await;
        if index >= queue.len() {
            return Err(PlayerError::StateError("Queue index out of bounds".into()));
        }
        let track = queue.remove(index);
        if let Some(current) = *self.current_index.read().await {
            if index < current {
                *self.current_index.write().await = Some(current - 1);
            } else if index == current {
                *self.current_index.write().await = if current < queue.len() {
                    Some(current)
                } else {
                    None
                };
            }
        }
        drop(queue);
        self.shuffle_history.write().await.clear();
        Ok(track)
    }

    pub async fn clear_queue(&self) {
        self.queue.write().await.clear();
        *self.current_index.write().await = None;
        self.shuffle_history.write().await.clear();
    }

    pub async fn get_current_index(&self) -> Option<usize> {
        *self.current_index.read().await
    }
    pub async fn set_current_index(&self, index: Option<usize>) {
        *self.current_index.write().await = index;
    }
    pub async fn get_repeat_mode(&self) -> RepeatMode {
        *self.repeat_mode.read().await
    }
    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        *self.repeat_mode.write().await = mode;
        self.shuffle_history.write().await.clear();
    }
    pub async fn get_shuffle(&self) -> bool {
        *self.shuffle_enabled.read().await
    }
    pub async fn set_shuffle(&self, enabled: bool) {
        *self.shuffle_enabled.write().await = enabled;
        let current = *self.current_index.read().await;
        let mut hist = self.shuffle_history.write().await;
        hist.clear();
        // Seed history with current index when shuffle is enabled so previous has a base
        if enabled {
            if let Some(idx) = current {
                hist.push(idx);
            }
        }
    }
}

/// Unified audio source supporting standard Rodio decoders and Opus/WebM decoders.
pub enum DecodedAudioSource {
    Rodio(Decoder<BufReader<File>>),
    Opus(Box<OpusSource>),
}

impl Iterator for DecodedAudioSource {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Rodio(s) => s.next(),
            Self::Opus(s) => s.next(),
        }
    }
}

impl Source for DecodedAudioSource {
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        match self {
            Self::Rodio(s) => s.current_span_len(),
            Self::Opus(s) => s.current_span_len(),
        }
    }

    #[inline]
    fn channels(&self) -> NonZeroU16 {
        match self {
            Self::Rodio(s) => s.channels(),
            Self::Opus(s) => s.channels(),
        }
    }

    #[inline]
    fn sample_rate(&self) -> NonZeroU32 {
        match self {
            Self::Rodio(s) => s.sample_rate(),
            Self::Opus(s) => s.sample_rate(),
        }
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        match self {
            Self::Rodio(s) => s.total_duration(),
            Self::Opus(s) => s.total_duration(),
        }
    }

    #[inline]
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        match self {
            Self::Rodio(s) => s.try_seek(pos),
            Self::Opus(s) => s.try_seek(pos),
        }
    }
}

/// Context passed to decoder strategies containing file metadata and container sniffing state.
struct DecoderContext<'a> {
    file: &'a mut File,
    path: &'a str,
    ext: String,
    is_ebml: bool,
}

impl<'a> DecoderContext<'a> {
    fn new(file: &'a mut File, path: &'a str) -> Self {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();

        let mut header = [0u8; 4];
        let is_ebml = file.read(&mut header).unwrap_or(0) == 4 && &header == b"\x1a\x45\xdf\xa3";
        let _ = file.seek(SeekFrom::Start(0));

        Self {
            file,
            path,
            ext,
            is_ebml,
        }
    }
}

/// A strategy for constructing an audio decoder.
trait DecoderStrategy {
    fn try_decode(
        &self,
        ctx: &mut DecoderContext,
    ) -> Result<Option<DecodedAudioSource>, PlayerError>;
}

/// Fast-path decoder strategy for WebM/Opus container streams (EBML header or .webm/.opus extension).
struct OpusContainerStrategy;

impl DecoderStrategy for OpusContainerStrategy {
    fn try_decode(
        &self,
        ctx: &mut DecoderContext,
    ) -> Result<Option<DecodedAudioSource>, PlayerError> {
        if ctx.is_ebml || ctx.ext == "webm" || ctx.ext == "opus" {
            if let Ok(cloned_file) = ctx.file.try_clone() {
                if let Ok(opus_src) = OpusSource::new(cloned_file, ctx.path) {
                    return Ok(Some(DecodedAudioSource::Opus(Box::new(opus_src))));
                }
            }
        }
        Ok(None)
    }
}

/// Decoder strategy using Rodio with format extension hinting.
struct RodioHintedStrategy;

impl DecoderStrategy for RodioHintedStrategy {
    fn try_decode(
        &self,
        ctx: &mut DecoderContext,
    ) -> Result<Option<DecodedAudioSource>, PlayerError> {
        if !ctx.ext.is_empty() {
            if let Ok(cloned_file) = ctx.file.try_clone() {
                let reader = BufReader::with_capacity(64 * 1024, cloned_file);
                match Decoder::builder()
                    .with_data(reader)
                    .with_hint(&ctx.ext)
                    .build()
                {
                    Ok(decoder) => return Ok(Some(DecodedAudioSource::Rodio(decoder))),
                    Err(e) => {
                        warn!(
                            path = %ctx.path,
                            ext = %ctx.ext,
                            error = %e,
                            "Extension-hinted decoder build failed; attempting default Decoder::new"
                        );
                    }
                }
            }
        }
        Ok(None)
    }
}

/// Decoder strategy using default unhinted Rodio format probing.
struct RodioDefaultStrategy;

impl DecoderStrategy for RodioDefaultStrategy {
    fn try_decode(
        &self,
        ctx: &mut DecoderContext,
    ) -> Result<Option<DecodedAudioSource>, PlayerError> {
        let _ = ctx.file.seek(SeekFrom::Start(0));
        if let Ok(cloned_file) = ctx.file.try_clone() {
            let reader = BufReader::with_capacity(64 * 1024, cloned_file);
            if let Ok(decoder) = Decoder::new(reader) {
                return Ok(Some(DecodedAudioSource::Rodio(decoder)));
            }
        }
        Ok(None)
    }
}

/// Fallback decoder strategy for Opus streams (handles WebM/Opus mislabeled with other extensions like .m4a/.mp3).
struct OpusFallbackStrategy;

impl DecoderStrategy for OpusFallbackStrategy {
    fn try_decode(
        &self,
        ctx: &mut DecoderContext,
    ) -> Result<Option<DecodedAudioSource>, PlayerError> {
        let _ = ctx.file.seek(SeekFrom::Start(0));
        if let Ok(cloned_file) = ctx.file.try_clone() {
            match OpusSource::new(cloned_file, ctx.path) {
                Ok(opus_src) => Ok(Some(DecodedAudioSource::Opus(Box::new(opus_src)))),
                Err(e) => Err(PlayerError::DecodeError(format!(
                    "Failed to decode audio file {}: {e}",
                    ctx.path
                ))),
            }
        } else {
            Err(PlayerError::DecodeError(format!(
                "Failed to decode audio file {}: failed to clone file handle",
                ctx.path
            )))
        }
    }
}

/// Helper to construct an audio decoder with extension hinting, Rodio fallback, and native Opus/WebM decoding.
fn create_decoder(mut file: File, path: &str) -> Result<DecodedAudioSource, PlayerError> {
    let mut ctx = DecoderContext::new(&mut file, path);

    let strategies: &[&dyn DecoderStrategy] = &[
        &OpusContainerStrategy,
        &RodioHintedStrategy,
        &RodioDefaultStrategy,
        &OpusFallbackStrategy,
    ];

    for strategy in strategies {
        if let Some(source) = strategy.try_decode(&mut ctx)? {
            return Ok(source);
        }
    }

    Err(PlayerError::DecodeError(format!(
        "Failed to decode audio file {path}: no decoder strategy succeeded"
    )))
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("Initialization error: {0}")]
    InitError(String),
    #[error("Player error: {0}")]
    SinkError(String),
    #[error("File error: {0}")]
    FileError(String),
    #[error("Decode error: {0}")]
    DecodeError(String),
    #[error("State error: {0}")]
    StateError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::AudioFormat;

    /// Publish the state a successful `play_track` leaves behind: track 0 of a
    /// two-track queue is current, with a 4:26 length.
    async fn seed_previous_state(player: &AudioPlayer) -> Track {
        let current = Track::new(
            "Previous".to_string(),
            "/library/previous.mp3".to_string(),
            266,
            AudioFormat::Mp3,
        );
        let queued = Track::new(
            "Queued".to_string(),
            "/library/queued.mp3".to_string(),
            180,
            AudioFormat::Mp3,
        );
        player.set_queue(vec![current.clone(), queued]).await;
        player.set_current_index(Some(0)).await;
        *player.current_track.write().await = Some(current.clone());
        *player.track_duration.write().await = Duration::from_secs(266);
        current
    }

    #[test]
    fn reconcile_never_shortens_a_known_duration() {
        // The real case: container says 4:26, rodio says 1:32.
        let db = Duration::from_secs(266);
        let decoded = Duration::from_secs(92);
        assert_eq!(reconcile_duration(db, Some(decoded)), db);
    }

    #[test]
    fn reconcile_uses_the_decoder_when_the_library_has_nothing() {
        assert_eq!(
            reconcile_duration(Duration::ZERO, Some(Duration::from_secs(92))),
            Duration::from_secs(92)
        );
    }

    #[test]
    fn reconcile_raises_the_duration_when_the_decoder_claims_more() {
        // A library row with a placeholder duration must still be corrected.
        assert_eq!(
            reconcile_duration(Duration::from_secs(30), Some(Duration::from_secs(92))),
            Duration::from_secs(92)
        );
    }

    #[test]
    fn reconcile_keeps_the_library_value_without_a_decoder_opinion() {
        assert_eq!(
            reconcile_duration(Duration::from_secs(266), None),
            Duration::from_secs(266)
        );
    }

    /// Headless-safe: `AudioPlayer::new()` opens no device (the output stream is
    /// lazy), so a fresh player has `sink == None`. `resume()` must not report
    /// success there, or the frontend waits forever for audio that never starts.
    #[tokio::test]
    async fn resume_without_a_sink_reports_an_error() {
        let player = AudioPlayer::new().unwrap();
        let err = player
            .resume()
            .await
            .expect_err("resume must not report success with no sink");
        assert!(
            matches!(&err, PlayerError::StateError(_)),
            "expected a StateError, got {err:?}"
        );
        let message = err.to_string();
        assert!(
            message.contains("nothing to resume"),
            "unexpected error message: {message}"
        );
        assert!(!player.is_playing().await);
    }

    /// `stop()` takes the sink out of the player but leaves `current_track`
    /// set — the exact state that used to turn every later resume into a silent
    /// no-op that reported success ("cannot play anything until restart").
    #[tokio::test]
    async fn resume_after_stop_reports_an_error() {
        let player = AudioPlayer::new().unwrap();
        player.stop().await.unwrap();
        let err = player
            .resume()
            .await
            .expect_err("resume must not report success after stop");
        assert!(
            matches!(&err, PlayerError::StateError(_)),
            "expected a StateError, got {err:?}"
        );
        let message = err.to_string();
        assert!(
            message.contains("nothing to resume"),
            "unexpected error message: {message}"
        );
        assert!(!player.is_playing().await);
    }

    /// Headless-safe: a missing file is rejected by `File::open`, long before the
    /// output device is touched, so the whole "failed start" contract is
    /// exercisable with no audio hardware.
    ///
    /// This is the regression being pinned. `play_track` used to publish
    /// `current_track` and `track_duration` *before* doing the fallible work, so a
    /// failure returned `Err` while the player kept claiming the track that never
    /// started — the player bar pointed at a song that was not playing, and
    /// "next" walked on from a bogus position.
    #[tokio::test]
    async fn play_track_that_cannot_start_keeps_the_previous_track() {
        let player = AudioPlayer::new().unwrap();
        let previous = seed_previous_state(&player).await;
        let duration_before = player.duration().await;

        let incoming = Track::new(
            "Never Plays".to_string(),
            "/nonexistent/auralis/never-plays.mp3".to_string(),
            92,
            AudioFormat::Mp3,
        );
        let err = player
            .play_track(incoming)
            .await
            .expect_err("a missing file must not report success");
        assert!(
            matches!(&err, PlayerError::FileError(_)),
            "expected a FileError, got {err:?}"
        );

        let current = player
            .get_current_track()
            .await
            .expect("a failed start must not clear the current track");
        assert_eq!(
            current.id, previous.id,
            "current_track moved to a track that never started"
        );
        assert_eq!(current.title, previous.title);
        assert_eq!(
            current.duration_secs, previous.duration_secs,
            "the current track's own length was rewritten by a failed start"
        );
        assert_eq!(
            player.duration().await,
            duration_before,
            "track_duration was overwritten by a failed start"
        );
        assert_eq!(
            player.get_current_index().await,
            Some(0),
            "the queue index must not move on a failed start"
        );
        let queue = player.get_queue().await;
        assert_eq!(queue[0].id, previous.id);
        assert_eq!(
            queue[0].duration_secs, 266,
            "the queue entry was re-stamped by a failed start"
        );
        assert!(
            !player.is_playing().await,
            "a failed start must not leave a sink behind"
        );
    }

    /// The same guarantee at the *second* rejection point: the file is there, so
    /// `File::open` succeeds, and `create_decoder` is what refuses it. Pins that
    /// the rollback is structural (nothing is written until playback exists) and
    /// not just an early `return` on the open.
    #[tokio::test]
    async fn play_track_on_an_undecodable_file_keeps_the_previous_track() {
        let dir = std::env::temp_dir().join(format!("auralis_test_play_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let corrupt = dir.join("corrupt.m4a");
        std::fs::write(&corrupt, b"NOT_A_REAL_AUDIO_FILE").unwrap();

        let player = AudioPlayer::new().unwrap();
        let previous = seed_previous_state(&player).await;
        let duration_before = player.duration().await;

        let incoming = Track::new(
            "Corrupt".to_string(),
            corrupt.to_str().unwrap_or_default().to_string(),
            120,
            AudioFormat::M4a,
        );
        let err = player
            .play_track(incoming)
            .await
            .expect_err("an undecodable file must not report success");
        assert!(
            matches!(&err, PlayerError::DecodeError(_)),
            "expected a DecodeError, got {err:?}"
        );

        assert_eq!(
            player.get_current_track().await.map(|t| t.id),
            Some(previous.id),
            "current_track moved to a track that never started"
        );
        assert_eq!(player.duration().await, duration_before);
        assert_eq!(player.get_queue().await[0].duration_secs, 266);
        assert!(!player.is_playing().await);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The commit half, driven directly.
    ///
    /// A real `start_sink` needs a rodio output device, so the *successful* start
    /// path cannot be exercised headlessly and this test does not claim to: it
    /// proves that the commit publishes every field `play_track` owns, in one
    /// place, from an `EstablishedStart` and nothing else.
    #[tokio::test]
    async fn commit_start_publishes_the_established_track() {
        let player = AudioPlayer::new().unwrap();
        let incoming = Track::new(
            "Incoming".to_string(),
            "/library/incoming.mp3".to_string(),
            30,
            AudioFormat::Mp3,
        );
        player.set_queue(vec![incoming.clone()]).await;
        player.set_current_index(Some(0)).await;

        // A payload `start_sink` would have produced for a 30s library row whose
        // decoder claimed 92s: `reconcile_duration` keeps the larger value and the
        // commit re-stamps every published copy with it.
        let start = EstablishedStart {
            duration: Duration::from_secs(92),
            decoded: Some(Duration::from_secs(92)),
        };
        player.commit_start(Some(incoming.clone()), &start).await;

        assert_eq!(player.duration().await, Duration::from_secs(92));
        let current = player
            .get_current_track()
            .await
            .expect("the commit must publish the track");
        assert_eq!(current.id, incoming.id);
        assert_eq!(current.duration_secs, 92);
        let queue = player.get_queue().await;
        assert_eq!(
            queue[0].duration_secs, 92,
            "the queue entry at current_index must be re-stamped with the established length"
        );
    }

    /// With no decoder opinion there is nothing new to say about the length, so
    /// the commit must not reach outside the track it was given. The queue entry
    /// at `current_index` is a *copy* that can be some other track entirely — a
    /// bare `play(path)` has no queue identity at all — and restamping it here
    /// would be a write nobody asked for.
    ///
    /// This test seeds the index as the previous track's rather than the
    /// incoming one's, which is now a caller bug rather than a `next` /
    /// `previous` state; it is kept because the guard it pins is exactly the
    /// protection a bare-path commit needs. The ordering that keeps `next` /
    /// `previous` out of this situation is pinned separately, by
    /// `next_moves_the_queue_index_before_starting_the_track` and its two
    /// siblings.
    #[tokio::test]
    async fn commit_start_without_a_decoder_opinion_leaves_the_queue_entry_alone() {
        let player = AudioPlayer::new().unwrap();
        let previous = seed_previous_state(&player).await;
        let incoming = Track::new(
            "Incoming".to_string(),
            "/library/incoming.mp3".to_string(),
            42,
            AudioFormat::Mp3,
        );

        let start = EstablishedStart {
            duration: Duration::from_secs(42),
            decoded: None,
        };
        player.commit_start(Some(incoming.clone()), &start).await;

        assert_eq!(player.duration().await, Duration::from_secs(42));
        let current = player
            .get_current_track()
            .await
            .expect("the commit must publish the track");
        assert_eq!(current.id, incoming.id);
        assert_eq!(
            current.duration_secs, 42,
            "with no decoder opinion the published track keeps the library's own length"
        );
        let queue = player.get_queue().await;
        assert_eq!(
            queue[0].id, previous.id,
            "the queue itself must be untouched"
        );
        assert_eq!(
            queue[0].duration_secs, 266,
            "the still-current queue entry must keep its own length"
        );
    }

    /// `play(path)` has no `Track` to publish, so its commit may only refresh the
    /// length of what is already current — it must not invent an identity, and it
    /// must not clear one either.
    #[tokio::test]
    async fn commit_start_without_a_track_refreshes_only_the_duration() {
        let player = AudioPlayer::new().unwrap();
        let previous = seed_previous_state(&player).await;

        let start = EstablishedStart {
            duration: Duration::from_secs(300),
            decoded: Some(Duration::from_secs(300)),
        };
        player.commit_start(None, &start).await;

        assert_eq!(player.duration().await, Duration::from_secs(300));
        let current = player
            .get_current_track()
            .await
            .expect("a bare-path commit must not clear the current track");
        assert_eq!(current.id, previous.id);
        assert_eq!(current.duration_secs, 300);
    }

    /// Attach a recorder for the queue index in effect at each start attempt, and
    /// hand back the log.
    ///
    /// This is the only seam that can show which entry a transition's commit
    /// would re-stamp: a *successful* start needs a rodio output device, and a
    /// failed one publishes nothing at all, so neither the committed state nor
    /// the queue tells you what the index was while the start was in flight.
    fn record_start_indices(player: &AudioPlayer) -> Arc<std::sync::Mutex<Vec<Option<usize>>>> {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        *player.start_observer.lock().expect("observer mutex") = Some(Arc::new(move |index| {
            recorder.lock().expect("recorder mutex").push(index);
        }));
        seen
    }

    /// The forward transition, and the regression this pins: `next` used to call
    /// `play_track` first and move the index afterwards, so during the commit the
    /// index still named the *outgoing* track. Its queue entry was re-stamped with
    /// the incoming track's length and the track that ended up playing never
    /// received its own repair — on every ordinary transition, auto-advance
    /// included.
    ///
    /// The start is made to fail (the queued track's path does not exist) so this
    /// runs with no audio hardware; the observer reports the index at the moment
    /// the start was attempted, which is the value the commit would have read.
    /// The rollback is asserted in the same breath, because a failed start that
    /// left the index forward would highlight one entry while the player bar
    /// still showed another.
    #[tokio::test]
    async fn next_moves_the_queue_index_before_starting_the_track() {
        let player = AudioPlayer::new().unwrap();
        seed_previous_state(&player).await;
        let seen = record_start_indices(&player);

        let err = player
            .next_internal(false)
            .await
            .expect_err("the queued track's file does not exist, so the start must fail");
        assert!(
            matches!(&err, PlayerError::FileError(_)),
            "expected a FileError, got {err:?}"
        );

        assert_eq!(
            *seen.lock().unwrap(),
            vec![Some(1)],
            "next must move the queue index to the incoming track before starting it, \
             otherwise the commit re-stamps the outgoing track's entry"
        );
        assert_eq!(
            player.get_current_index().await,
            Some(0),
            "a failed start must leave the index on the track the player still describes"
        );
    }

    /// The same ordering where `next` wraps off the end of the queue to index 0.
    /// Covered separately because the outgoing track is then the *last* entry: the
    /// old ordering stamped the tail of the queue while the track actually
    /// playing was the head, which the forward case cannot show.
    #[tokio::test]
    async fn next_wrapping_to_the_start_moves_the_index_before_starting_the_track() {
        let player = AudioPlayer::new().unwrap();
        seed_previous_state(&player).await;
        player.set_current_index(Some(1)).await;
        player.set_repeat_mode(RepeatMode::All).await;
        let seen = record_start_indices(&player);

        let err = player
            .next_internal(false)
            .await
            .expect_err("the wrapped-to track's file does not exist, so the start must fail");
        assert!(
            matches!(&err, PlayerError::FileError(_)),
            "expected a FileError, got {err:?}"
        );

        assert_eq!(
            *seen.lock().unwrap(),
            vec![Some(0)],
            "a wrapped next must point the index at the wrapped-to track before starting it"
        );
        assert_eq!(
            player.get_current_index().await,
            Some(1),
            "a failed start must leave the index on the track the player still describes"
        );
    }

    /// `previous` carried the identical bug and needs the identical ordering; its
    /// rollback matters as much, since going back to an unplayable track would
    /// otherwise leave the highlight on that track while the player bar showed
    /// the one before it.
    #[tokio::test]
    async fn previous_moves_the_queue_index_before_starting_the_track() {
        let player = AudioPlayer::new().unwrap();
        seed_previous_state(&player).await;
        player.set_current_index(Some(1)).await;
        let seen = record_start_indices(&player);

        let err = player
            .previous()
            .await
            .expect_err("the previous track's file does not exist, so the start must fail");
        assert!(
            matches!(&err, PlayerError::FileError(_)),
            "expected a FileError, got {err:?}"
        );

        assert_eq!(
            *seen.lock().unwrap(),
            vec![Some(0)],
            "previous must move the queue index to the incoming track before starting it, \
             otherwise the commit re-stamps the outgoing track's entry"
        );
        assert_eq!(
            player.get_current_index().await,
            Some(1),
            "a failed start must leave the index on the track the player still describes"
        );
    }

    /// `previous` wrapping from the head of the queue to the tail under
    /// `RepeatAll`, so the target index is neither the current one nor the
    /// neighbouring one. Same ordering, same rollback, and the only remaining
    /// combination of (method × wrap-around) that the three tests above leave
    /// unpinned.
    #[tokio::test]
    async fn previous_wrapping_to_the_end_moves_the_index_before_starting_the_track() {
        let player = AudioPlayer::new().unwrap();
        seed_previous_state(&player).await;
        player.set_repeat_mode(RepeatMode::All).await;
        let seen = record_start_indices(&player);

        let err = player
            .previous()
            .await
            .expect_err("the wrapped-to track's file does not exist, so the start must fail");
        assert!(
            matches!(&err, PlayerError::FileError(_)),
            "expected a FileError, got {err:?}"
        );

        assert_eq!(
            *seen.lock().unwrap(),
            vec![Some(1)],
            "a wrapped previous must point the index at the wrapped-to track before starting it"
        );
        assert_eq!(
            player.get_current_index().await,
            Some(0),
            "a failed start must leave the index on the track the player still describes"
        );
    }

    #[tokio::test]
    async fn test_next_shuffle_index_single_track() {
        let player = AudioPlayer::new().unwrap();
        assert_eq!(
            player.next_shuffle_index(1, Some(0), RepeatMode::Off).await,
            None
        );
        assert_eq!(
            player.next_shuffle_index(1, None, RepeatMode::Off).await,
            Some(0)
        );
        assert_eq!(
            player.next_shuffle_index(1, Some(0), RepeatMode::All).await,
            Some(0)
        );
    }

    #[tokio::test]
    async fn test_next_shuffle_index_repeat_all() {
        let player = AudioPlayer::new().unwrap();
        let idx = player.next_shuffle_index(3, Some(0), RepeatMode::All).await;
        assert!(idx.is_some());
        assert_ne!(idx, Some(0));
        assert!(idx.unwrap() < 3);
    }

    #[tokio::test]
    async fn test_next_shuffle_index_repeat_off_exhaustion() {
        let player = AudioPlayer::new().unwrap();
        {
            let mut hist = player.shuffle_history.write().await;
            hist.push(0);
            hist.push(1);
            hist.push(2);
        }
        assert_eq!(
            player.next_shuffle_index(3, Some(2), RepeatMode::Off).await,
            None
        );
    }

    #[tokio::test]
    async fn test_record_shuffle_history() {
        let player = AudioPlayer::new().unwrap();
        player.record_shuffle_history(Some(0), 1).await;
        {
            let hist = player.shuffle_history.read().await;
            assert_eq!(*hist, vec![0, 1]);
        }

        // Recording the same next index should not duplicate it at the end
        player.record_shuffle_history(Some(1), 1).await;
        {
            let hist = player.shuffle_history.read().await;
            assert_eq!(*hist, vec![0, 1]);
        }

        // Recording a new index
        player.record_shuffle_history(Some(1), 2).await;
        {
            let hist = player.shuffle_history.read().await;
            assert_eq!(*hist, vec![0, 1, 2]);
        }
    }

    #[test]
    fn test_create_decoder_extension_hint_and_fallback() {
        let dir =
            std::env::temp_dir().join(format!("auralis_test_decoder_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);

        // Generate 1s 8000Hz 8-bit mono WAV fixture
        let sample_rate: u32 = 8000;
        let num_samples: u32 = 8000;
        let mut data = Vec::with_capacity(44 + num_samples as usize);
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + num_samples).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes()); // PCM
        data.extend_from_slice(&1u16.to_le_bytes()); // Mono
        data.extend_from_slice(&sample_rate.to_le_bytes());
        data.extend_from_slice(&sample_rate.to_le_bytes()); // Byte rate
        data.extend_from_slice(&1u16.to_le_bytes()); // Block align
        data.extend_from_slice(&8u16.to_le_bytes()); // Bits per sample
        data.extend_from_slice(b"data");
        data.extend_from_slice(&num_samples.to_le_bytes());
        data.resize(44 + num_samples as usize, 0x80);

        let wav_path = dir.join("test_track.wav");
        std::fs::write(&wav_path, &data).unwrap();

        // 1. Test creation with explicit extension hint (.wav)
        let file_wav = File::open(&wav_path).unwrap();
        let dec_wav = create_decoder(file_wav, wav_path.to_str().unwrap());
        assert!(
            dec_wav.is_ok(),
            "Expected create_decoder to succeed with .wav extension hint"
        );

        // 2. Test fallback when extension hint is unknown/custom (.customext)
        let custom_path = dir.join("test_track.customext");
        std::fs::write(&custom_path, &data).unwrap();
        let file_custom = File::open(&custom_path).unwrap();
        let dec_custom = create_decoder(file_custom, custom_path.to_str().unwrap());
        assert!(
            dec_custom.is_ok(),
            "Expected create_decoder to succeed via fallback Decoder::new"
        );

        // 3. Test failure on corrupt audio content
        let corrupt_path = dir.join("corrupt.m4a");
        std::fs::write(&corrupt_path, b"NOT_A_REAL_AUDIO_FILE").unwrap();
        let file_corrupt = File::open(&corrupt_path).unwrap();
        let dec_corrupt = create_decoder(file_corrupt, corrupt_path.to_str().unwrap());
        assert!(
            dec_corrupt.is_err(),
            "Expected corrupt file to fail decoding"
        );

        // 4. Test real WebM Opus file (scratch/sample.m4a)
        let sample_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch/sample.m4a");
        if sample_path.exists() {
            let file_sample = File::open(&sample_path).unwrap();
            let dec_sample = create_decoder(file_sample, sample_path.to_str().unwrap());
            assert!(
                dec_sample.is_ok(),
                "Expected create_decoder to succeed on WebM Opus file"
            );
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}
