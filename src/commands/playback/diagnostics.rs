//! Pause / resume: mitigation + diagnostics
//!
//! Symptom, live since v2.6.16 and the owner's top complaint after downloads:
//! pause, resume, and nothing plays — sometimes with the bar still claiming to
//! be playing.
//!
//! Already ruled out, and deliberately not re-litigated here: the 250 ms watcher,
//! the frontend's 700 ms proof-of-life watch, `start_sink`, auto-advance via
//! `sink.empty()`, and sink replacement. Nothing in our own code runs at 1 ms.
//!
//! THE MITIGATION is a candidate, not a diagnosed fix. The only asymmetry
//! anyone could find in rodio's paused -> playing transition is inside
//! `Pausable`: `set_paused(false)` clears `paused_channels` but leaves
//! `remaining_paused_samples` set, so the first samples after an unpause can
//! still be silence. That predicts a short silent lead-in, not permanent
//! silence, so it may not be the whole story — and no comment here should be
//! read as a root-cause claim until the owner's exact observed symptom is in
//! hand. What the fresh-sink path *does* buy is a decisive next run: a new sink
//! cannot inherit state from the paused one, so if the replay plays and the
//! unpause does not, that transition is the differentiator.
//!
//! THE DIAGNOSTICS exist because the failure is silent in both directions. A
//! successful `resume` is indistinguishable from a resume that produced no
//! audio, and `tracing` goes to stdout, which a release Android build gives the
//! owner no way to read. So every attempt files a report, the watcher judges it
//! against its own ticks, and the report is rendered into the queue panel with a
//! copy button — the same shape as the download row's "Copy report".
//!
//! The six shapes the log is built to separate:
//!
//! | verdict                | meaning                                              |
//! |------------------------|------------------------------------------------------|
//! | `playing_confirmed`    | the sink reported playing; silence here is downstream of our state |
//! | `drained_immediately`  | nothing ever played and the source was already drained |
//! | `played_then_drained`  | audio started and was consumed before the window closed |
//! | `never_playing`        | a live source, no command paused it, and rodio still would not play it |
//! | `paused_by_command`    | a `pause` command landed after this resume armed; see [`attribute_pause`] |
//! | `no_observation`       | the watcher did not sample inside the window (task starved) |
//!
//! `playing_confirmed` versus `never_playing` is the pair that matters: the
//! first clears this code and the driver/`Pausable` transition, the second
//! means the unpause itself did nothing. `progress=N` in the detail line is the
//! number of `playback:progress` events the run really emitted, so "no audio"
//! can be told apart from "no events reached the frontend" without guessing.
//!
//! **`paused_by_command` exists because `never_playing` was not saying what it
//! looked like it said.** The device run of 2026-09-29 recorded
//! `never_playing (polls=7 progress=0 empty=0)`, and for three releases that
//! line was read as "rodio refused to start the fresh sink". It cannot mean
//! that. `start_sink` builds its player with `Player::connect_new` and never
//! calls `pause()` on it, and in rodio 0.22.2 a new `Player` is born unpaused
//! (`src/player.rs:89`, `pause: AtomicBool::new(false)`) whose pause flag has
//! exactly two writers — `Player::pause()` (`src/player.rs:265`) and
//! `Player::clear()` (`src/player.rs:283`), and this crate calls neither on a
//! live sink. So `empty=0` with `progress=0` is not "rodio did nothing": it is
//! "a live sink is sitting paused", and for a sink this crate built and did not
//! pause itself, that means a `pause` command arrived. The log now says that,
//! and says when.
//!
//! It deliberately does **not** name the actor, because the counter cannot: the
//! in-app button, `PlayerController.pause()`, the Android notification /
//! MediaSession and the audio-focus handler all land in the same
//! [`crate::infrastructure::media::AudioPlayer::pause`]. What it does give is the
//! offset from the arm, which is the discriminator — a pause a few hundred
//! milliseconds after the resume is a user tap, and one landing inside the first
//! poll is a focus or MediaSession event nobody touched the screen for.

