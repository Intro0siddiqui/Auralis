# Auralis — YouTube Download Path

> Full accounting of the download pipeline, every measurement taken against it, and
> what is known, inferred, falsified, or still unknown. Updated **2026-09-29**
> (v2.6.64). Companion to `AGENTS.md` (architecture/conventions) and `PROJECT.md`
> (build/release/device).

---

## 0. Status

**A download completes end-to-end with no PO token at all. The blocker was ours.**

Resolved 2026-09-29, and the conclusion is the inverse of what this document
asserted for twenty releases. Three findings, in order of importance:

1. **There is no server-side window.** The file the app called "windowed to 54 s
   of 216 s" is complete. Proven from the container, on the same machine and the
   same network, from the same URL the phone used — **10 992 443 bytes, all
   received**, audio `stts` declaring **9 540 608 samples at 44 100 Hz =
   216.34 s**, and the last audio sample ending **inside** the file. The device's
   decoder read **25.2 %** of those samples and reported `measured=54.4s`.
2. **The PO token is not required for this path.** `itag 18` muxed completed at
   100 % with the token **explicitly withheld** (`held pot-apply: ANDROID is not
   web-family`). SABR, and therefore the whole `status=3` gate, is off the critical
   path.
3. **The completeness gate rejected a perfect file.** The sharpest form: the
   device's own forensics printed `bytes=complete` and `table=216.3s` — both
   correct — and `acceptance()` vetoed anyway, on the decoder.

```
                    ┌──────────────────────────────────┐
                    │ rodio stops decoding at ~25% of   │
                    │ a muxed itag-18 file              │
                    └────────────────┬─────────────────┘
                                     │  measured=54.4s, audible_until=54.4s
                    ┌────────────────▼─────────────────┐
                    │ acceptance() sees a short decode │
                    │ and REJECTS a complete file     │   <-- the bug
                    └────────────────┬─────────────────┘
                                     │
                    ┌────────────────▼─────────────────┐
                    │ owner sees "windowed download"  │
                    └──────────────────────────────────┘
```

The real chain — note there is no token anywhere in it:

```
muxed itag 18 from the ANDROID client
   → googlevideo serves the WHOLE object  (the 403 does not apply to this class)
   → every advertised byte arrives
   → the file is complete and self-consistent
   → our decoder reads 25% and says "54 seconds"
   → our own gate calls it truncated and throws the download away
```

### What is actually open

| # | Item | State |
|---|---|---|
| 1 | `acceptance()` must trust the container over the decoder | agent in flight |
| 2 | `reconcile_duration` must allow a downward correction, or a 216 s track shows as 54 s and seeks past it are refused | agent in flight |
| 3 | Resume re-pauses: `polls=7 progress=0 empty=0` | agent in flight |
| 4 | MediaStore publish fails 100 %: `Invalid column display_name`, API 36 — **this is why `Download/Auralis/` is always empty** | `@refactor` |
| 5 | PO token / SABR — needed only for the **audio-only adaptive** class, i.e. audio instead of audio+video. A quality difference, not a capability gate | on hold |

---

## 0a. The positive control, and the partial success

Two facts that reframe everything above, both from the owner, and both easy to
forget:

**1. The device already solves this, daily.** The owner watches YouTube on the
phone that runs Auralis — same WebView, same residential Jio line, same ISP,
logged out. That environment demonstrably obtains a valid PO token, minted by
YouTube's own `botguard.js`, in that engine. **The WebView we are failing inside
is provably capable.** That is a positive control, and it is why every "barren
environment / add jsdom" theory is dead: jsdom is *less* realistic than a real
WebView, so adopting it would be moving away from the working case.

**2. Downloads are not uniformly broken.** Some videos download successfully and
some do not. Any absolute-failure framing is wrong, and this report was wrong that
way for ~20 releases — the owner said so at **v2.6.41** and it was never written
down here. A working case beside a failing one is the most valuable diagnostic
asset available, because the difference between them is the variable. Untested
reading: a token is sometimes obtained and the difference is whether a live page
in that WebView had already minted — which would make our own `minted-stripped`
path the whole problem, and the WebView spike merely the wiring rather than an
experiment.

---

## 1. The pipeline

