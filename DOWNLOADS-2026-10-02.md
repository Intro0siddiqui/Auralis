# Downloads — findings 2026-10-02

Status: **the `ip=` thread is closed by measurement.** The 403's remaining cause is
**still unknown**, and the last WebView-only state we have not tested is named at the end.

---

## 1. The headline: the CDN does not check the client's source address

`examples/fetch_probe.rs` fetches one googlevideo URL several ways. Run against a freshly
resolved `Ral6kFSx7ZY` (muxed itag 18, `client: ANDROID`), on the device itself:

```
A reqwest + app headers + Range   (what the app actually sends)  HTTP 206
B reqwest + UA + Range           (no Referer/Origin/Sec-Fetch)  HTTP 206
C reqwest + bare, no UA, Range   (reqwest's own default)        HTTP 206
D reqwest + app headers, NO Range                                HTTP 200 (whole file)
E reqwest + HTTP/1.1 only + app headers                          HTTP 206
```

Headers, TLS/HTTP2 fingerprint and HTTP version are all **exonerated**. The app's exact
header set returns 206.

### The decisive one

```
bind (OS-chosen)                              -> HTTP 206
bind 2409:40c4:2145:d44a:6018:1fa5:4a7a:2fa0  -> HTTP 206   <- this is ip=
bind 2409:40c4:2145:d44a:dc6e:9ff:fe42:44ac   -> HTTP 206   <- a DIFFERENT local address
bind 2409:40c4:2145:d44a:ad74:5a0d:1155:402b  -> CONNECT FAILED   <- rotated away
```

**Row 3 is the load-bearing result.** Same URL, egressing from a different IPv6 address
than the one in `ip=`, and it still serves. There is no source-address binding.

### Row 4 is why the pin should not exist at all

`ip=` names the **privacy** address, and RFC 4941 rotates it. This session watched it rotate:

```
earlier resolve:  2409:40c4:2145:d44a:ad74:5a0d:1155:402b
fresh resolve:    2409:40c4:2145:d44a:6018:1fa5:4a7a:2fa0
```

Pinning to an address that has rotated away is a **connect failure on a download that would
otherwise have succeeded**. The pin is not merely useless — in that case it is actively
harmful, and harmfully in the same way v2.6.68 already was.

---

## 2. What the tampering experiment was actually showing

§4.7.16 of AGENTS.md changed `ip=` to `203.0.113.7` and got a 403, then concluded *"the CDN
checks the source address."*

It does not. `ip` is listed in **`sparams`**, so it is covered by the signature. Editing
`ip=` invalidates `sig`/`lsig`. That 403 was **signature validation failing**, not an address
check.

This is exactly what `@audit` wrote in the research (issue.md, 2026-10-02):

> The URL is cryptographically bound to that IP — the `sig` parameter covers `ip` in
> `sparams`, so you can't change it without invalidating the signature.

**The research had it right.** The pin was built on §4.7.16's inference over it.

---

## 3. The pin's history, in three versions

| version | call | measured |
|---|---|---|
| v2.6.68 (shipped) | `resolve_to_addrs(host, ip)` | **connect failure, 3/3 tracks** |
| 2026-10-02 fix | `local_address(ip)` | 206 — same as unpinned, buys nothing |
| measurement says | **no pin** | and removes a failure mode |

`resolve_to_addrs(host, ip)` means *"to reach this host, dial this address"*. Since `ip=` is
**the client's** address, it asked the phone to **be** the CDN. It could never connect.

That is why the device moved from `403 Forbidden, ct=text/plain, body ""` to
`error sending request` at byte 0. Not a new fault — the same fault by a worse route.

---

## 4. Two real bugs found on the way

### 4.1 The retry ladder never saw a transport failure

```js
is403       = errRaw.includes('403') || errRaw.includes('Forbidden')
isTruncated = /Truncated download/i
isResumable = /Incomplete download|Stream interrupted|timed out|timeout|
              stalled|ECONNRESET|connection reset|HTTP 5\d\d/i
```

`reqwest` reports **every** connect, DNS and TLS failure as `error sending request` with no
status code. That matches none of the three, so the gate did `map.delete` + `return` —
**zero retries**, with `MAX_AUTO_RETRIES` never consulted.

The owner read it as "the budget ran out after one retry". It had never entered the ladder.
Fixed: transport failures are retry-worthy and set `rotate`, since the next rung is a
different CDN hostname that may connect when this one did not.

### 4.2 `CLASS_ORDER` was backwards

It tried `adaptive` first, on the stated grounds that muxed *"has never succeeded"* — falsified
twice (the truncations were our own decoder miscounting a complete file; muxed then completed
end to end). Measured now: **muxed 206, adaptive itag 140 403 at byte 0**.

The ladder was spending attempt #1 on the class that cannot work. Now
`['muxed', 'adaptive', 'opus']`. Adaptive is the *better* file and should return to first
place the day it stops 403ing.

---

## 5. What is still unexplained

**The app 403s where `curl` and `reqwest` both 206 — same phone, same line, same track,
same class.**

Excluded by measurement:

- source address — row 3 above
- headers — variant A above
- TLS / HTTP2 fingerprint — variants A and E above
- `Range` — variant D
- format class — adaptive *is* 403 at byte 0, but muxed also 403'd on the device, so the
  class explains the ordering fix and **not** the failure

### The last WebView-only state: cookies and session

The pipeline has two HTTP stacks that share nothing:

| stage | who runs it | stack | cookies |
|---|---|---|---|
| resolution | `youtube.js` → `fetch` in the WebView | Chromium, WebView process | YouTube's full jar + `visitorData` |
| transfer | `reqwest` in Rust | hyper + rustls, our process | **none** |

This split is the only place WebView-only state can reach the transfer, and it is the one
thing left untested.

**Proposed test:** forward the WebView's `youtube.com` cookies into the download request as a
`Cookie` header, then re-run `fetch_probe` with and without them. If they change nothing,
cookies are excluded too and the divergence is somewhere we have not looked yet.

**The alternative worth naming:** move resolution into Rust so one HTTP client is used
end to end. That removes the split by construction rather than by measurement. Larger, and
now the more attractive option precisely because the cookie path is the last one standing.

---

## 6. Two structural notes

**Why reqwest at all** — the transfer has to be Rust: byte accounting, `range_topup`, the
completeness gate, staging + atomic rename, `tags.rs`, MediaStore publish. WebView streaming
would move all of it to JS and leave no staged file to promote.

**A fourth falsified belief, and the shape they share.** `_display_name` (§4.7.11), the
decoder (§4.7.13), the PO token (§4.7.10) and now the `ip=` binding are all
**a correct mechanism trace sitting on an unchecked premise**. In each case the mechanism
research was good — and that is what made the premise stop being examined.

---

## Reproducing

```bash
# 1. fresh URL (they expire)
cd /tmp/opencode/yts && node fresh.mjs Ral6kFSx7ZY > /tmp/opencode/fresh_url.txt

# 2. header / client variants
cargo run --example fetch_probe -- "$(cat /tmp/opencode/fresh_url.txt)" Ral6kFSx7ZY

# 3. the egress-binding test — substitute this device's own addresses
cargo run --example fetch_probe -- "$(cat /tmp/opencode/fresh_url.txt)" Ral6kFSx7ZY \
  --bind "none,<ip= value>,<other local address>,<rotated-away address>"
```

Row 4 only produces `CONNECT FAILED` while that address is still absent from the device.
Once the OS has re-issued it, that row becomes another 206 — which is itself worth watching,
because it would mean the address came back into service and the rotation is a cycle rather
than a one-way move.