use crate::commands::library::html_escape;
use crate::domain::models::Track;
use crate::infrastructure::media::AudioPlayer;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use uuid::Uuid;

/// How many resume attempts the log keeps. Matches the download side's
/// `window.__auralisClientReports` (last 20), so the evidence for a failure
/// sits in the same place whichever failure is being chased.
pub const RESUME_LOG_CAPACITY: usize = 20;

/// How many of those the queue panel shows. The copy button carries all of
/// them; on-screen text stays short enough to read on a phone.
pub const RESUME_LOG_VISIBLE: usize = 5;

/// How long the watcher samples a resumed sink before judging it.
///
/// Long enough for several [`super::WATCHER_INTERVAL`] ticks — "never playing" should
/// be several independent samples, not one — and short enough that the log
/// line still describes the moment the user pressed play.
pub const RESUME_PROBE_WINDOW: Duration = Duration::from_millis(1500);

/// How long a rejected resume waits for the frontend's replay to be reported
/// back onto its report. The frontend replays within milliseconds of the
/// rejection arriving.
pub const RESUME_REPLAY_WINDOW: Duration = Duration::from_secs(10);

/// Paused positions shorter than this are not restored after a fresh-sink
/// replay: the seek would cost more than the fraction of a second it saves.
pub const POSITION_RESTORE_FLOOR: Duration = Duration::from_millis(500);

/// The sink is running and kept running: the resume worked as far as our state
/// is concerned.
pub const VERDICT_CONFIRMED: &str = "playing_confirmed";
/// No sample ever saw it playing, and the source was already drained.
pub const VERDICT_DRAINED_IMMEDIATE: &str = "drained_immediately";
/// It played and was consumed inside the window.
pub const VERDICT_PLAYED_THEN_DRAINED: &str = "played_then_drained";
/// A live source is queued, but rodio never left the paused state **and nothing
/// paused it** — see [`attribute_pause`] for why the second half is the only
/// honest reading of the first.
pub const VERDICT_NEVER_PLAYING: &str = "never_playing";
/// A live source is queued and was explicitly paused by a `pause` command that
/// landed after this resume armed. See [`attribute_pause`].
pub const VERDICT_PAUSED_BY_COMMAND: &str = "paused_by_command";
/// The watcher sampled nothing: the task is starved, which is its own bug.
pub const VERDICT_NO_OBSERVATION: &str = "no_observation";
/// A newer resume replaced this one before its window closed.
pub const VERDICT_SUPERSEDED: &str = "superseded";
/// Nothing was resumed (already playing), so there was nothing to watch.
pub const VERDICT_NOT_PROBED: &str = "not_probed";
/// The command itself failed; there is no sink to judge.
pub const VERDICT_FAILED: &str = "failed";

/// Probe verdict type alias representing classification outcomes.
pub type ProbeVerdict = &'static str;

/// `result` values. `pending` is only ever a transient initial value: a report
/// is filed with its result already set, which is why
/// [`note_replay_outcome`] matches on `error` rather than on "not ok".
pub const RESULT_PENDING: &str = "pending";
pub const RESULT_OK: &str = "ok";
pub const RESULT_ERROR: &str = "error";

/// Pre-state labels, from the one atomic `sink_snapshot()`.
pub const PRE_PLAYING: &str = "playing";
pub const PRE_PAUSED: &str = "paused";
/// No sink at all, or a drained one. The two are indistinguishable from
/// outside `AudioPlayer` (`is_sink_empty()` answers `true` for both), so the
/// label does not pretend to know which.
pub const PRE_NO_SOURCE: &str = "no_source";

/// The `onclick` for the log's copy button.
///
/// A const rather than an inline literal so rustfmt is never asked to break it:
/// rustfmt abandons an item containing a line it cannot wrap, and that is how
/// real misformatting hides from `cargo fmt --check`. Single quotes only — the
/// string is interpolated into a double-quoted HTML attribute.
pub const RESUME_LOG_COPY_JS: &str =
    "if(navigator.clipboard){var b=window.Auralis&&window.Auralis.bridge;\
navigator.clipboard.writeText(this.dataset.report)\
.then(function(){b&&b.showToast&&b.showToast('Resume log copied','success');})\
.catch(function(){b&&b.showToast&&b.showToast('Copy failed','error');});}";

