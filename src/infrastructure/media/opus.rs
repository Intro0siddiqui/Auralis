//! Opus & WebM Audio Decoder
//!
//! Provides demuxing for WebM/Matroska and Ogg containers via Symphonia,
//! with high-performance pure-Rust Opus sample decoding:
//! - On 64-bit architectures (`aarch64`, `x86_64`): uses `rusty-opus` with AVX2 & ARM64 NEON SIMD kernels.
//! - On 32-bit architectures (`armv7`, `i686`): uses `opus-decoder` for clean non-64-bit SIMD compatibility.

use rodio::source::SeekError;
use rodio::Source;
use std::fs::File;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::Path;
use std::time::Duration;
use symphonia::core::codecs::CODEC_TYPE_OPUS;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, warn};

/// Decoder engine abstraction selecting `rusty-opus` on 64-bit and `opus-decoder` on 32-bit.
pub enum OpusDecoderEngine {
    #[cfg(target_pointer_width = "64")]
    RustyOpus(rusty_opus::OpusDecoder),
    #[cfg(target_pointer_width = "32")]
    OpusDecoder {
        decoder: opus_decoder::OpusDecoder,
        i16_scratch: Vec<i16>,
    },
}

impl OpusDecoderEngine {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, String> {
        #[cfg(target_pointer_width = "64")]
        {
            rusty_opus::OpusDecoder::new(sample_rate as i32, channels as usize)
                .map(OpusDecoderEngine::RustyOpus)
                .map_err(|e| format!("rusty-opus error: {e}"))
        }

        #[cfg(target_pointer_width = "32")]
        {
            opus_decoder::OpusDecoder::new(sample_rate, channels as usize)
                .map(|decoder| OpusDecoderEngine::OpusDecoder {
                    decoder,
                    i16_scratch: vec![0i16; 5760 * channels as usize],
                })
                .map_err(|e| format!("opus-decoder error: {e:?}"))
        }
    }

    /// Decode an encoded Opus packet into the provided `out` buffer of f32 samples.
    /// Returns the number of decoded samples per channel.
    pub fn decode(
        &mut self,
        data: &[u8],
        max_samples_per_channel: usize,
        out: &mut [f32],
    ) -> Result<usize, String> {
        match self {
            #[cfg(target_pointer_width = "64")]
            OpusDecoderEngine::RustyOpus(dec) => dec
                .decode(data, max_samples_per_channel, out)
                .map_err(|e| format!("rusty-opus decode error: {e}")),
            #[cfg(target_pointer_width = "32")]
            OpusDecoderEngine::OpusDecoder {
                decoder,
                i16_scratch,
            } => {
                let needed = max_samples_per_channel * decoder.channels();
                if i16_scratch.len() < needed {
                    i16_scratch.resize(needed, 0);
                }
                match decoder.decode(data, i16_scratch, false) {
                    Ok(samples_per_ch) => {
                        let total = samples_per_ch * decoder.channels();
                        for (i, &s) in i16_scratch[..total].iter().enumerate() {
                            out[i] = s as f32 / 32768.0;
                        }
                        Ok(samples_per_ch)
                    }
                    Err(e) => Err(format!("opus-decoder decode error: {e:?}")),
                }
            }
        }
    }
}

/// An audio source that demuxes WebM/MKV/Ogg containers with Symphonia
/// and decodes Opus packets into PCM `f32` samples.
pub struct OpusSource {
    format_reader: Box<dyn FormatReader>,
    track_id: u32,
    decoder: OpusDecoderEngine,
    channels: u16,
    sample_rate: u32,
    total_duration: Option<Duration>,
    /// Decoded samples waiting to be yielded by Iterator
    current_samples: Vec<f32>,
    sample_index: usize,
    /// Scratch buffer for Opus decoding
    decode_buf: Vec<f32>,
    /// Reached EOF on packets
    is_eof: bool,
}

