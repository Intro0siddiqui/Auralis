//! Byte-level forensics for a downloaded media file.
//!
//! Why this exists
//! ---------------
//! The completeness gate used to trust a decoder's opinion of a file's length
//! (`completeness::verify_decoded_duration`). Real-device evidence (v2.6.44,
//! track `BElct8HWkp8`, residential Jio) showed how badly that can go:
//!
//! ```text
//! [received 21379314 bytes of 21379314 advertised (itag=18,
//!  end_reason=all-advertised-bytes-received)]
//! url+header: HTTP 416 | header: HTTP 416 | url: HTTP 400
//! Truncated download: only 99s of 287s of audio is actually present
//! ```
//!
//! Every byte the server had was on disk, and `416` says the object really does
//! end there. 21.4 MB for a 287 s track is 596 kbps — exactly a muxed 360p
//! progressive — while a genuine 99-second window would be ~7 MB. So the file was
//! *complete* and the decoder was wrong: `total_duration()` reported 99 s for a
//! container whose own sample table describes 287 s.
//!
//! Two independent questions therefore have to be answered separately:
//!
//! 1. **Are all the media bytes present?** The MP4 sample table answers this
//!    exactly, with no decoding: `stco`/`co64` give chunk offsets, `stsc` says
//!    how many samples live in each chunk, `stsz` gives their sizes. The highest
//!    chunk offset plus the size of that chunk's samples is the last byte the
//!    file must contain. A fragmented file answers the same question through its
//!    `sidx`, whose subsegment sizes sum to the whole object.
//! 2. **Is there actually audio there?** Sample tables describe intent, not
//!    content, so the file is additionally decoded end to end to find the last
//!    audible sample. This is what separates "complete file, decoder gave up" from
//!    "complete file, three minutes of silence".
//!
//! Nothing here panics on malformed input: every field is optional and an
//! unreadable structure simply yields [`Verdict::Unknown`], which makes callers
//! fall back to their previous behaviour.

use rodio::{Decoder, Source};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Files larger than this are not inspected: the whole file is read into memory
/// and a phone should not have to hold a 200 MB buffer for a diagnostic.
const MAX_INSPECT_BYTES: u64 = 192 * 1024 * 1024;

/// A decoded sample counts as audible above this magnitude (out of ±32768), so
/// dithering noise and digital silence do not read as "audio present".
const AUDIBLE_THRESHOLD: i16 = 16;

/// What the container structure says about completeness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every byte the container references is present in the file.
    Complete,
    /// The container references bytes past the end of the file.
    Truncated { missing_bytes: u64 },
    /// The structure could not be understood (WebM/Opus, an unexpected layout, a
    /// file too large to inspect, ...). Callers must keep their old behaviour.
    Unknown,
}

/// Structural facts about the audio track of a file.
#[derive(Debug, Clone, Default)]
pub struct ContainerFacts {
    pub size_bytes: u64,
    /// `mp4-stbl` (sample tables), `mp4-fragmented` (`moof`/`sidx`) or `unknown`.
    pub container: &'static str,
    pub has_video_track: bool,
    pub audio_track_found: bool,
    /// Duration the track header claims (`mdhd` / `sidx`).
    pub declared_secs: Option<f64>,
    /// Duration the sample table claims (`stts` / `mdhd`).
    pub table_secs: Option<f64>,
    /// Highest byte offset the audio data must reach for the file to be whole.
    pub audio_data_end: Option<u64>,
    /// Bytes still missing when the file is short.
    pub missing_bytes: Option<u64>,
    pub sample_count: Option<u64>,
    pub chunk_count: Option<u64>,
    pub fragment_count: u64,
    pub verdict: Option<Verdict>,
}

impl ContainerFacts {
    /// One-line summary for an error message or a log line.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("container={}", self.container));
        parts.push(format!("size={}B", self.size_bytes));
        parts.push(format!(
            "audio_track={}",
            if self.audio_track_found { "yes" } else { "no" }
        ));
        if self.has_video_track {
            parts.push("muxed_video=yes".to_string());
        }
        if let Some(secs) = self.table_secs {
            parts.push(format!("table={secs:.1}s"));
        } else if let Some(secs) = self.declared_secs {
            parts.push(format!("declared={secs:.1}s"));
        }
        if let Some(end) = self.audio_data_end {
            parts.push(format!("audio_data_end={end}B"));
        }
        if self.fragment_count > 0 {
            parts.push(format!("fragments={}", self.fragment_count));
        }
        let verdict = match &self.verdict {
            Some(Verdict::Complete) => "bytes=complete".to_string(),
            Some(Verdict::Truncated { missing_bytes }) => format!("bytes=missing {missing_bytes}"),
            _ => "bytes=unknown".to_string(),
        };
        parts.push(verdict);
        parts.join(" ")
    }
}