pub const RESUME_LOG_OPEN: &str = "<div class=\"resume-log\" style=\"margin-top: var(--space-3); \
border-top: 1px solid var(--glass-border); padding-top: var(--space-3);\">";

pub const RESUME_LOG_TITLE_OPEN: &str =
    "<div style=\"font-size: var(--text-xs); color: var(--text-3); \
text-transform: uppercase; letter-spacing: 0.05em; margin-bottom: var(--space-2); \
font-weight: var(--font-semibold);\">";

pub const RESUME_LOG_TEXT_OPEN: &str =
    "<pre style=\"margin: 0 0 var(--space-2); font-size: var(--text-xs); \
color: var(--text-2); white-space: pre-wrap; word-break: break-word; \
line-height: 1.4;\">";

pub const RESUME_LOG_BUTTON: &str =
    "<button type=\"button\" class=\"btn btn-secondary btn-sm neu\" \
data-action=\"copy-resume-log\" data-report=\"{REPORT}\" onclick=\"{JS}\">Copy resume log</button>";

pub const RESUME_LOG_CLOSE: &str = "</div>";

/// The two facts [`AudioPlayer`] exposes about pausing, sampled once per tick.
///
/// rodio's pause flag is a bare `AtomicBool` with no cause attached, so these
/// two numbers are the *only* way a caller can tell "rodio never started this
/// sink" from "something paused it". Sampled by value rather than read from a
/// lock inside the probe, which is what keeps the probe a pure function.
///
/// The pair must be sampled together, which `AudioPlayer::pause` arranges by
/// storing the instant *before* it bumps the counter: a caller that sees the
/// new counter is necessarily reading after the instant was stored, so the
/// offset it derives belongs to the pause that actually cancelled this resume.
/// The other order hands out the previous pause's timestamp against the new
/// counter — a plausible-looking but wrong number, which is worse than none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PauseSample {
    /// [`AudioPlayer::pause_epoch`]: how many `pause` calls have been served.
    pub epoch: u64,
    /// [`AudioPlayer::last_pause_at`]: when the most recent one was served.
    pub at: Option<Instant>,
}

impl PauseSample {
    pub fn of(player: &AudioPlayer) -> Self {
        Self {
            epoch: player.pause_epoch(),
            at: player.last_pause_at(),
        }
    }
}

/// Which route a resume takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeStrategy {
    /// rodio already reports the sink as playing.
    AlreadyPlaying,
    /// Un-pause the existing sink.
    Unpause,
    /// Build a new sink from the current track and restore the position.
    FreshSinkReplay,
}

impl ResumeStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            ResumeStrategy::AlreadyPlaying => "none",
            ResumeStrategy::Unpause => "unpause",
            ResumeStrategy::FreshSinkReplay => "fresh_sink_replay",
        }
    }
}

/// One `stat` of the file a resume would have to decode: what the log shows,
/// and whether a fresh decode is worth attempting.
pub struct FilePreflight {
    pub note: String,
    pub readable: bool,
}

/// Pick the route for one resume.
///
/// `has_live_source` is `!is_empty` from the atomic snapshot: false covers both
/// "no sink" and "drained", and in both cases `AudioPlayer::resume` is the right
/// caller because it is what reports the `StateError` the frontend already
/// knows how to answer with a replay.
///
/// `file_ready` only ever *withholds* the mitigation. A file that cannot be
/// stat'ed is not proof of an unplayable file — on Android a MediaStore copy can
/// still resolve it (`cached_copy_for_path`) — so the worst a wrong `false` does
/// is fall back to the old behaviour, which is exactly the safe direction.
pub fn choose_resume_strategy(
    is_playing: bool,
    has_live_source: bool,
    file_ready: bool,
) -> ResumeStrategy {
    if is_playing {
        return ResumeStrategy::AlreadyPlaying;
    }
    if !has_live_source {
        return ResumeStrategy::Unpause;
    }
    if file_ready {
        return ResumeStrategy::FreshSinkReplay;
    }
    ResumeStrategy::Unpause
}

