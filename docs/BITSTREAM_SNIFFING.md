# 🔬 Bitstream Sniffing Engine

**Implementation Date:** 2026-01-14  
**Modules:** `video_sniffing.rs`, `audio_sniffing.rs`  
**Rationale:** Provide deterministic, OS-independent codec identification as a final fallback for unrecognized media files.

---

## 🛠️ Architecture

The sniffing engine is designed to be **lightweight, fast, and deterministic**. It operates only when higher-level OS APIs (Property Store, Media Foundation) return generic identifiers (e.g., `3F40F4F0`) or "Unknown".

### Core Principles
1. **Low I/O Overhead**: Reads only the first 128 KB (audio) to 256 KB (video) of a file.
2. **Pure-Rust**: Zero external dependencies (no FFmpeg, no MediaInfo).
3. **Regex-Free**: Uses efficient byte pattern searching and window iterators.
4. **Deterministic**: Relies on international bitstream standards (NAL units, syncwords).

---

## 📽️ Video Detection Strategy

### 1. Container Atoms (Level 1)
For MP4 and MKV files, we perform deep-atom sniffing to find codec identifiers that the OS might have missed.
- **MP4/MOV**: Searches for atoms like `avc1`, `hvc1`, `av01`, `vp09`.
- **MKV/WebM**: Searches for EBML CodecID strings like `V_MPEG4/ISO/AVC`.

### 2. Bitstream Probing (Level 2)
Scans the raw bitstream for formal synchronization codes.
- **H.264 (AVC)**: Detects `00 00 00 01` start codes followed by NAL Unit Type 7 (SPS) and 8 (PPS).
- **HEVC (H.265)**: Detects NAL Unit Type 32 (VPS) and 33 (SPS).
- **VP9**: Identifies the 3-byte sync code `0x49 0x83 0x42`.

---

## 🎧 Audio Detection Strategy

### 1. Container & Chunks (Level 1)
- **WAV (RIFF)**: Parses the `fmt ` chunk format tags (PCM, MP3, AAC).
- **M4A/MP4**: Parses `mp4a` atoms for Object Type IDs.

### 2. Syncword Probing (Level 2)
Finds rhythmic syncwords that define the start of audio frames.
- **AAC**: ADTS syncword `0xFFF`.
- **MP3**: Frame sync `0x7FF`.
- **AC-3 / E-AC-3**: Syncword `0x0B77` and Bitstream ID (BSID) analysis.
- **FLAC/Opus/Vorbis**: Magic signatures (`fLaC`, `OpusHead`, `vorbis`).

---

## 🔄 Integration Workflow

The metadata pipeline follows this priority:

1. **Property Store (Fastest)**: Uses Windows Shell extensions (K-Lite, Icaros).
2. **Media Foundation**: Direct codec subtype GUID resolution.
3. **Codec Registry**: Resolution via Windows Registry `CLSID` lookups.
4. **Sniffing Engine (Definitive)**: Final bitstream verification.

Resulting metadata is tagged with `(Sniffed)` if the definitive layer was required.

---

## 🧪 Verification
- **H.264 ES**: Successfully identifies raw bitstreams in `.ts` files (previously `3F40F4F0`).
- **Ogg/OGM**: Correctly identifies Theora/Vorbis/FLAC tracks.
- **Corrupted Headers**: Resistant to minor header corruption by scanning a larger buffer (256 KB).

---

**Autor:** MTT Senior Engineering Team  
**Status:** ✅ Production-Ready