impl OpusSource {
    /// Attempt to open an audio file as an Opus/WebM stream from a path.
    pub fn open(path: &str) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open file {path}: {e}"))?;
        Self::new(file, path)
    }

    /// Create a new OpusSource from a seekable reader and file path hint.
    pub fn new<R: MediaSource + 'static>(reader: R, path: &str) -> Result<Self, String> {
        let mss = MediaSourceStream::new(Box::new(reader), Default::default());
        let mut hint = Hint::new();

        if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| format!("Symphonia probe failed for {path}: {e}"))?;

        let format_reader = probed.format;

        // Find the first Opus audio track
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec == CODEC_TYPE_OPUS)
            .ok_or_else(|| format!("No Opus audio track found in {path}"))?
            .clone();

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(48000).max(1);
        let channels = track
            .codec_params
            .channels
            .map(|c| c.count() as u16)
            .unwrap_or(2)
            .max(1);

        // Calculate total duration if available
        let total_duration = if let (Some(n_frames), Some(tb)) =
            (track.codec_params.n_frames, track.codec_params.time_base)
        {
            let time = tb.calc_time(n_frames);
            Some(Duration::from_secs_f64(time.seconds as f64 + time.frac))
        } else if let (Some(n_frames), sr) = (track.codec_params.n_frames, sample_rate) {
            if sr > 0 {
                Some(Duration::from_secs_f64(n_frames as f64 / sr as f64))
            } else {
                None
            }
        } else {
            None
        };

        let decoder = OpusDecoderEngine::new(sample_rate, channels)
            .map_err(|e| format!("Failed to create OpusDecoder for {path}: {e}"))?;

        // 120ms max frame size at 48kHz = 5760 samples per channel
        let decode_buf = vec![0.0f32; 5760 * channels as usize];

        Ok(Self {
            format_reader,
            track_id,
            decoder,
            channels,
            sample_rate,
            total_duration,
            current_samples: Vec::with_capacity(5760 * channels as usize),
            sample_index: 0,
            decode_buf,
            is_eof: false,
        })
    }

    fn decode_next_packet(&mut self) -> bool {
        self.current_samples.clear();
        self.sample_index = 0;

        loop {
            match self.format_reader.next_packet() {
                Ok(packet) => {
                    if packet.track_id() != self.track_id {
                        continue;
                    }
                    let data = &packet.data[..];
                    if data.is_empty() {
                        continue;
                    }
                    let max_samples_per_channel = 5760;
                    let needed_len = max_samples_per_channel * self.channels as usize;
                    if self.decode_buf.len() < needed_len {
                        self.decode_buf.resize(needed_len, 0.0);
                    }
                    match self
                        .decoder
                        .decode(data, max_samples_per_channel, &mut self.decode_buf)
                    {
                        Ok(samples_per_ch) => {
                            let count = samples_per_ch * self.channels as usize;
                            self.current_samples
                                .extend_from_slice(&self.decode_buf[..count]);
                            return true;
                        }
                        Err(e) => {
                            warn!("Opus decode error on packet: {e}");
                            continue;
                        }
                    }
                }
                Err(symphonia::core::errors::Error::IoError(ref err))
                    if err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return false;
                }
                Err(symphonia::core::errors::Error::ResetRequired) => {
                    // Reset required by container
                    continue;
                }
                Err(e) => {
                    debug!("Format reader end of stream or error: {e}");
                    return false;
                }
            }
        }
    }
}

impl Iterator for OpusSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        while self.sample_index >= self.current_samples.len() {
            if self.is_eof {
                return None;
            }
            if !self.decode_next_packet() {
                self.is_eof = true;
                return None;
            }
        }

        let sample = self.current_samples[self.sample_index];
        self.sample_index += 1;
        Some(sample)
    }
}

impl Source for OpusSource {
    fn current_span_len(&self) -> Option<usize> {
        let rem = self.current_samples.len().saturating_sub(self.sample_index);
        if rem > 0 {
            Some(rem)
        } else if self.is_eof {
            Some(0)
        } else {
            None
        }
    }