pub fn pre_state_str(is_playing: bool, is_empty: bool) -> &'static str {
    if is_playing {
        PRE_PLAYING
    } else if is_empty {
        PRE_NO_SOURCE
    } else {
        PRE_PAUSED
    }
}

/// What one resume attempt did, in enough detail to tell the failure shapes
/// apart. Rendered verbatim into the queue panel, so every field earns its
/// place.
#[derive(Debug, Clone)]
pub struct ResumeReport {
    pub at: String,
    pub track_id: String,
    pub track_id_short: String,
    pub title: String,
    pub strategy: &'static str,
    pub pre_state: &'static str,
    pub result: &'static str,
    /// The error text, when there is one.
    pub detail: String,
    pub file_note: String,
    pub pos_before_secs: f64,
    /// `n/a`, `ok`, or `failed: …` — whether the paused position survived the
    /// replay.
    pub pos_restored: String,
    /// Filled in later by the frontend's replay (`note_replay_outcome`).
    pub replay: String,
    pub probe: &'static str,
    pub probe_detail: String,
}

impl ResumeReport {
    pub fn new(
        track: Option<&Track>,
        strategy: ResumeStrategy,
        pre_state: &'static str,
        file_note: String,
        pos_before_secs: f64,
    ) -> Self {
        let track_id = track.map(|t| t.id.to_string()).unwrap_or_default();
        let track_id_short: String = track_id.chars().take(8).collect();
        let title = track
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "(no track)".to_string());
        Self {
            at: now_iso(),
            track_id,
            track_id_short,
            title,
            strategy: strategy.as_str(),
            pre_state,
            result: RESULT_PENDING,
            detail: String::new(),
            file_note,
            pos_before_secs,
            pos_restored: "n/a".to_string(),
            replay: String::new(),
            probe: VERDICT_NOT_PROBED,
            probe_detail: String::new(),
        }
    }
}

/// A resume in flight, waiting for the watcher to judge it.
pub struct ResumeProbe {
    pub armed_at: Instant,
    pub report: ResumeReport,
    pub polls: u32,
    pub playing_polls: u32,
    pub empty_polls: u32,
    /// An empty sample *after* a playing one: audio started and was consumed,
    /// which is a different failure from never starting.
    pub drained_after_playing: bool,
    /// [`AudioPlayer::pause_epoch`] as it stood when the probe was armed. A
    /// pause that landed *before* the resume is therefore invisible to it,
    /// which is the point: the question this probe answers is "what happened
    /// to the sink this resume built", not "was the sink ever paused".
    pub pause_epoch_at_arm: u64,
}

pub struct ResumeLog {
    pub pending: Option<ResumeProbe>,
    /// Oldest first, capped at [`RESUME_LOG_CAPACITY`]. A `VecDeque` would be
    /// the natural shape, but `VecDeque::new` is not a `const fn` and this is
    /// built in a `static`; at 20 entries the `remove(0)` costs nothing.
    pub reports: Vec<ResumeReport>,
    /// The newest report came from a rejected resume and is still waiting for
    /// the frontend's replay to be reported back onto it.
    pub awaiting_replay: bool,
    pub awaiting_since: Option<Instant>,
}

pub(crate) static RESUME_LOG: Mutex<ResumeLog> = Mutex::new(ResumeLog {
    pending: None,
    reports: Vec::new(),
    awaiting_replay: false,
    awaiting_since: None,
});