```
YouTubeResolver (ui/js/youtube.js)
  └─ vendored youtubei.js  ──►  InnerTube /player  ──►  formats + serverAbrStreamingUrl
        └─ client ladder (§3) and PO-token mint (§6)
                    │
                    ▼
DownloadRequest { url, itag, ext, title, artist, album }
                    │  Tauri invoke
                    ▼
Downloader (src/infrastructure/media/downloader.rs)
  └─ run_stream ──► reqwest ──► googlevideo / SABR
        └─ byte accounting ──► completeness gate ──► forensics
                    │
                    ▼
commit ──► tags.rs (lofty) ──► android_downloads.rs (MediaStore publish)
                    │
                    ▼
library scan ──► playable track
```

Three safety nets sit between the network and the user's library, and **all
three are load-bearing**:

| Net | Where | What it catches |
|---|---|---|
| Byte accounting | `downloader.rs::run_stream` | received vs advertised bytes |
| Completeness gate | `completeness.rs::verify_decoded_duration` | a file that ends early |
| Forensics | `forensics.rs` | *which* problem: transfer vs server window |

All three are container-truth-based, deliberately **not** decoder-based. A
decoder's opinion is not evidence: rodio under-reports `total_duration()` on
these MP4s (v2.6.45), which is how a 4:26 track once displayed as 1:32.

---

## 2. Files

| Concern | File |
|---|---|
| Resolver, client ladder, force modes | `ui/js/youtube.js` |
| Retry ladder, class selection, client report | `ui/js/modules/downloads.js` |
| PO-token mint (BotGuard) | `ui/js/modules/po_token.js` |
| Mint diagnostics, 16-step trace, snapshot taxonomy | `ui/js/modules/po_diagnostics.js` |
| Web vs non-web token scoping | `ui/js/modules/pot_scope.js` |
| Downloader, byte accounting, range top-up | `src/infrastructure/media/downloader.rs` |
| Completeness gate | `src/infrastructure/media/completeness.rs` |
| Container forensics | `src/infrastructure/media/forensics.rs` |
| Range top-up (416/403/400 classification) | `src/infrastructure/media/range_topup.rs` |
| MediaStore dual-save | `src/infrastructure/media/android_downloads.rs` |
| Tag writing | `src/infrastructure/media/tags.rs` |
| Download commands, PO injection, client report | `src/commands/downloads.rs` |
| Vendored `youtubei.js` + node shims | `ui/vendor/` |
| Vendored bgutils 4.0.3 | `ui/vendor/bgutils/` |

---

## 3. The resolver: clients and the class ladder

### 3.1 Client groups

Derived from three named groups rather than a hand-written array, because the
order *is* the rule:

| Group | Clients | Why |
|---|---|---|
| `SERVABLE_CLIENTS` | `MWEB`, `WEB`, `WEB_SAFARI` | the only clients a Web/BotGuard PO token is valid on |
| `TOKEN_FREE_CLIENTS` | `ANDROID_VR`, `TV` | need no token |
| `UNMINTABLE_CLIENTS` | `IOS`, `ANDROID` | need DroidGuard/iOSGuard, which we cannot mint |

`IOS` **does** resolve with real audio URLs; the requirement is on GVS, not format
discovery. It is kept, not deleted.

### 3.2 URL classes

The ladder walks **classes of URL**, not error types, in this order:

```
adaptive (itag 140 m4a)  →  audio-only opus (itag 251 webm)  →  muxed (itag 18 mp4)
```

Rotation to another client is the **last** rung, because a byte-0 403 from
`googlevideo` is not client-specific.

Force modes (`forceLegacyProgressive` / `forceOpusAudio`) are **exclusive** and
set on every attempt from `nextClass` alone. They were not, until v2.6.62 — a
stale flag made the ladder request opus three times instead of reaching muxed.

---

## 4. Measurements — per class, per track

**Read this alongside §0: the classes below all failed on `yF9nmg_jHNs`, but on
`94WoNQyK_KY` the muxed rung completed at 100 %.** Outcome is per track, not per
class — which is the other half of why the "all classes fail" framing was wrong.

Device run, `yF9nmg_jHNs` (216.4 s), residential Jio. v2.6.64.

| # | class | client | itag | outcome |
|---|---|---|---|---|
| 1 | adaptive audio | `ANDROID_VR` | 140 m4a | `HTTP 403` at byte 0 |
| 2 | audio-only opus | `ANDROID_VR` | 251 webm | `HTTP 403` at byte 0 |
| 3 | muxed | `ANDROID_VR` | 18 mp4 | windowed |
| 4 | muxed | `ANDROID` | 18 mp4 | windowed (final error) |