    fn channels(&self) -> NonZeroU16 {
        NonZeroU16::new(self.channels).unwrap_or(NonZeroU16::new(2).unwrap())
    }

    fn sample_rate(&self) -> NonZeroU32 {
        NonZeroU32::new(self.sample_rate).unwrap_or(NonZeroU32::new(48000).unwrap())
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let time = symphonia::core::units::Time::from(pos);
        match self.format_reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track_id),
            },
        ) {
            Ok(_) => {
                if let Ok(new_dec) = OpusDecoderEngine::new(self.sample_rate, self.channels) {
                    self.decoder = new_dec;
                }
                self.current_samples.clear();
                self.sample_index = 0;
                self.is_eof = false;
                Ok(())
            }
            Err(e) => {
                warn!("Opus seek failed: {e}");
                Err(SeekError::NotSupported {
                    underlying_source: "OpusSource",
                })
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OpusMetadata {
    pub duration_secs: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Extract metadata from a WebM / Matroska or Opus audio file using Symphonia probe.
pub fn extract_opus_metadata(path: &Path) -> Result<OpusMetadata, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Symphonia probe failed: {e}"))?;

    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec == CODEC_TYPE_OPUS)
        .or_else(|| probed.format.tracks().first())
        .ok_or_else(|| "No track found in media container".to_string())?
        .clone();

    let sample_rate = track.codec_params.sample_rate.unwrap_or(48000);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(2);

    let duration_secs = if let (Some(n_frames), Some(tb)) =
        (track.codec_params.n_frames, track.codec_params.time_base)
    {
        let time = tb.calc_time(n_frames);
        (time.seconds as f64 + time.frac).round() as u32
    } else if let (Some(n_frames), sr) = (track.codec_params.n_frames, sample_rate) {
        if sr > 0 {
            (n_frames as f64 / sr as f64).round() as u32
        } else {
            0
        }
    } else {
        0
    };

    let mut title = None;
    let mut artist = None;
    let mut album = None;

    // Check Symphonia metadata revisions for tags
    if let Some(meta) = probed.format.metadata().current() {
        for tag in meta.tags() {
            match tag.std_key {
                Some(symphonia::core::meta::StandardTagKey::TrackTitle) => {
                    title = Some(tag.value.to_string());
                }
                Some(symphonia::core::meta::StandardTagKey::Artist) => {
                    artist = Some(tag.value.to_string());
                }
                Some(symphonia::core::meta::StandardTagKey::Album) => {
                    album = Some(tag.value.to_string());
                }
                _ => {}
            }
        }
    }

    Ok(OpusMetadata {
        duration_secs,
        sample_rate,
        channels,
        title,
        artist,
        album,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_m4a_opus_decoding() {
        let sample_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch/sample.m4a");
        if !sample_path.exists() {
            eprintln!("scratch/sample.m4a not found, skipping test");
            return;
        }

        let mut source = OpusSource::open(sample_path.to_str().unwrap())
            .expect("Failed to open scratch/sample.m4a with OpusSource");

        assert_eq!(source.sample_rate().get(), 48000);
        assert_eq!(source.channels().get(), 2);

        // Decode at least the first 50000 samples (~0.5s of stereo audio)
        let mut sample_count = 0;
        for _ in 0..50000 {
            if source.next().is_some() {
                sample_count += 1;
            } else {
                break;
            }
        }
        assert!(
            sample_count > 1000,
            "Expected to decode at least 1000 samples, decoded {}",
            sample_count
        );

        let meta = extract_opus_metadata(&sample_path).expect("extract_opus_metadata failed");
        assert_eq!(meta.sample_rate, 48000);
        assert_eq!(meta.channels, 2);
        assert!(meta.duration_secs > 0, "Duration should be non-zero");
    }
}