/// One resume at a time. See the `resume` command's doc comment for why an
/// overlapping pair is a correctness problem and not just wasted work.
///
/// A `tokio::sync::Mutex` rather than the `std::sync::Mutex` the log uses,
/// because this one is held across the whole command — `play_track` and the
/// position restore are both async, and blocking a runtime worker on a
/// `std::sync::Mutex` is how a `try_seek` (a *blocking* `recv` inside rodio)
/// turns into a stalled worker.
pub static RESUME_INFLIGHT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Poison-tolerant lock: a panic in one resume attempt must not blind every
/// later one, and the state here is plain data with no invariant a panic could
/// half-apply. Same recovery as `AudioPlayer::output_stream_sync`.
pub fn resume_log() -> MutexGuard<'static, ResumeLog> {
    RESUME_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn now_iso() -> String {
    // UTC, matching the download reports' `new Date().toISOString()`.
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub async fn preflight_audio_file(track: Option<&Track>) -> FilePreflight {
    let path = match track {
        Some(t) => t.file_path.as_str(),
        None => {
            return FilePreflight {
                note: "no-track".to_string(),
                readable: false,
            }
        }
    };
    match tokio::fs::metadata(path).await {
        Ok(md) if md.is_file() && md.len() > 0 => {
            let mib = md.len() as f64 / (1024.0 * 1024.0);
            FilePreflight {
                note: format!("ok:{mib:.2}MB"),
                readable: true,
            }
        }
        Ok(md) => {
            let note = if md.is_file() {
                "empty-file"
            } else {
                "not-a-file"
            };
            FilePreflight {
                note: note.to_string(),
                readable: false,
            }
        }
        Err(e) => FilePreflight {
            note: format!("stat-failed:{e}"),
            readable: false,
        },
    }
}

/// Restart the current track on a brand-new sink instead of un-pausing the old
/// one, then put the listener back where they were.
///
/// The position has to be read by the caller *before* this runs:
/// `play_track` → `start_sink` → `mark_playing` zeroes the accumulated position,
/// and `start_sink` also tears the paused sink down before it opens the file —
/// so a file that cannot be opened costs the listener their paused sink. That is
/// why [`choose_resume_strategy`] refuses to take this path without a readable
/// file, and why the position restore is best-effort rather than required: a
/// failed seek still leaves audio playing from the start, which beats silence.
pub async fn replay_on_fresh_sink(
    player: &AudioPlayer,
    track: Option<&Track>,
    pos_before: Duration,
    report: &mut ResumeReport,
) -> Result<(), String> {
    let track = match track {
        Some(t) => t.clone(),
        None => {
            return Err("nothing to resume: no current track is loaded".to_string());
        }
    };
    info!(
        track_id = %track.id,
        file_path = %track.file_path,
        pos_before = pos_before.as_secs_f64(),
        "Rebuilding the sink for resume instead of un-pausing it"
    );
    if let Err(e) = player.play_track(track).await {
        return Err(e.to_string());
    }
    if pos_before < POSITION_RESTORE_FLOOR {
        report.pos_restored = "n/a".to_string();
        return Ok(());
    }
    match player.seek(pos_before).await {
        Ok(()) => {
            // `ok` alone has been the wrong thing to read: when the native
            // seek fails, `seek_fallback` builds a *replacement* player and
            // reuses the sink's paused state, so a restore taken while the
            // fresh sink is paused installs a new sink that has never been
            // played — the very shape the probe then reports as "never
            // playing". The distinction is one `take_seek_rebuilt_sink()` away
            // and it is the difference between "the position was restored on
            // the sink the resume built" and "the position was restored on a
            // different, paused sink".
            report.pos_restored = if player.take_seek_rebuilt_sink() {
                "rebuilt".to_string()
            } else {
                "ok".to_string()
            };
        }
        Err(e) => {
            // Not fatal: playback is running, from the wrong offset. The log
            // says so, because "resume restarted my track" is otherwise
            // indistinguishable from a bug.
            report.pos_restored = format!("failed:{e}");
            warn!(error = %e, "Resume replay could not restore the paused position");
        }
    }
    Ok(())
}

/// Judge a finished probe window. Pure, so the whole table above is testable.
pub fn classify_probe(
    polls: u32,
    playing: u32,
    empty: u32,
    drained_after_playing: bool,
) -> ProbeVerdict {
    if polls == 0 {
        return VERDICT_NO_OBSERVATION;
    }
    if playing == 0 {
        return if empty > 0 {
            VERDICT_DRAINED_IMMEDIATE
        } else {
            VERDICT_NEVER_PLAYING
        };
    }
    if drained_after_playing {
        return VERDICT_PLAYED_THEN_DRAINED;
    }
    VERDICT_CONFIRMED
}

/// Whether a verdict is worth a toast. A confirmed resume, a resume that was
/// never attempted, and one replaced by a later attempt are all quiet: the first
/// two need no news, and the third is about to be reported by the probe that
/// replaced it.
pub fn verdict_is_failure(verdict: &str) -> bool {
    !matches!(
        verdict,
        VERDICT_CONFIRMED | VERDICT_NOT_PROBED | VERDICT_SUPERSEDED
    )
}

pub fn push_resume_report(reports: &mut Vec<ResumeReport>, report: ResumeReport) {
    reports.push(report);
    while reports.len() > RESUME_LOG_CAPACITY {
        reports.remove(0);
    }
}

pub fn file_resume_report(report: ResumeReport) {
    let mut log = resume_log();
    log.awaiting_replay = false;
    push_resume_report(&mut log.reports, report);
}

pub fn file_resume_report_opening_replay(report: ResumeReport) {
    let mut log = resume_log();
    log.awaiting_replay = true;
    log.awaiting_since = Some(Instant::now());
    push_resume_report(&mut log.reports, report);
}

pub fn arm_resume_probe(report: ResumeReport, pause_epoch: u64) {
    let mut log = resume_log();
    if let Some(stale) = log.pending.take() {
        // Two resumes inside one window: the newer attempt is the one the user
        // cares about, but the older must not silently vanish from the log.
        let mut superseded = stale.report;
        superseded.probe = VERDICT_SUPERSEDED;
        superseded.probe_detail = format!("replaced after {} poll(s)", stale.polls);
        push_resume_report(&mut log.reports, superseded);
    }
    log.pending = Some(ResumeProbe {
        armed_at: Instant::now(),
        report,
        polls: 0,
        playing_polls: 0,
        empty_polls: 0,
        drained_after_playing: false,
        pause_epoch_at_arm: pause_epoch,
    });
}

pub fn resume_probe_pending() -> bool {
    resume_log().pending.is_some()
}

/// Feed one watcher tick to the pending probe. Returns the finished report on
/// the tick that closes the window.
///
/// `playing` is also the number of `playback:progress` events the run emitted
/// for this attempt, because the watcher emits that event on exactly this
/// condition — which is what makes "no audio" and "no events reached the
/// frontend" separable from here at all.
///
/// `pause` is this tick's sample of [`AudioPlayer`]'s pause bookkeeping; it is
/// compared against the value the probe was armed with, so a `pause` that
/// arrives mid-window is attributed to the resume it cancelled. Pure
/// arithmetic on values the caller already holds, so the whole thing stays
/// headlessly testable — no audio device is needed to say "this sink was
/// paused by a command".
pub fn observe_resume_probe(
    now: Instant,
    is_playing: bool,
    is_empty: bool,
    pause: PauseSample,
) -> Option<ResumeReport> {
    let mut log = resume_log();
    let probe = log.pending.as_mut()?;
    probe.polls = probe.polls.saturating_add(1);
    if is_playing {
        probe.playing_polls = probe.playing_polls.saturating_add(1);
    }
    if is_empty {
        probe.empty_polls = probe.empty_polls.saturating_add(1);
        if probe.playing_polls > 0 {
            probe.drained_after_playing = true;
        }
    }
    if now.saturating_duration_since(probe.armed_at) < RESUME_PROBE_WINDOW {
        return None;
    }
    let probe = log.pending.take()?;
    let mut report = probe.report;
    report.probe = classify_probe(
        probe.polls,
        probe.playing_polls,
        probe.empty_polls,
        probe.drained_after_playing,
    );
    let paused_by = attribute_pause(probe.armed_at, probe.pause_epoch_at_arm, pause);
    report.probe = apply_pause_attribution(report.probe, paused_by);
    report.probe_detail = format!(
        "polls={} progress={} empty={}{}",
        probe.polls,
        probe.playing_polls,
        probe.empty_polls,
        match paused_by {
            Some(offset) => format!(" paused_cmd=+{}ms", offset.as_millis()),
            None => String::new(),
        }
    );
    log.awaiting_replay = false;
    push_resume_report(&mut log.reports, report.clone());
    Some(report)
}

/// Whether a `pause` command was served between the probe being armed and the
/// tick being judged, and how long after arming it landed.
///
/// The offset is measured from the arming instant, so the report reads "how
/// long into the resume did the cancellation arrive" — the number that
/// separates a user tapping again from an audio-focus or MediaSession pause
/// nobody asked for. `None` means no pause was served in the window, which is
/// the only case in which a paused sink is our own fault.
///
/// The epoch decides and the timestamp only decorates: a pause with no
/// recorded instant (which [`AudioPlayer::pause`] does not produce, but which
/// this code does not assume away) still reports `Some(ZERO)` rather than
/// silently degrading to "nothing paused it", because wrongly blaming rodio is
/// the failure mode this whole mechanism exists to stop.
pub fn attribute_pause(
    armed_at: Instant,
    epoch_at_arm: u64,
    sample: PauseSample,
) -> Option<Duration> {
    if sample.epoch <= epoch_at_arm {
        return None;
    }
    Some(
        sample
            .at
            .map(|at| at.saturating_duration_since(armed_at))
            .unwrap_or(Duration::ZERO),
    )
}

/// Re-attribute a verdict once it is known that a `pause` command landed
/// inside the window.
///
/// Only the verdicts that mean "the sink was there and produced nothing" are
/// rewritten. `playing_confirmed` is left alone (audio ran, whatever a later
/// pause did to it), and so are the two bookkeeping verdicts — `superseded`
/// and `not_probed` are not observations of a sink at all.
pub fn apply_pause_attribution(
    verdict: &'static str,
    paused_by_command: Option<Duration>,
) -> &'static str {
    match (verdict, paused_by_command) {
        (VERDICT_NEVER_PLAYING | VERDICT_DRAINED_IMMEDIATE, Some(_)) => VERDICT_PAUSED_BY_COMMAND,
        _ => verdict,
    }
}

