#!/usr/bin/env python3
"""
generate_test_audio.py — Synthetic WAV/MP3 fixture generator for Auralis testing.

Generates deterministic audio fixtures in scripts/tests/fixtures/ for local
and CI audio scanning, player playback, and import tests without requiring external binaries.
"""

import os
import math
import struct
import wave

FIXTURES_DIR = os.path.join(os.path.dirname(__file__), "fixtures")

def generate_wav(filename="test_audio.wav", duration_sec=1.5, sample_rate=44100, freq=440.0):
    os.makedirs(FIXTURES_DIR, exist_ok=True)
    filepath = os.path.join(FIXTURES_DIR, filename)
    num_samples = int(sample_rate * duration_sec)

    with wave.open(filepath, "w") as wav_file:
        wav_file.setnchannels(1)  # Mono
        wav_file.setsampwidth(2)  # 16-bit
        wav_file.setframerate(sample_rate)

        for i in range(num_samples):
            t = float(i) / sample_rate
            sample = int(32767.0 * 0.5 * math.sin(2.0 * math.pi * freq * t))
            data = struct.pack("<h", sample)
            wav_file.writeframesraw(data)

    print(f"✓ Generated synthetic WAV: {filepath} ({os.path.getsize(filepath)} bytes)")
    return filepath

def generate_mp3(filename="test_audio.mp3"):
    """
    Generates a minimal valid MP3 file with MPEG-1 Layer 3 frames.
    """
    os.makedirs(FIXTURES_DIR, exist_ok=True)
    filepath = os.path.join(FIXTURES_DIR, filename)

    # 32kbps 44.1kHz Mono MP3 frame header: 0xFF 0xFB 0x10 0x64 (or 0xFF 0xFB 0x30 0xC4)
    # Frame size for 32kbps, 44100Hz: 144 * 32000 / 44100 = 104 bytes
    frame_header = bytes([0xFF, 0xFB, 0x10, 0x64])
    frame_data = bytes([0x00] * 100)
    frame = frame_header + frame_data

    # Write ~50 frames to form ~1.3 seconds of MP3 audio
    with open(filepath, "wb") as f:
        for _ in range(50):
            f.write(frame)

    print(f"✓ Generated synthetic MP3: {filepath} ({os.path.getsize(filepath)} bytes)")
    return filepath

def main():
    print("==> Generating synthetic audio test fixtures...")
    generate_wav("test_audio.wav")
    generate_wav("test_audio_b.wav", freq=880.0)
    generate_mp3("test_audio.mp3")
    print("✓ All test audio fixtures generated successfully.")

if __name__ == "__main__":
    main()
