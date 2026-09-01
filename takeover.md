# 🤖 Auralis Agent Takeover — v2.6.22 Smart Scanner & Playability Gate

> **Instructions for the receiving orchestrator agent**: This document contains everything
> you need to continue work on the Auralis codebase without any prior context.
> Follow the workflow described below precisely. Do NOT commit anything until
> both fixer subagents confirm zero warnings and 42/42 node tests pass. Then
> bump to v2.6.22, commit, tag, and push.

---

## 1. Repository Context

- **Repo**: `/workspaces/Auralis` (Tauri v2, Rust backend, HTMX frontend)
- **Current version**: `v2.6.21` (tag `fd59043` on `main`)
- **AGENTS.md** at the repo root contains all architecture and convention rules — all agents MUST read it before touching code.
- **`fileopt-todo.md`**: Roadmap for future monolithic file decomposition (do NOT do this now).
- **CI**: `.github/workflows/build.yml` — the lint job runs `cargo fmt --check` + `cargo clippy --all-targets --all-features -- -D warnings`. This MUST pass or the release is broken.

### Key Files for This Task

| File | Purpose |
|---|---|
| `src/infrastructure/media/downloader.rs` | Download engine: `validate_audio_file`, staging/atomic rename, retry loop |
| `src/infrastructure/filesystem/android.rs` | Android scanner: `ingest_buffer` (local import), `scan_sandboxed_dir` |
| `src/infrastructure/filesystem/desktop.rs` | Desktop scanner: `scan_library_paths_with_progress` |
| `src/infrastructure/filesystem/metadata.rs` | `MetadataExtractor::extract` — lofty-based metadata + embedded art |
| `src/commands/library.rs` | `import_audio_file` Tauri command — calls `ingest_buffer` |
| `ui/js/youtube.js` | YouTube resolver — `resolve()`, client fallback, PO-token, format selection |
| `Cargo.toml` | Version (bump to `2.6.22`) |
| `tauri.conf.json` | Version (bump to `2.6.22`) |
| `package.json` | Version (bump to `2.6.22`) |
| `AGENTS.md` | Update version references to `v2.6.22` |

---

## 2. What Needs to Be Built — The Problem

Files downloaded from YouTube or imported from Android file pickers are silently accepted
into the library even when:
1. The `.m4a` moov atom is at the end of the file (non-FastStart layout), causing
   Symphonia/rodio to throw `Decode error: An IO error occurred while reading, writing,
   or seeking the stream`.
2. The file has `duration_secs = 0` due to lofty failing to parse headers.
3. The file is partially written, truncated, or outright corrupt.
4. A track appears in "Continue Listening" with `0:00` duration and `"Unknown Artist"`,
   and crashes playback when tapped.

The fix is a **static playability health check** that gates every file before it enters
the database or library.

---

## 3. Exact Implementation Tasks

### 🔧 Task A — Backend Fixer Subagent

**File: `src/infrastructure/media/downloader.rs`**

1. **Strict Duration Gate in `validate_audio_file`** (already exists ~line 171, strengthen it):
   - If `duration_secs == 0` AND `expected_duration_secs.is_some()` → return error:
     `"Decoded duration is 0s — file has unreadable atom index tables or is truncated"`.
   - If `duration_secs == 0` AND no expected duration → validate that `file_size > 10_240`
     (10 KB minimum) AND `sample_rate > 0` AND `channels > 0` from `AudioProperties`.
     If any fail → return error.
   - Do NOT remove the existing ±5s tolerance check for non-zero expected durations.

2. **Downloader `run_stream` reject-on-zero gate** (~line 781 where `validate_audio_file` is called):
   - If `validate_audio_file` returns `Err(...)`, mark download as `Failed`, delete the
     `.part` staging file, emit `download:failed` event. Do NOT fall through to atomic rename.
   - Verify the existing `cleanup_staging_file` helper is called correctly on validation failure.

**File: `src/infrastructure/filesystem/android.rs` — `ingest_buffer` method (~line 75)**

3. **Static playability health check on local import**:
   - After the file bytes are written to disk, add a **Symphonia dry-run probe**:
     open the written file, pass it to `rodio::Decoder::builder().with_data(BufReader::new(file)).build()`.
   - If decoder initialization fails → delete the written file (`tokio::fs::remove_file`)
     and return `ScannerError::MetadataError(format!("File is unplayable by the audio decoder: {e}"))`.
     Do NOT insert a shell track into the DB.
   - If it succeeds → drop the decoder handle (dry-run only) and continue normally.

4. **Scanner skip-on-zero in `scan_sandboxed_dir`** (and desktop `scan_library_paths_with_progress`):
   - When `MetadataExtractor::extract` returns `Ok(track)` but `track.duration_secs == 0`,
     try a secondary `Probe::open().guess_file_type().read()` probe.
   - If guessed probe also returns 0 duration → log warning, skip file entirely (no DB insert),
     increment a `skipped_unplayable` counter in `ScanSummary`.
   - Include `skipped_unplayable` count in the tracing log and returned `ScanSummary`.