/// Stamp the outcome of the replay the frontend runs after a rejected resume
/// onto that resume's report.
///
/// A no-op unless the newest report is a rejected resume for this same track
/// that is still inside [`RESUME_REPLAY_WINDOW`], so an unrelated play cannot
/// rewrite history.
pub fn note_replay_outcome(track_id: &Uuid, outcome: Result<(), String>) {
    let now = Instant::now();
    let mut log = resume_log();
    if !log.awaiting_replay {
        return;
    }
    let fresh = log
        .awaiting_since
        .is_some_and(|t| now.saturating_duration_since(t) <= RESUME_REPLAY_WINDOW);
    if !fresh {
        log.awaiting_replay = false;
        return;
    }
    let id = track_id.to_string();
    let applies = log
        .reports
        .last()
        .is_some_and(|r| r.result == RESULT_ERROR && r.track_id == id);
    if !applies {
        return;
    }
    if let Some(report) = log.reports.last_mut() {
        report.replay = match outcome {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("failed:{e}"),
        };
    }
    log.awaiting_replay = false;
}

/// The log, newest first.
pub fn resume_log_snapshot() -> Vec<ResumeReport> {
    let log = resume_log();
    log.reports.iter().rev().cloned().collect()
}

/// Test-only reset for the process-wide [`RESUME_LOG`].
///
/// The state is global because the watcher and the commands are separate
/// tasks; the tests serialise on [`RESUME_LOG_TEST_LOCK`] instead of
/// pretending it is per-test.
#[cfg(test)]
pub fn reset_resume_log() {
    let mut log = resume_log();
    log.pending = None;
    log.reports.clear();
    log.awaiting_replay = false;
    log.awaiting_since = None;
}