/// What the audio actually contains once decoded.
#[derive(Debug, Clone, Default)]
pub struct ContentFacts {
    /// Length the decoder reported for the whole file.
    pub decoded_secs: Option<u64>,
    /// Position of the last sample above the audible threshold.
    pub audible_secs: Option<f64>,
    pub sample_rate: u32,
    pub total_samples: u64,
    pub audible_samples: u64,
}

impl ContentFacts {
    pub fn summary(&self) -> String {
        format!(
            "decoded={}s audible_until={}s ({} of {} samples above {AUDIBLE_THRESHOLD}, {} Hz)",
            self.decoded_secs
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".into()),
            self.audible_secs
                .map(|s| format!("{s:.1}"))
                .unwrap_or_else(|| "?".into()),
            self.audible_samples,
            self.total_samples,
            self.sample_rate
        )
    }
}

// ---------------------------------------------------------------------------
// MP4 box walking
// ---------------------------------------------------------------------------

/// A located box: `start`/`end` delimit its *payload*, so children can be walked
/// without re-reading the header.
struct Located {
    kind: [u8; 4],
    start: usize,
    end: usize,
}

impl Located {
    fn is(&self, name: &[u8; 4]) -> bool {
        &self.kind == name
    }
}

/// Boxes whose payload is itself a list of boxes.
const CONTAINERS: [[u8; 4]; 10] = [
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"mvex", *b"moof", *b"traf", *b"edts",
    *b"udta",
];

/// Read the box starting at `off`, returning its payload bounds.
fn read_box(buf: &[u8], off: usize, end: usize) -> Option<Located> {
    if off + 8 > end {
        return None;
    }
    let raw = u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
    let mut kind = [0u8; 4];
    kind.copy_from_slice(&buf[off + 4..off + 8]);
    let (size, header) = if raw == 1 {
        if off + 16 > end {
            return None;
        }
        let mut wide = [0u8; 8];
        wide.copy_from_slice(&buf[off + 8..off + 16]);
        (u64::from_be_bytes(wide) as usize, 16)
    } else if raw == 0 {
        (end - off, 8)
    } else {
        (raw, 8)
    };
    if size < header || off + size > end {
        return None;
    }
    Some(Located {
        kind,
        start: off + header,
        end: off + size,
    })
}

fn children(buf: &[u8], b: &Located) -> Vec<Located> {
    let mut out = Vec::new();
    let mut off = b.start;
    while let Some(child) = read_box(buf, off, b.end) {
        out.push(child);
        off = child.end;
    }
    out
}

/// Depth-first search for every box of one kind inside a container.
fn find_all(buf: &[u8], root: &Located, name: &[u8; 4]) -> Vec<Located> {
    let mut out = Vec::new();
    let mut stack = children(buf, root);
    while let Some(b) = stack.pop() {
        if b.is(name) {
            out.push(b);
        }
        if CONTAINERS.contains(&b.kind) {
            stack.extend(children(buf, &b));
        }
    }
    out
}

fn be_u32(buf: &[u8], at: usize) -> Option<u32> {
    if at + 4 > buf.len() {
        return None;
    }
    Some(u32::from_be_bytes([
        buf[at],
        buf[at + 1],
        buf[at + 2],
        buf[at + 3],
    ]))
}

fn be_u64(buf: &[u8], at: usize) -> Option<u64> {
    if at + 8 > buf.len() {
        return None;
    }
    let mut wide = [0u8; 8];
    wide.copy_from_slice(&buf[at..at + 8]);
    Some(u64::from_be_bytes(wide))
}

/// `version` + `flags` of a FullBox, or `None` when the box is too short.
fn full_box_version(buf: &[u8], b: &Located) -> Option<u8> {
    if b.start < b.end {
        Some(buf[b.start])
    } else {
        None
    }
}

/// `(timescale, duration_in_timescale_units)` from an `mdhd` box.
fn parse_mdhd(buf: &[u8], b: &Located) -> Option<(u64, u64)> {
    let version = full_box_version(buf, b)?;
    if version == 1 {
        Some((be_u64(buf, b.start + 20)?, be_u64(buf, b.start + 28)?))
    } else {
        Some((
            be_u32(buf, b.start + 12)? as u64,
            be_u32(buf, b.start + 16)? as u64,
        ))
    }
}