### 4.1 The window, in detail

The muxed object is **complete and self-consistent**:

```
received 10992443 bytes, every advertised byte present
container=mp4-stbl  table=216.3s  audio_data_end=10992443B (exactly EOF)
decoded=75s  measured=54.4s  audible_until=54.4s  (2400256 of 2400256 samples, 44100 Hz)
```

The sample table describes 216.3 s and the last byte is at EOF — and only 54.4 s
is audible. So the server windowed the **media, not the transfer**. Range top-up
confirms the object genuinely ends there:

```
url+header: HTTP 416   header: HTTP 416 ("the object ends at 10992443 bytes")   url: HTTP 400
```

**This is the gate working.** It refused to save a 54 s file as a 216 s track and
named the correct cause. If a future transport yields a window, that is the
answer — **do not weaken the gate to accommodate it.**

---

## 5. SABR

### 5.1 Contract (read-from-source + measured)

Sources: `protos/video_streaming/video_playback_abr_request.proto` and
`src/core/SabrStream.ts` in **LuanRT/GoogleVideo**.

- **Body**: `VideoPlaybackAbrRequest` protobuf (proto2). `initialization_format_ids`,
  `selected_audio_format_ids`, `streamer_context` (carries `po_token`),
  `video_playback_ustreamer_config`.
- **URL**: `serverAbrStreamingUrl` + `&rn=<requestNumber>`
- **Method**: POST, `Range` header removed
- **Headers**: `content-type: application/x-protobuf`, `accept: application/vnd.yt-ump`,
  `accept-encoding: identity`
- **Framing**: UMP — `varint(partId) varint(size) payload`. `MEDIA_HEADER=20`,
  `MEDIA=21`, `SABR_REDIRECT=43`, `SABR_ERROR=44`, `SABR_CONTEXT_UPDATE=57`,
  `STREAM_PROTECTION_STATUS=58`.

Both inputs we were unsure about are **already present** in our player response:

- `player_config.media_common_config.media_ustreamer_request_config.video_playback_ustreamer_config`
  (1560 chars, base64)
- `signatureTimestamp` = 20719

The audio formats are **SABR-only**: `itag 140/249/250/251`,
`content_length=3503522`, `approxDuration=216433ms` (the full 216 s), and **no
`url` field at all**. The bytes are not obtainable by plain HTTP GET.

### 5.2 The gate

```
[ERROR] [SabrStream] Cannot proceed with stream: attestation required
```

Traced to source: **not** a client-side guard. It is the server's own
`STREAM_PROTECTION_STATUS` UMP part (58) with `status === 3`, thrown at
`SabrStream.js:776-787`. The request was well-formed and selected itag 140 before
the refusal.

| token supplied | server response |
|---|---|
| none | `status=3` — attestation required |
| cold-start (locally generated) | `status=3` — attestation required |

*Weakness in the second row:* in that run `visitor_data` was `undefined`, so the
token was generated unbound (16 chars). A correctly-bound cold-start token is
**not** cleanly tested. The direction is still what the name guarantees — a
cold-start token is by construction not attested.

Per bgutils' own documentation: **status 2** = "a PO Token is required, but the
client can request up to 1–2 MB using a cold start token before playback is
interrupted"; **status 3** = "the client cannot continue fetching media data
without a valid PO token." **A cold-start token is what the client already has
when status 2 is reported — it cannot satisfy status 3.**

---

## 6. The PO token — was the single blocker; not on the path that works

### 6.1 How we try to get one

```
page-context-probe → bgutils-import → attestation-challenge → visitor-data
→ interpreter-url → interpreter-fetch → new-function-eval → botguard-global
→ botguard-load → snapshot → generate-it → mint → cache-write
```

Everything through `snapshot` succeeds. On device (v2.6.61 trace): `new Function`
works, 24/24 globals present, a 63 570-byte interpreter compiled and ran in 27 ms,
`globalThis.trayride.a` is a function, the VM handshake completed.

### 6.2 The failure

```
FAIL snapshot   snapshot returned string(len=3160) but pushed NOTHING into
                webPoSignalOutput (length=0, still 0 after waiting 609ms)
                shape=empty-array · settleGrew=false · responseType=string
```