/// Serialises the tests that touch [`RESUME_LOG`].
#[cfg(test)]
pub static RESUME_LOG_TEST_LOCK: Mutex<()> = Mutex::new(());

/// One log line. Field order is the debugging order: what ran, what the state
/// was, what the file looked like, what the probe concluded, then identity.
pub fn format_resume_report(report: &ResumeReport) -> String {
    let mut line = String::with_capacity(200);
    line.push_str(&report.at);
    line.push(' ');
    line.push_str(report.result);
    line.push_str(&format!(" strategy={}", report.strategy));
    line.push_str(&format!(" pre={}", report.pre_state));
    line.push_str(&format!(" file={}", report.file_note));
    line.push_str(&format!(
        " pos={:.1}s restored={}",
        report.pos_before_secs, report.pos_restored
    ));
    line.push_str(&format!(" probe={}", report.probe));
    if !report.probe_detail.is_empty() {
        line.push_str(&format!(" ({})", report.probe_detail));
    }
    if !report.replay.is_empty() {
        line.push_str(&format!(" replay={}", report.replay));
    }
    if !report.detail.is_empty() {
        line.push_str(&format!(" err={}", report.detail));
    }
    line.push_str(&format!(
        " track=\"{}\" id={}",
        report.title, report.track_id_short
    ));
    line
}