/// Total duration in timescale units from an `stts` box.
fn parse_stts_total(buf: &[u8], b: &Located) -> Option<u64> {
    if full_box_version(buf, b)? != 0 {
        return None;
    }
    let count = be_u32(buf, b.start + 4)? as usize;
    let mut total: u64 = 0;
    for i in 0..count {
        let at = b.start + 8 + i * 8;
        let samples = be_u32(buf, at)? as u64;
        let delta = be_u32(buf, at + 4)? as u64;
        total = total.saturating_add(samples.saturating_mul(delta));
    }
    Some(total)
}

/// `(uniform_sample_size, sample_count)` from an `stsz` box. A non-zero uniform
/// size means per-sample sizes are not stored, which the oracle handles.
fn parse_stsz(buf: &[u8], b: &Located) -> Option<(u64, u64)> {
    if full_box_version(buf, b)? != 0 {
        return None;
    }
    Some((
        be_u32(buf, b.start + 4)? as u64,
        be_u32(buf, b.start + 8)? as u64,
    ))
}

/// Sum of the sample sizes in `[from, to)` from an `stsz` box.
fn stsz_range_bytes(buf: &[u8], b: &Located, uniform: u64, count: u64, from: u64, to: u64) -> u64 {
    if to <= from {
        return 0;
    }
    if uniform > 0 {
        return uniform.saturating_mul(to - from);
    }
    let span_end = to.min(count);
    if span_end <= from {
        return 0;
    }
    let mut total = 0u64;
    let mut idx = from;
    while idx < span_end {
        if let Some(size) = be_u32(buf, b.start + 12 + (idx as usize) * 4) {
            total = total.saturating_add(size as u64);
        }
        idx += 1;
    }
    total
}

/// `(first_chunk, samples_per_chunk)` runs from an `stsc` box.
fn parse_stsc(buf: &[u8], b: &Located) -> Option<Vec<(u64, u64)>> {
    if full_box_version(buf, b)? != 0 {
        return None;
    }
    let count = be_u32(buf, b.start + 4)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = b.start + 8 + i * 12;
        out.push((be_u32(buf, at)? as u64, be_u32(buf, at + 4)? as u64));
    }
    Some(out)
}

/// Chunk offsets from `stco` (32-bit) or `co64` (64-bit).
fn parse_chunk_offsets(buf: &[u8], b: &Located) -> Option<Vec<u64>> {
    if full_box_version(buf, b)? != 0 {
        return None;
    }
    let count = be_u32(buf, b.start + 4)? as usize;
    let wide = b.is(b"co64");
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = b.start + 8 + i * if wide { 8 } else { 4 };
        let value = if wide {
            be_u64(buf, at)
        } else {
            be_u32(buf, at).map(|v| v as u64)
        }?;
        out.push(value);
    }
    Some(out)
}

/// Parsed `sidx`: `(timescale, earliest_presentation_time, total_duration,
/// first_offset, total_referenced_size)`.
///
/// Layout (ISO 14496-12): `version/flags(4) reference_ID(4) timescale(4)
/// earliest_presentation_time(4|8) first_offset(4|8) reserved(2)
/// reference_count(2)`, then 12-byte entries. The referenced sizes sum to the
/// whole object, so `sidx.end + first_offset + Σ referenced_size` is the file
/// size a complete file must reach.
struct Sidx {
    secs: f64,
    expected_size: u64,
}

fn parse_sidx(buf: &[u8], b: &Located) -> Option<Sidx> {
    let version = full_box_version(buf, b)?;
    let timescale = be_u32(buf, b.start + 8)? as u64;
    if timescale == 0 {
        return None;
    }
    let (earliest, first_offset, count_at, entries_at) = if version == 0 {
        (
            be_u32(buf, b.start + 12)? as u64,
            be_u32(buf, b.start + 16)? as u64,
            b.start + 22,
            b.start + 24,
        )
    } else {
        (
            be_u64(buf, b.start + 12)?,
            be_u64(buf, b.start + 20)?,
            b.start + 30,
            b.start + 32,
        )
    };
    let count = be_u32(buf, count_at)? as usize & 0x7fff;
    let mut duration = 0u64;
    let mut referenced = 0u64;
    let mut at = entries_at;
    for _ in 0..count {
        let size = be_u32(buf, at)? as u64;
        let sub = be_u32(buf, at + 4)? as u64;
        referenced = referenced.saturating_add(size);
        duration = duration.saturating_add(sub);
        at += 12;
    }
    let expected_size = (b.end as u64)
        .saturating_add(first_offset)
        .saturating_add(referenced);
    Some(Sidx {
        secs: earliest.saturating_add(duration) as f64 / timescale as f64,
        expected_size,
    })
}