`webPoSignalOutput[0]` must be a `getMinter` function. It is never populated.
`PMD:Undefined` is what `WebPoMinter.create` throws when that slot is `undefined`.

### 6.3 Hypotheses, and what happened to each

Every one of these was **tested**, not argued:

| # | Hypothesis | Test | Verdict |
|---|---|---|---|
| 1 | The push is a race — it happens after `snapshot()` resolves | 600 ms bounded re-read, on device | **Falsified** — `settleGrew=false` |
| 2 | The challenge from `/att/get` is stale; use the TV client's (`BgUtils#44`) | Full TV-config path, offline, 31 647-byte program | **Falsified** — `length=0`, identical |
| 3 | We pass `snapshot({contentBinding})` wrong | 6 contentBinding variants | **Falsified** — `len=0` and **`respLen` identical (2971)** in all six; the VM ignores the argument |
| 4 | The minter moved into the snapshot response | Dumped the response | **Falsified** — opaque `$pzg5…` blob, not JSON |
| 5 | A non-function is at `[0]` and `PMD:Undefined` conflates it | `SNAPSHOT_SHAPES` taxonomy (v2.6.64) | **Falsified** — length is 0, not populated |
| 6 | Our vendored bgutils is behind | Checked releases + `main` source | **No upgrade exists** — 4.0.3 *is* the latest; `main` uses the same `webPoSignalOutput` contract |
| 7 | A user-supplied Settings token would work | Logic + bgutils docs | **Only if BotGuard-minted**; a cold-start token does not satisfy status 3 |
| 8 | The VM aborts because it detects a *"barren environment"* (`jsdom`/`node-canvas` fixes it) | Compared the claim against our own device trace | **Refuted** — our mint runs in the **real Android WebView**: eval works, 24/24 globals, 63 570-byte interpreter compiled and ran, VM handshake completed, array still empty. Not barren — *partial*. |

**Corroboration from upstream:** LuanRT closed their own issue
(`LuanRT/BgUtils#48`, *"Generates an invalid (?) poToken"*) as **"not planned"**,
stating of `WEB`/`MWEB`: *"even then it might fail because it recently
transitioned to SABR-only and the client version ytjs uses just happens to be
outdated."*

**Reading, labelled as inference:** the web clients may no longer be mintable
from outside. The cited projects that "applied the BgUtils#44 fix" applied it to
a path that our measurements show does not reach our symptom.

---

## 7. The dev box reproduces the failing network

**The most operationally important fact in this document.** Verified first-hand:

```
ipinfo.io    -> 152.58.59.240   AS55836 Reliance Jio Infocomm Limited   Bhopal, IN
phone report -> 2409:40c4:f9ab:20c:88e5:d5d6:54d0:d75b
URLs YouTube hands THIS BOX carry
               ip=2409%3A40c4%3Af9%3Ab20c%3A88e5%3Ad5d6%3A54d0%3Ad75b   <- the phone's address
```

The box and the phone are on the same home connection and YouTube binds issued
URLs to the same address for both. **YouTube work no longer needs a device
round-trip to iterate on.**

### 7.1 Running the vendored `youtubei.js` under node

The shims already committed in `ui/vendor/` (`process.mjs`, `events.mjs`,
`async_hooks.mjs`, `tty.mjs`) make this work with no bundler:

```js
// scratch dir = copy of ui/vendor/ + package.json containing {"type":"module"}
const { Innertube, UniversalCache } = await import('./vendor/youtubei.esm.mjs');
const it = await Innertube.create({ retrieve_player: true,
  generate_session_locally: true, cache: new UniversalCache(false),
  fetch: globalThis.fetch });
const info = await it.getInfo('yF9nmg_jHNs');
// -> playability_status OK, 28 adaptive formats, server_abr_streaming_url present
// -> it.session.player.signature_timestamp === 20719
```

Gotchas, each of which cost a run:

- **`Platform.shim.eval = async (d) => new Function(d.output)()`** is required,
  or `decipher` throws *"you must provide your own JavaScript evaluator"*.
- **`await stream.start(...)`** — it is async. Omitting the `await` does not
  throw; it silently yields `{}` for the selected formats, which reads like
  "no audio found".
- **`{"type":"module"}` in the scratch dir.** The repo-root `package.json` has no
  `type`, so `node --check` on any `ui/js/**/*.js` fails with
  `Unexpected token 'export'`. That is a **harness artifact, not a syntax
  error** — it has been misread as a real failure more than once.