/// The toast text for a resume that was accepted and then produced nothing.
///
/// The `paused_by_command` case gets its own sentence rather than the generic
/// one, because it is the only verdict whose cause is already known: a release
/// build writes `tracing` to stdout and the owner has no logcat, so a toast
/// that says "Resume produced no audio" and stops there has cost three
/// releases of forensics to decode one log line. Naming the cancellation — and
/// saying how long after the resume it arrived — is the difference between a
/// report the owner can act on and one they have to send back.
pub fn resume_failure_toast(report: &ResumeReport) -> String {
    if report.probe == VERDICT_PAUSED_BY_COMMAND {
        return format!(
            "Resume was cancelled: a pause command landed after it ({}). \
             A second tap on the play button, the notification, or an audio-focus pause all \
             arrive here. Details: queue panel > Resume log.",
            report.probe_detail
        );
    }
    format!(
        "Resume produced no audio ({}). Details: queue panel > Resume log.",
        report.probe
    )
}

/// Render the log into the queue panel. `reports` is newest first.
///
/// Empty in, empty out: with no resume attempted yet this must not put a
/// heading on the panel.
pub fn render_resume_log_html(reports: &[ResumeReport]) -> String {
    if reports.is_empty() {
        return String::new();
    }
    let mut html = String::with_capacity(2048);
    html.push_str(RESUME_LOG_OPEN);
    let shown = reports.len().min(RESUME_LOG_VISIBLE);
    html.push_str(RESUME_LOG_TITLE_OPEN);
    html.push_str(&format!("Resume log (newest {shown} of {})", reports.len()));
    html.push_str("</div>");
    html.push_str(RESUME_LOG_TEXT_OPEN);
    for report in reports.iter().take(RESUME_LOG_VISIBLE) {
        html.push_str(&html_escape(&format_resume_report(report)));
        html.push('\n');
    }
    html.push_str("</pre>");
    // The whole log, not just the visible lines: the point of the button is to
    // get the evidence off the device.
    let full: Vec<String> = reports.iter().map(format_resume_report).collect();
    let button = RESUME_LOG_BUTTON
        .replace("{REPORT}", &html_escape(&full.join("\n")))
        .replace("{JS}", &html_escape(RESUME_LOG_COPY_JS));
    html.push_str(&button);
    html.push_str(RESUME_LOG_CLOSE);
    html
}