/// `handler_type` of a track, e.g. `soun` / `vide`.
fn parse_handler(buf: &[u8], b: &Located) -> Option<[u8; 4]> {
    if b.start + 12 > b.end {
        return None;
    }
    let mut out = [0u8; 4];
    out.copy_from_slice(&buf[b.start + 8..b.start + 12]);
    Some(out)
}

struct TrackFacts {
    is_audio: bool,
    timescale: u64,
    duration_units: u64,
    table_units: Option<u64>,
    data_end: Option<u64>,
    sample_count: Option<u64>,
    chunk_count: Option<u64>,
}

fn inspect_track(buf: &[u8], trak: &Located) -> Option<TrackFacts> {
    let handler = find_all(buf, trak, b"hdlr")
        .first()
        .and_then(|b| parse_handler(buf, b))?;
    let mdhd_box = find_all(buf, trak, b"mdhd").into_iter().next()?;
    let (timescale, duration_units) = parse_mdhd(buf, &mdhd_box)?;
    if timescale == 0 {
        return None;
    }
    let table_units = find_all(buf, trak, b"stts")
        .into_iter()
        .next()
        .and_then(|b| parse_stts_total(buf, &b));

    let stsz_box = find_all(buf, trak, b"stsz").into_iter().next();
    let offsets_box = find_all(buf, trak, b"stco")
        .into_iter()
        .next()
        .or_else(|| find_all(buf, trak, b"co64").into_iter().next());
    let stsc_box = find_all(buf, trak, b"stsc").into_iter().next();

    let mut data_end = None;
    let mut sample_count = None;
    let mut chunk_count = None;
    if let (Some(stsz_box), Some(offsets_box), Some(stsc_box)) =
        (&stsz_box, &offsets_box, &stsc_box)
    {
        if let (Some((uniform, count)), Some(offsets), Some(runs)) = (
            parse_stsz(buf, stsz_box),
            parse_chunk_offsets(buf, offsets_box),
            parse_stsc(buf, stsc_box),
        ) {
            if let Some((last_offset, last_first_sample, samples_in_last)) =
                last_chunk(&offsets, &runs)
            {
                data_end = Some(last_offset.saturating_add(stsz_range_bytes(
                    buf,
                    stsz_box,
                    uniform,
                    count,
                    last_first_sample,
                    last_first_sample.saturating_add(samples_in_last),
                )));
                sample_count = Some(count);
                chunk_count = Some(offsets.len() as u64);
            }
        }
    }

    Some(TrackFacts {
        is_audio: &handler == b"soun",
        timescale,
        duration_units,
        table_units,
        data_end,
        sample_count,
        chunk_count,
    })
}

/// Offset of the last chunk, the index of its first sample, and how many samples
/// it holds. `stsc` maps chunk number ranges to a sample count per chunk.
fn last_chunk(offsets: &[u64], runs: &[(u64, u64)]) -> Option<(u64, u64, u64)> {
    let last_index = offsets.len() as u64;
    if last_index == 0 || runs.is_empty() {
        return None;
    }
    // Sample index of the first sample of the last chunk = sum over all chunks.
    let mut samples_before: u64 = 0;
    let mut samples_per_chunk: u64 = runs[0].1;
    let mut run_idx = 0usize;
    for chunk_no in 1..=last_index {
        while run_idx + 1 < runs.len() && runs[run_idx + 1].0 <= chunk_no {
            run_idx += 1;
        }
        samples_per_chunk = runs[run_idx].1;
        if chunk_no < last_index {
            samples_before = samples_before.saturating_add(samples_per_chunk);
        }
    }
    Some((*offsets.last()?, samples_before, samples_per_chunk))
}

