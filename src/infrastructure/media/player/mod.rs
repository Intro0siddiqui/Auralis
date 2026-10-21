//! Audio Player
//!
//! Audio playback using rodio (0.22+).

use rodio::{mixer::Mixer, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// Test-only slot holding the start observer. See the field's doc comment.
#[cfg(test)]
type StartObserverSlot = Arc<std::sync::Mutex<Option<Arc<dyn Fn(Option<usize>) + Send + Sync>>>>;

/// Audio player using rodio
#[derive(Clone)]
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
    /// How many [`AudioPlayer::pause`] calls this player has served.
    ///
    /// rodio cannot answer "why is this sink paused". `Player::is_paused`
    /// reads one `AtomicBool` **per `Player`** — read-from-source, rodio
    /// 0.22.2 `src/player.rs`: `pause: AtomicBool::new(false)` in `new()`
    /// (line 89), stored only by `pause()` (line 265) and by `clear()`
    /// (line 283), which this crate never calls. So for a sink we built, and
    /// did not pause ourselves, "is paused" and "a `pause` command arrived"
    /// are *the same statement*, and the resume log could not tell them apart.
    /// That is what made a `never_playing (empty=0)` verdict — a live sink
    /// that is paused — read for three releases as "rodio never started the
    /// fresh sink", when in fact something paused it a moment after the
    /// resume did.
    ///
    /// Monotonic, so a probe can snapshot it when it arms and compare on every
    /// tick without a lock, and a pause that landed *before* the resume armed
    /// is correctly invisible to that resume.
    pause_epoch: Arc<AtomicU64>,
    /// When the most recent [`AudioPlayer::pause`] was served. Paired with
    /// `pause_epoch`: the counter says *that* a pause happened, this says
    /// *when*, and a resume needs the offset to tell a user tapping again from
    /// an audio-focus pause nobody asked for. Kept out of the `AtomicU64`
    /// because `Instant` is 16 bytes and cannot be packed into one.
    last_pause_at: Arc<std::sync::Mutex<Option<Instant>>>,
    /// When the most recent seek occurred. Used to protect against false
    /// buffer underruns, false track-end triggers, and premature focus abandonment
    /// while rodio flushes its decode pipeline.
    last_seek_at: Arc<std::sync::Mutex<Option<Instant>>>,
    /// Whether the most recent [`AudioPlayer::seek`] had to replace the sink
    /// rather than seek the live one. Read-and-cleared by the caller via
    /// [`AudioPlayer::take_seek_rebuilt_sink`].
    last_seek_rebuilt_sink: Arc<AtomicBool>,
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

// SAFETY: `AudioPlayer` is a bag of `Arc<RwLock<_>>` / `Arc<Mutex<_>>` /
// `Arc<Atomic*>` around `Send` primitives (`Duration`, `Track`, `Player`,
// `bool`, `u64`, etc.)
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
/// Reconcile three duration claims about one file: the library's, the
/// decoder's, and the container's sample table.
///
/// The container is authoritative whenever it is known: `total_duration()`
/// reads a container header, and for the MP4s YouTube serves (fragmented,
/// and muxed 360p progressives among them) it can stop at the first fragment
/// it manages to parse — measured on `yF9nmg_jHNs` (2026-09-29, dev box),
/// whose 216.34 s table decoded as 75 s by `total_duration()` and 54.4 s by
/// a full sample walk. A decoder that claims less than the table miscounted,
/// so the table wins in both directions: it corrects a stale-long library
/// value *down* and an unscanned/zero one *up*.
///
/// Without a container reading, the legacy rule holds: the decoder may only
/// ever *raise* the library value, never lower it. Before that rule, a >5 s
/// disagreement overwrote the library value with the decoder's, which
/// shortened the progress bar, capped seeking at the wrong point and made
/// the player report a long track as over.
fn reconcile_duration(
    db: Duration,
    decoded: Option<Duration>,
    container: Option<Duration>,
) -> Duration {
    if let Some(table) = container.filter(|c| !c.is_zero()) {
        return table;
    }
    match decoded {
        Some(dec) if !db.is_zero() => db.max(dec),
        Some(dec) => dec,
        None => db,
    }
}

/// The container's own duration claim for a file, off the async runtime.
///
/// `inspect_container` walks MP4 box structure (unparsable containers yield
/// no table), so this is `None` for anything but a readable MP4 — in which
/// case the caller keeps its existing answer. Only invoked when the library
/// and decoder already disagree, so the common path pays nothing.
async fn container_table_secs(path: &str) -> Option<Duration> {
    let owned = std::path::PathBuf::from(path);
    tokio::task::spawn_blocking(move || {
        super::forensics::inspect_container(&owned)
            .table_secs
            .filter(|t| *t > 0.0)
            .map(Duration::from_secs_f64)
    })
    .await
    .ok()
    .flatten()
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
            pause_epoch: Arc::new(AtomicU64::new(0)),
            last_pause_at: Arc::new(std::sync::Mutex::new(None)),
            last_seek_at: Arc::new(std::sync::Mutex::new(None)),
            last_seek_rebuilt_sink: Arc::new(AtomicBool::new(false)),
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
        // recorded. The decoder may only ever *raise* it, never lower it —
        // unless the container's own sample table is consulted (below), which
        // is authoritative in both directions: see `reconcile_duration`.
        let decoded = source.total_duration();
        let mut duration = reconcile_duration(library_duration, decoded, None);
        let disagree_secs = decoded.is_some_and(|dec_dur| {
            (dec_dur.as_secs() as i64 - library_duration.as_secs() as i64).unsigned_abs() > 5
        });
        if disagree_secs {
            // The two cheap signals disagree by more than the warn threshold,
            // and one of them is wrong in a way `max()` cannot settle: a stale
            // library value can be too *long* as well as too short. Ask the
            // sample table, which is the same oracle the download gate trusts.
            // Rare path (only these MP4s disagree), off the runtime thread,
            // and a parse failure keeps today's answer — never a worse one.
            if let Some(table) = container_table_secs(path).await {
                duration = reconcile_duration(library_duration, decoded, Some(table));
            }
        }
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
    /// its `Track`, and without a container arbitration `reconcile_duration`
    /// never lowers a value — so the extra guard would change no byte. With
    /// one, stamping is the whole point: the decoder is the only thing that
    /// can raise a placeholder duration (and the container the only thing that
    /// can correct a stale one), and the queue is a copy that has to follow
    /// or the UI disagrees with the player bar.
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

    /// Pause the current sink, and record that a pause command was served.
    ///
    /// The counter is the whole point of the method being here rather than at
    /// the call site: rodio's pause flag is per-`Player` and has no cause
    /// attached (see [`AudioPlayer::pause_epoch`]), so a resume that is
    /// silently undone a moment later is otherwise indistinguishable from a
    /// sink rodio refused to start. Bumped **after** the flag is applied, so
    /// "the epoch moved" means "the sink is paused by this call".
    ///
    /// It records *that* a pause was served, not *who* asked: the in-app
    /// button, the frontend's `PlayerController.pause()`, the notification and
    /// the Android audio-focus path all arrive here and are not told apart.
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
        // Instant first, counter second. The two are read separately by the
        // probe, and this order is what makes the pair consistent: a reader
        // that sees the new counter is necessarily reading after the instant
        // was stored, so the offset it reports is *this* pause's. The other
        // order hands out the previous pause's timestamp against the new
        // counter — a plausible-looking but wrong number, which is worse than
        // no number.
        {
            let mut at = self
                .last_pause_at
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *at = Some(Instant::now());
        }
        self.pause_epoch.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// How many [`AudioPlayer::pause`] calls this player has served.
    ///
    /// A caller that wants to know whether a sink went quiet *because of a
    /// pause* snapshots this, does its work, and compares. Equal values prove
    /// nothing paused the sink in between — which for a sink this crate built
    /// and did not pause itself, is the interesting case, because rodio leaves
    /// no other trace of a start having been undone.
    pub fn pause_epoch(&self) -> u64 {
        self.pause_epoch.load(Ordering::SeqCst)
    }

    /// When the most recent [`AudioPlayer::pause`] was served, if any.
    ///
    /// Poison-tolerant for the same reason as `resume_log()`: a panic inside
    /// the critical section must not make every later pause in the process
    /// unwrap-fail, and the only thing stored here is a copyable `Instant`.
    pub fn last_pause_at(&self) -> Option<Instant> {
        *self
            .last_pause_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    ///
    /// Whether the seek had to *replace* the sink is recorded and readable with
    /// [`AudioPlayer::take_seek_rebuilt_sink`]. The fallback builds a brand-new
    /// player and reuses `was_paused` to decide whether to pause it, so a seek
    /// taken while the sink is paused installs a **new, paused, never-played**
    /// sink — the exact shape the resume log was reading as "rodio never
    /// started it". The seek itself returns `Ok` either way, which is why
    /// `restored=ok` on its own was never evidence of anything.
    pub async fn seek(&self, position: Duration) -> Result<(), PlayerError> {
        debug!(?position, "Seeking to position");
        // Cleared on entry so a caller that reads the flag after a *failed*
        // seek is not handed the previous seek's answer.
        self.last_seek_rebuilt_sink.store(false, Ordering::SeqCst);

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
        eprintln!(
            "AURALIS SEEK: pos={:?}, res={:?}, was_paused={}",
            position, res, was_paused
        );

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
                // Only now: the fallback can itself fail (unreadable file, no
                // output device), and in that case nothing was replaced. Set
                // before the `?`-free early return so a failed replacement
                // never reads as a completed one.
                self.last_seek_rebuilt_sink.store(true, Ordering::SeqCst);
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
        *self.play_started_at.write().await = Some(
            Instant::now()
                .checked_sub(position)
                .unwrap_or_else(Instant::now),
        );
        *self.last_seek_at.lock().unwrap() = Some(Instant::now());

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
        } else {
            player.play();
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
        *self.play_started_at.write().await = Some(
            Instant::now()
                .checked_sub(position)
                .unwrap_or_else(Instant::now),
        );
        *self.last_seek_at.lock().unwrap() = Some(Instant::now());

        Ok(())
    }

    /// Read and clear the flag left by the most recent [`AudioPlayer::seek`]:
    /// did that seek have to build a replacement sink?
    ///
    /// Read-and-clear so two consecutive seeks cannot be confused for one, and
    /// so a caller that never asks pays nothing.
    pub fn take_seek_rebuilt_sink(&self) -> bool {
        self.last_seek_rebuilt_sink.swap(false, Ordering::SeqCst)
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

    /// Whether a seek occurred within the last 1500ms.
    pub fn is_recently_seeking(&self) -> bool {
        self.last_seek_at
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map(|t| t.elapsed() < Duration::from_millis(1500))
            .unwrap_or(false)
    }

    pub async fn is_playing(&self) -> bool {
        self.play_anchor.read().await.is_some()
            && self
                .sink
                .read()
                .await
                .as_ref()
                .map(|s| !s.is_paused())
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
        let has_anchor = self.play_anchor.read().await.is_some();
        let guard = self.sink.read().await;
        match guard.as_ref() {
            Some(s) => (has_anchor && !s.is_paused(), s.empty()),
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
mod tests;
