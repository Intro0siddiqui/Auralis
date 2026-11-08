//! AAC Audio Decoder
//!
//! Provides demuxing for MP4, M4A, and ADTS containers via Symphonia,
//! with high-performance pure-Rust AAC sample decoding via `rusty_aac`.
//!
//! Specifically handles multichannel (5.1 / 7.1 surround sound) AAC streams,
//! fragmented MP4 (DASH/sidx) tracks, and files where Symphonia's built-in
//! AAC decoder rejects with "aac too complex". Automatically downmixes
//! multichannel streams to balanced stereo (ITU-R BS.775) with vocal clarity.

use rodio::source::SeekError;
use rodio::Source;
use std::fs::File;
use std::num::{NonZeroU16, NonZeroU32};
use std::path::Path;
use std::time::Duration;
use symphonia::core::codecs::CODEC_TYPE_AAC;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, warn};

/// Downmix interleaved PCM float samples to stereo (2 channels) or mono.
#[inline]
fn downmix_to_stereo(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    match channels {
        1 => {
            out.reserve(samples.len() * 2);
            for &s in samples {
                out.push(s);
                out.push(s);
            }
        }
        2 => {
            out.extend_from_slice(samples);
        }
        6 => {
            // 5.1 surround sound: [FL, FR, Center, LFE, SL, SR]
            // Standard ITU-R BS.775 stereo downmix:
            // L = (FL + 0.7071*Center + 0.7071*SL + 0.5*LFE) * norm
            // R = (FR + 0.7071*Center + 0.7071*SR + 0.5*LFE) * norm
            let frames = samples.len() / 6;
            out.reserve(frames * 2);
            const COEF_C: f32 = 0.7071;
            const COEF_S: f32 = 0.7071;
            const COEF_LFE: f32 = 0.5;
            const NORM: f32 = 1.0 / (1.0 + COEF_C + COEF_S + COEF_LFE * 0.5);
            for chunk in samples.chunks_exact(6) {
                let fl = chunk[0];
                let fr = chunk[1];
                let fc = chunk[2];
                let lfe = chunk[3];
                let sl = chunk[4];
                let sr = chunk[5];
                let l = (fl + COEF_C * fc + COEF_S * sl + COEF_LFE * lfe) * NORM;
                let r = (fr + COEF_C * fc + COEF_S * sr + COEF_LFE * lfe) * NORM;
                out.push(l);
                out.push(r);
            }
        }
        _ => {
            // Generic multichannel downmix: distribute odd/even channels to Left/Right
            let frames = samples.len() / channels;
            out.reserve(frames * 2);
            let scale = 1.0 / (channels as f32 / 2.0).max(1.0);
            for chunk in samples.chunks_exact(channels) {
                let mut l = 0.0;
                let mut r = 0.0;
                for (i, &s) in chunk.iter().enumerate() {
                    if i % 2 == 0 {
                        l += s;
                    } else {
                        r += s;
                    }
                }
                out.push(l * scale);
                out.push(r * scale);
            }
        }
    }
}

/// An audio source that demuxes MP4/M4A/ADTS containers with Symphonia
/// and decodes AAC packets into PCM `f32` stereo samples using `rusty_aac`.
pub struct AacSource {
    format_reader: Box<dyn FormatReader>,
    track_id: u32,
    decoder: rusty_aac::AacDecoder,
    extra_data: Option<Box<[u8]>>,
    sample_rate: u32,
    total_duration: Option<Duration>,
    /// Decoded interleaved stereo samples waiting to be yielded by Iterator
    current_samples: Vec<f32>,
    sample_index: usize,
    /// Reached EOF on packets
    is_eof: bool,
}

impl AacSource {
    /// Attempt to open an audio file as an AAC stream from a path.
    pub fn open(path: &str) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("Failed to open file {path}: {e}"))?;
        Self::new(file, path)
    }

    /// Create a new AacSource from a seekable reader and file path hint.
    pub fn new<R: MediaSource + 'static>(reader: R, path: &str) -> Result<Self, String> {
        let mut hint = Hint::new();
        if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        } else {
            hint.with_extension("m4a");
        }
        hint.mime_type("audio/mp4");

        let mss = MediaSourceStream::new(Box::new(reader), Default::default());

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| format!("Symphonia probe failed for {path}: {e}"))?;

        let format_reader = probed.format;

        // Find the first AAC audio track
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec == CODEC_TYPE_AAC)
            .ok_or_else(|| format!("No AAC audio track found in {path}"))?
            .clone();

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44100).max(1);

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

        let extra_data = track.codec_params.extra_data;
        let decoder = if let Some(edata) = &extra_data {
            rusty_aac::AacDecoder::with_config_bytes(edata)
                .map_err(|e| format!("Failed to configure AacDecoder for {path}: {e:?}"))?
        } else {
            rusty_aac::AacDecoder::new()
        };

        Ok(Self {
            format_reader,
            track_id,
            decoder,
            extra_data,
            sample_rate,
            total_duration,
            current_samples: Vec::with_capacity(4096),
            sample_index: 0,
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

                    match self.decoder.decode(data, None) {
                        Ok(audio) => {
                            let ch = audio.channels as usize;
                            if ch > 0 && !audio.samples.is_empty() {
                                downmix_to_stereo(&audio.samples, ch, &mut self.current_samples);
                                return true;
                            }
                        }
                        Err(rusty_aac::Error::Again) => continue,
                        Err(e) => {
                            debug!("AAC packet decode error: {e:?}; continuing");
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

impl Iterator for AacSource {
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

impl Source for AacSource {
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
        NonZeroU16::new(2).unwrap()
    }

    fn sample_rate(&self) -> NonZeroU32 {
        let sr = self
            .decoder
            .output_sample_rate()
            .unwrap_or(self.sample_rate)
            .max(1);
        NonZeroU32::new(sr).unwrap_or(NonZeroU32::new(44100).unwrap())
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
                if let Some(edata) = &self.extra_data {
                    if let Ok(new_dec) = rusty_aac::AacDecoder::with_config_bytes(edata) {
                        self.decoder = new_dec;
                    }
                } else {
                    self.decoder = rusty_aac::AacDecoder::new();
                }
                self.current_samples.clear();
                self.sample_index = 0;
                self.is_eof = false;
                Ok(())
            }
            Err(e) => {
                warn!("AAC seek failed: {e}");
                Err(SeekError::NotSupported {
                    underlying_source: "AacSource",
                })
            }
        }
    }
}