/// Read the container structure of a media file.
///
/// Never fails: an unreadable or unsupported file yields
/// [`Verdict::Unknown`].
pub fn inspect_container(path: &Path) -> ContainerFacts {
    let mut facts = ContainerFacts {
        container: "unknown",
        ..Default::default()
    };
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return facts,
    };
    facts.size_bytes = size;
    if size == 0 || size > MAX_INSPECT_BYTES {
        return facts;
    }
    let mut buf = Vec::new();
    if File::open(path)
        .and_then(|mut f| f.read_to_end(&mut buf))
        .is_err()
    {
        return facts;
    }

    // Top level: find the movie box, count fragments, read any segment index.
    let mut off = 0usize;
    let mut moov: Option<Located> = None;
    let mut sidx: Option<Sidx> = None;
    while let Some(b) = read_box(&buf, off, buf.len()) {
        match &b.kind {
            b"moov" => moov = Some(b),
            b"moof" => facts.fragment_count += 1,
            b"sidx" => sidx = parse_sidx(&buf, &b),
            _ => {}
        }
        off = b.end;
    }

    if let Some(index) = sidx {
        facts.container = "mp4-fragmented";
        facts.declared_secs = Some(index.secs);
        facts.table_secs = Some(index.secs);
        facts.audio_data_end = Some(index.expected_size);
        facts.verdict = Some(if index.expected_size <= size {
            Verdict::Complete
        } else {
            Verdict::Truncated {
                missing_bytes: index.expected_size - size,
            }
        });
        return facts;
    }

    let Some(moov) = moov else {
        return facts;
    };

    let mut audio: Option<TrackFacts> = None;
    for trak in children(&buf, &moov) {
        if !trak.is(b"trak") {
            continue;
        }
        let Some(track) = inspect_track(&buf, &trak) else {
            continue;
        };
        if track.is_audio && audio.is_none() {
            audio = Some(track);
        } else if !track.is_audio {
            facts.has_video_track = true;
        }
    }

    let Some(track) = audio else {
        return facts;
    };
    facts.audio_track_found = true;
    facts.container = if facts.fragment_count > 0 {
        "mp4-fragmented"
    } else {
        "mp4-stbl"
    };
    facts.declared_secs = Some(track.duration_units as f64 / track.timescale as f64);
    facts.table_secs =
        Some(track.table_units.unwrap_or(track.duration_units) as f64 / track.timescale as f64);
    facts.sample_count = track.sample_count;
    facts.chunk_count = track.chunk_count;
    if let Some(end) = track.data_end {
        facts.audio_data_end = Some(end);
        facts.verdict = Some(if end <= size {
            Verdict::Complete
        } else {
            Verdict::Truncated {
                missing_bytes: end - size,
            }
        });
    }
    facts
}