- **No package manager on this box** — no `npm`/`pnpm`/`yarn`/`bun`. Fetch
  tarballs directly:
  `curl -sL <registry tarball> | tar xz -C <pkg> --strip-components=1`.
- **The audio formats are SABR-only.** Do not conclude from a node run that
  "the URLs work".

### 7.2 The SABR reference implementation

`googlevideo@4.1.1` from the npm registry, single dependency
`@bufbuild/protobuf`. Run this way it reproduced the `status=3` refusal locally,
which is how §5.2 was established without a device.

**Do not vendor it before a token works** — the transport is worthless without
one. When that day comes, the split should be: **JS issues SABR and streams
bytes into the existing Rust downloader**, so byte accounting, forensics, and
the completeness gate all keep applying. SABR needs the player nsig decipher,
which needs `new Function`, so it cannot live in Rust.

---

## 8. Cleared — not the problem

Recording these so nobody re-litigates them:

- **The server does not window the media.** The sharpest thing this project got
  wrong, and it is settled: `yF9nmg_jHNs` is a complete 10 992 443-byte file whose
  audio sample table declares 216.34 s with the last sample inside the file. Our
  decoder read 25.2 % of it. **`AGENTS.md` §4.7.13 has the measurement.**
- **The PO token is not required for muxed.** `itag 18` completed with the token
  explicitly withheld. SABR is off the critical path.
- **`~60 s` is not a SABR cutoff here.** The `LuanRT/GoogleVideo#52` citation is
  still a real library limit and still worth quoting — it is simply *not our
  symptom*.

- **The 403 is not a header problem.** Client-matched `UA`/`Referer`/`Origin` are
  all injected (`commands/downloads.rs`). A Web token was once appended to a
  non-web client's URL; that was fixed in v2.6.50 and is not recurring.
- **The window is not a transfer failure.** Every advertised byte arrived.
- **The player is not truncating tracks.** It was, once (v2.6.46): rodio
  under-reports duration, the progress bar filled early, and `seek()` refused
  past it. `reconcile_duration` now lets the decoder only *raise* a duration.
- **Cover art is not broken.** `assetProtocol` was missing from
  `tauri.conf.json` entirely; enabled in v2.6.47.
- **The retry ladder is not broken.** It reached all three classes correctly on
  v2.6.64, in order, exactly once each.
- **The dev box is not a different network.** Same line as the phone.

---

## 9. Open questions

Ordered by information-per-effort.

0. **Does the gate accept a complete file with a short decode, and still reject a
   genuinely short one?** Both directions, or it has merely been made permissive.
   In flight.

1. **Does a correctly visitorData-bound cold-start token satisfy status 3?** Our
   test was imperfect (`visitor_data` was `undefined`, so the token was unbound).
   If the answer is yes, a user-supplied token becomes a real shortcut. Cheapest
   thing left to try.
2. **Does anything other than a BotGuard-minted token satisfy status 3?** Cookies
   or login state, a different client, cold-start plus `X-Goog-Visitor-Id`. A
   negative result is still worth having.
3. **Is there a maintained fork or a third-party token source?** bgutils 4.0.3 is
   the last release; LuanRT marked their own report "not planned". If a fork
   exists, that is the most likely route.
4. **Is the web family still mintable at all?** `web` is SABR-only, so even a
   minted token would need the SABR transport. Confirm rather than assume.
5. **What exactly does live BotGuard do differently?** Unreadable — it is a
   runtime blob Google serves. The only lever is observing inputs/outputs.

---

## 10. Reproducing the SABR refusal offline

```bash
mkdir -p /tmp/sabrtest && cd /tmp/sabrtest
printf '{"name":"sabrtest","private":true,"type":"module"}\n' > package.json
mkdir -p node_modules/@bufbuild/protobuf
curl -sL https://registry.npmjs.org/googlevideo/-/googlevideo-4.1.1.tgz \
  | tar xz -C node_modules --strip-components=1 2>/dev/null || {
  mkdir -p node_modules/googlevideo
  curl -sL https://registry.npmjs.org/googlevideo/-/googlevideo-4.1.1.tgz \
    | tar xz -C node_modules/googlevideo --strip-components=1; }
curl -sL "$(curl -s https://registry.npmjs.org/@bufbuild/protobuf \
  | python3 -c "import json,sys;d=json.load(sys.stdin);l=d['dist-tags']['latest'];print(d['versions'][l]['dist']['tarball'])")" \
  | tar xz -C node_modules/@bufbuild/protobuf --strip-components=1
# copy ui/vendor/ to node_modules/youtubei.js/ and add a package.json with
# { "type": "module", "main": "youtubei.esm.mjs" }
```