5. **Re-scan repair for existing 0-duration tracks**:
   - In scanner, when a path already exists in DB (`find_by_path` returns `Some(existing)`)
     but `existing.duration_secs == 0`, treat it as `needs_rescan` and attempt re-extraction
     rather than skipping as "already scanned".

**Ensure `cargo check --lib` passes with zero warnings before reporting back.**

---

### 🔧 Task B — Frontend & Format Fixer Subagent

**File: `ui/js/youtube.js`**

1. **Format preference scoring** in `resolve()` where the winning format is selected:

```js
function scoreFormat(fmt) {
  const mime = (fmt.mimeType || '').toLowerCase();
  // itag 140 = standard M4A 128kbps, usually FastStart from YouTube CDN
  if (fmt.itag === 140) return 3;
  if (mime.includes('mp4') && !mime.includes('video')) return 2;
  if (mime.includes('m4a')) return 2;
  return 1;
}
```

   Sort candidate formats by `scoreFormat` descending before picking the winner.
   **⚠️ IMPORTANT**: Check `Cargo.toml` first — rodio `0.22.2` in this project does NOT
   have the `opus` feature declared. Do NOT prefer WebM/Opus — it will cause `DecodeError`
   on rodio. Only apply itag 140 / m4a progressive preference.

2. **Error propagation to UI (`ui/js/modules/downloads.js`)**:
   - Ensure `download:failed` events (not just `download:completed`) surface the full error
     string including `"unplayable"` / `"0s duration"` text in the failed download row.
   - Check `updateDownloadProgressUI` — confirm `event.payload.error` is surfaced correctly.

3. **Test requirement**: `node --test scripts/tests/youtube_resolver.test.js` must remain
   42/42 pass. If format scoring is added, add a test asserting
   `scoreFormat({itag:140, mimeType:'audio/mp4'}) > scoreFormat({itag:251, mimeType:'audio/webm'})`.

**Ensure all node tests pass before reporting back.**

---

### 🔎 Task C — Verification Investigator Subagent

After both fixer subagents report completion, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check --all-targets
node --test scripts/tests/youtube_resolver.test.js
git status -s
```

Report back PASS or FAIL with full output. Do NOT modify any files.

---

## 4. Release Steps (Orchestrator Does This After Verification PASS)

1. **Bump version to `2.6.22`** in:
   - `Cargo.toml` line 3: `version = "2.6.22"`
   - `tauri.conf.json` line 4: `"version": "2.6.22"`
   - `package.json` line 3: `"version": "2.6.22"`
   - `AGENTS.md`: update all `v2.6.21` / `2.6.21` references to `v2.6.22` / `2.6.22`

2. **Update Cargo.lock**:
   ```bash
   cargo check --lib
   ```

3. **Commit, tag, push**:
   ```bash
   git add -A
   git commit -m "fix(scanner,downloader,frontend): static playability health check, reject 0-duration and corrupt audio on import and download"
   git tag v2.6.22
   git push origin main
   git push origin v2.6.22
   ```

4. **Verify CI** with `gh run list -L 3` — both `main` and `v2.6.22` builds must show ✓.

---

## 5. Subagent Dispatch Instructions

Spawn **Subagents 1 and 2 in parallel**, then spawn Subagent 3 after both report done.

### Subagent 1 — Backend Scanner & Downloader Health Check Engineer
- **Task**: Implement all of Task A (Section 3) above.
- **Primary files**: `src/infrastructure/media/downloader.rs`,
  `src/infrastructure/filesystem/android.rs`,
  `src/infrastructure/filesystem/desktop.rs`
- **Constraint**: `cargo check --lib` must pass with zero warnings.
- **Must read**: `/workspaces/Auralis/AGENTS.md` before any changes.

### Subagent 2 — Frontend Format Scoring & Error UI Specialist
- **Task**: Implement all of Task B (Section 3) above.
- **Primary files**: `ui/js/youtube.js`, `ui/js/modules/downloads.js`,
  `scripts/tests/youtube_resolver.test.js`
- **Constraint**: 42/42 node tests pass.
- **Must read**: `/workspaces/Auralis/AGENTS.md` and check `Cargo.toml` rodio features
  before touching format preferences.

### Subagent 3 — Release Verification Investigator
- **Task**: Run Task C verification suite (Section 3). Read-only.
- **Start only after Subagents 1 AND 2 both report success.**

---

## 6. Non-Negotiable Conventions

- **No `unwrap()`/`panic!()` in production code** — use `Result`/`Option` + `tracing`.
- **All Tauri commands return `Result<T, String>`** on the wire.
- **`cargo fmt` must be clean before any commit** — CI fails on format errors.
- **Clippy is `-D warnings`** — `sort_by` vs `sort_by_key` killed v2.6.21's lint. Be careful.
- **Do not touch `fileopt-todo.md`** — it is a tracked roadmap for a future session.
- **Do not bump versions** — the orchestrator does that only after verification passes.