/// Decode the whole file and find how much of it is actually audible.
///
/// This is the check a sample table cannot make: the table says which bytes
/// *should* be there, not whether they contain music. A complete file that ends
/// in digital silence reports a large gap between `audible_secs` and
/// `decoded_secs`, which is the signature of a server-side partial stream.
pub fn inspect_content(path: &Path, ext: &str) -> ContentFacts {
    let mut facts = ContentFacts::default();
    let Ok(file) = File::open(path) else {
        return facts;
    };
    let reader = BufReader::with_capacity(64 * 1024, file);
    let decoder = if ext.is_empty() {
        Decoder::new(reader).ok()
    } else {
        Decoder::builder()
            .with_data(reader)
            .with_hint(ext)
            .build()
            .ok()
            .or_else(|| {
                // The hint can be wrong for a mislabelled container; retry with a
                // fresh handle (the first reader was consumed by the builder).
                File::open(path)
                    .ok()
                    .and_then(|f| Decoder::new(BufReader::with_capacity(64 * 1024, f)).ok())
            })
    };
    let Some(decoder) = decoder else {
        return facts;
    };
    facts.decoded_secs = decoder.total_duration().map(|d| d.as_secs());
    facts.sample_rate = decoder.sample_rate();
    let rate = decoder.sample_rate().max(1) as f64;
    let mut index: u64 = 0;
    for sample in decoder.samples() {
        if sample.abs() > AUDIBLE_THRESHOLD {
            facts.audible_samples = index + 1;
        }
        index += 1;
    }
    facts.total_samples = index;
    facts.audible_secs = Some(facts.audible_samples as f64 / rate);
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but *valid* MP4 sample table: one audio track whose single
    /// chunk starts 1000 bytes into the file and holds two 512-byte samples.
    /// `sample_bytes` is how much of that media data is actually written.
    fn synthetic_mp4(sample_bytes: usize) -> Vec<u8> {
        fn box_of(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = Vec::with_capacity(payload.len() + 8);
            out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            out
        }

        // stts: one entry, 2 samples of 1024 timescale units.
        let mut stts = vec![0u8, 0, 0, 0];
        stts.extend_from_slice(&1u32.to_be_bytes());
        stts.extend_from_slice(&2u32.to_be_bytes());
        stts.extend_from_slice(&1024u32.to_be_bytes());

        // stsc: one run, first_chunk 1, 2 samples per chunk.
        let mut stsc = vec![0u8, 0, 0, 0];
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&2u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());

        // stsz: no uniform size, 2 samples of 512 bytes each.
        let mut stsz = vec![0u8, 0, 0, 0];
        stsz.extend_from_slice(&0u32.to_be_bytes());
        stsz.extend_from_slice(&2u32.to_be_bytes());
        stsz.extend_from_slice(&512u32.to_be_bytes());
        stsz.extend_from_slice(&512u32.to_be_bytes());

        // stco: a single chunk at offset 1000.
        let mut stco = vec![0u8, 0, 0, 0];
        stco.extend_from_slice(&1u32.to_be_bytes());
        stco.extend_from_slice(&1000u32.to_be_bytes());

        let stts_box = box_of(b"stts", &stts);
        let stsc_box = box_of(b"stsc", &stsc);
        let stsz_box = box_of(b"stsz", &stsz);
        let stco_box = box_of(b"stco", &stco);
        let stbl = box_of(b"stbl", &[stts_box, stsc_box, stsz_box, stco_box].concat());
        let minf = box_of(b"minf", &stbl);

        // mdhd: version 0, timescale 48000, duration 2048 units.
        let mut mdhd = vec![0u8, 0, 0, 0];
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&48_000u32.to_be_bytes());
        mdhd.extend_from_slice(&2048u32.to_be_bytes());
        mdhd.extend_from_slice(&0u16.to_be_bytes());

        let mdia = box_of(b"mdia", &[box_of(b"mdhd", &mdhd), minf].concat());

        // hdlr: version/flags, pre_defined, handler_type = "soun".
        let mut hdlr = vec![0u8, 0, 0, 0];
        hdlr.extend_from_slice(&0u32.to_be_bytes());
        hdlr.extend_from_slice(b"soun");
        let trak = box_of(b"trak", &[box_of(b"hdlr", &hdlr), mdia].concat());
        let moov = box_of(b"moov", &trak);

        // Pad to the chunk offset, then write the media data.
        let mut file = moov;
        while file.len() < 1000 {
            file.push(0);
        }
        file.extend(std::iter::repeat(0u8).take(sample_bytes));
        file
    }

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("auralis_forensics_{}_{}", std::process::id(), name));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("probe.mp4");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_complete_sample_table_is_reported_complete() {
        let path = write_temp("complete", &synthetic_mp4(1024));
        let facts = inspect_container(&path);
        assert_eq!(facts.container, "mp4-stbl", "{}", facts.summary());
        assert!(facts.audio_track_found);
        assert_eq!(facts.audio_data_end, Some(2024));
        assert_eq!(
            facts.verdict,
            Some(Verdict::Complete),
            "{}",
            facts.summary()
        );
        assert!((facts.table_secs.unwrap() - 2048.0 / 48_000.0).abs() < 1e-9);
        assert_eq!(facts.sample_count, Some(2));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_short_file_is_reported_with_the_missing_byte_count() {
        // The table still describes 1024 bytes of media, only 400 arrived.
        let path = write_temp("short", &synthetic_mp4(400));
        let facts = inspect_container(&path);
        assert_eq!(
            facts.verdict,
            Some(Verdict::Truncated { missing_bytes: 624 })
        );
        assert!(
            facts.summary().contains("bytes=missing 624"),
            "{}",
            facts.summary()
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn garbage_is_unknown_rather_than_wrong() {
        let path = write_temp("garbage", b"\x00\x01\x02not an mp4 at all");
        let facts = inspect_container(&path);
        assert_eq!(facts.verdict, None);
        assert_eq!(facts.container, "unknown");
        assert!(facts.summary().contains("bytes=unknown"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_file_is_unknown() {
        let facts = inspect_container(Path::new("/nonexistent/auralis/nope.mp4"));
        assert_eq!(facts.verdict, None);
        assert_eq!(facts.size_bytes, 0);
    }

    #[test]
    fn last_chunk_maps_offsets_to_sample_ranges() {
        // 5 chunks; the last one holds 3 samples starting at sample 4.
        let offsets = vec![10, 20, 30, 40, 50];
        let runs = vec![(1, 2), (4, 3)];
        let (offset, first_sample, samples) = last_chunk(&offsets, &runs).unwrap();
        assert_eq!((offset, first_sample, samples), (50, 4, 3));
    }
}