Then: request the player with `client: 'WEB'`, build a `SabrStream` with
`serverAbrStreamingUrl` + `videoPlaybackUstreamerConfig`, `await stream.start({...})`,
and read `audioStream`. Expect `Cannot proceed with stream: attestation required`.

---

## 10b. Decision record (2026-09-29)

- **Sidecar (`bgutil-ytdlp-pot-provider` on `127.0.0.1:4416`) — REJECTED for
  shipping.** It needs a Node runtime *on the device*, and the target is Android.
  Shipping a Node runtime inside an APK is a category error, not a tradeoff. At most
  it is a dev-box-over-LAN workaround while we develop.
- **WebView approach — CHOSEN, and unproven.** Load the real watch page, let
  YouTube's own `botguard.js` mint in its intended context, extract the result. It
  does not fix emulation, it removes the problem.
- **Where the token travels (do not get this wrong):** on SABR it is
  `streamer_context.po_token` **inside the protobuf body**, not a header. The
  `?pot=` / `/pot/<token>` URL form is the *non-SABR* CDN path. Reading a header, or
  grepping a SABR URL, finds nothing — and a null there is indistinguishable from
  "no mint".
- **Agreed spike:** one Tauri command — hidden `WebviewWindowBuilder` window
  (`visible(false)`), `initialization_script` patching `fetch` **and**
  `XMLHttpRequest` before load, load a watch page, capture `(url, method, headers,
  body)`, parse `po_token` out of the body, return to Rust. No downloader changes.
  Both transports, because we do not know which the player uses and a null we cannot
  interpret is what cost the previous fortnight.

---

## 11. Evidence discipline

Applies to everything above, and enforced in review.

1. **Claims about the outside world need the quoted cell and its origin.** A
   paraphrase is not a citation. If the sentence cannot be produced, the answer
   is **"unknown"**.
2. **Label confidence separately from content** — read-from-source vs inferred.
   Label inferences *especially when they look probably right*.
3. **Measurement beats citation.** On conflict, mark the citation contradicted
   rather than deleting it — the surviving wrong row is the evidence the belief
   existed.
4. **Need a fact nobody has quoted? Dispatch a reader** and require the quoted
   cell back.
5. **Do not implement on a summary.** The `EVENT_ID` / `BgUtils#44` fix was
   relayed by an audit agent, checked against the PR by hand — and the PR turned
   out to be `examples`-only with no library API. Reading the primary source
   changed the implementation. Then measuring it changed the answer.

**The failure pattern worth remembering.** Four confident-but-wrong beliefs
surfaced in this project within one month:

| Belief | Owner | How it died |
|---|---|---|
| `android_vr` returns only muxed `itag 18` | a yt-dlp citation | refuted by our own client report |
| `tv` has a DRM caveat that changes ordering | `@audit`, a quoted guide cell supplied *after* the code was written | re-reading the table |
| The 403s and the window are independent problems | me, `@audit` agreed | measured: SABR is gated on the same token |
| `forensics.rs:410` has an off-by-four | a subagent — **fabricated** | `grep` + `git diff HEAD` |

In every case the claim arrived already carrying an explanation, so nobody
re-derived it. The countermeasure that worked was not any of the beliefs: it was
that each one eventually got **grepped** instead of believed.

---

## 12. Not downloads, still open

Unrelated to the above, and all unconfirmed on device:

- `Download/Auralis/` publish — v2.6.57 attached a `publish_error` naming the
  failing JNI call; no report has surfaced it yet.
- Resume — mitigation shipped in v2.6.60 with a `strategy=`/`pre=`/`file=`/
  `replay=` log in the queue panel; not yet confirmed.
- Download → Home redirect — mechanism falsified; a generation guard shipped in
  v2.6.48; needs a device re-test.
- `include_patterns` — a real user setting (`settings.rs:139`) that
  `DesktopScanner` stores and never reads. **A setting that lies.** Owed, not
  dead code.
