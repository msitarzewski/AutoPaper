# Security review, 2026-10-10 (AutoPaper 0.1.3)

This is the dated 0.1.3 review, preserved with its original findings. Remediation is recorded in the
[October 10 follow-up](#follow-up-fixes-and-re-review-2026-10-10); the later
[0.2.0 release addendum](#follow-up-020-release-checks-2026-10-2021) records dependency applicability and
package/publication checks without treating them as a new penetration test.

A defensive review of the code and release chain by nine read-only reviewers, one per surface, run with a multi-agent workflow. Every finding rated above *info* that a reviewer classed as a **vulnerability** was then checked by independent verifiers told to refute it. Findings the reviewers classed as **hardening** were not independently re-verified; each carries the reviewer's file:line evidence. This is not an external penetration test.

**Result: no critical, high or medium vulnerabilities were confirmed.** Four items were claimed as vulnerabilities; verification downgraded all four to *info* (three are documentation inaccuracies; one is a narrow Console link). One *medium* hardening gap: the Flatpak publish step signs a bundle on a checksum from the same CI artifact.

## Summary

| Id | Kind | Severity | Verification | Finding |
|---|---|---|---|---|
| core-net-1 | hardening | low | not re-verified | Persisted/logged provider error text is scrubbed only by a heuristic redactor, so an echoed key can survive |
| core-net-2 | hardening | low | not re-verified | `.local` / `.localhost` names are treated as local without checking what they resolve to, so key-bearing plain http (and the proxy bypass) can reach a non-local address |
| core-net-3 | hardening | info | not re-verified | Response bodies are buffered with a large pre-allocation and several full copies; a hostile OpenAI-compatible server can force a few hundred MB of transient memory |
| core-net-4 | hardening | info | not re-verified | Cleartext Bearer keys to private/LAN addresses are accepted by design |
| core-parse-1 | hardening | low | not re-verified | Image decode allows roughly 1.5 GB RSS from a few-MB file (8 bytes/pixel budget, 64 MP, plus metadata allocations) |
| core-parse-2 | hardening | low | not re-verified | Data directory, SQLite history and image files are created with default umask permissions (0755/0644) |
| core-parse-3 | hardening | low | not re-verified | Provider image bytes are stored verbatim and later handed to OS image decoders, so core validation does not cover what the apps actually parse |
| core-parse-4 | hardening | info | not re-verified | Parsed model fields other than title and prompt are unbounded in length and are stored, embedded and hashed in full |
| core-parse-5 | hardening | info | not re-verified | ComfyUI progress watcher keeps one map entry per node id sent over /ws and treats any event as activity |
| core-parse-6 | hardening | info | not re-verified | Embedding model files are loaded without an integrity check and the Linux build can load them from an env var or a compile-time source path |
| secrets-1 | hardening | low | not re-verified | Error and log redaction is heuristic only; an echoed key can reach system logs, and JSON field names are never scrubbed |
| secrets-2 | hardening | low | not re-verified | Local database with prompts, history and Console traces is created with the default umask (no 0600/0700) |
| secrets-3 | hardening | info | not re-verified | oo7 silently falls back to an unencrypted Secret Service session; keys are never zeroised in memory |
| macos-1 | vulnerability | low | downgraded→info | Console renders model-written text as Markdown, so a model can plant clickable links of any URL scheme |
| macos-2 | hardening | low | not re-verified | Untrusted provider image bytes are stored as-is and then re-parsed by Apple ImageIO inside the app process |
| macos-3 | hardening | low | not re-verified | Sparkle feed is unsigned and verify-before-extraction is not enabled |
| macos-4 | hardening | low | not re-verified | release.sh passes the Apple app-specific password on the command line |
| macos-5 | hardening | low | not re-verified | Sparkle, the update-trust root, is pinned only by tag in a git-ignored project |
| macos-6 | hardening | info | not re-verified | Sparkle private signing key is usable without a prompt by any same-user process via the 'Always Allow' ACL |
| windows-1 | hardening | low | not re-verified | Signing VM has the whole Mac home directory mounted next to a live Azure signing session |
| windows-2 | hardening | low | not re-verified | Release build does not enforce lockfiles (cargo --locked, NuGet lock/hash pinning) |
| windows-3 | hardening | info | not re-verified | Full-trust package gives no isolation: Credential Manager keys and the data folder are open to any same-user process |
| windows-4 | hardening | info | not re-verified | autopaper_core.dll is built without Control Flow Guard and embeds a build-machine path |
| linux-1 | hardening | low | not re-verified | Signed Flatpak releases rest on unauthenticated CI output (checksum from the same artifact, no provenance, mutable action and container refs) |
| linux-2 | hardening | low | not re-verified | Repository signing key has no passphrase and no expiry, and no revocation or rotation path is documented |
| linux-3 | hardening | low | not re-verified | App data directory permissions follow the process umask; AutoPaper does not restrict them itself |
| linux-4 | hardening | info | not re-verified | Flatpak cache and state restore plus floating container tag in the release workflow (CI cache poisoning surface) |
| supply-chain-1 | hardening | low | not re-verified | GitHub Actions pinned to mutable tags/branches, not commit SHAs |
| supply-chain-2 | hardening | medium | not re-verified | Flatpak publish signs CI bundles on a checksum that comes from the same artifact (integrity, not authenticity) |
| supply-chain-3 | hardening | low | not re-verified | Sparkle dependency and its sign_update tool are resolved by version tag with no committed resolution (Package.resolved is not tracked) |
| supply-chain-4 | hardening | low | not re-verified | Notarization app-specific password and dev-signing PFX password passed on command lines (visible in process list) |
| supply-chain-5 | hardening | low | not re-verified | Sparkle appcast is unsigned: release-note HTML and release-page links are unauthenticated |
| supply-chain-6 | hardening | info | not re-verified | pages.yml grants pages: write and id-token: write to the build job that runs pandoc and repo scripts |
| supply-chain-7 | hardening | info | not re-verified | uniffi-bindgen-cs installed from a git tag at build time (build tool for the Windows C# bindings) |
| supply-chain-8 | hardening | low | not re-verified | Flatpak repository signing key is an unencrypted GPG key with no expiry, kept on the release Mac and mounted into Docker |
| supply-chain-9 | hardening | info | not re-verified | Flatpak build cache restored with a prefix restore-key; cache key excludes source |
| deps-1 | hardening | low | not re-verified | Windows release builds the C# bindings with an unlocked, tag-pinned `cargo install` from git |
| deps-2 | hardening | low | not re-verified | CI and release builds use floating action tags and an unpinned Rust toolchain, and there is no dependency-audit gate |
| deps-3 | hardening | low | not re-verified | NuGet dependencies for the Windows app are not locked, and transitive versions float |
| deps-4 | hardening | info | not re-verified | The embedding model is verified at fetch and in Flatpak, but not when it is bundled or loaded |
| deps-5 | hardening | info | not re-verified | RUSTSEC-2024-0436: `paste` 1.0.15 is unmaintained |
| deps-6 | hardening | info | not re-verified | No `#![forbid(unsafe_code)]` guard in the workspace crates |
| privacy-claims-1 | vulnerability | low | downgraded→info | Mood keywords are written to system logs although PRIVACY.md says no keywords appear in any diagnostic log |
| privacy-claims-2 | vulnerability | low | downgraded→info | 'Never leaves your computer' and 'not even your keywords' claims are broader than the data flows |
| privacy-claims-3 | vulnerability | low | downgraded→info | Descriptions of recent wallpapers from every mood are sent to the writer, so other moods' content reaches a provider |
| privacy-claims-4 | hardening | info | not re-verified | macOS sends Locale.current.identifier(.bcp47) with Unicode extension subtags, more than 'language and region' |
| privacy-claims-5 | hardening | info | not re-verified | macOS sandbox description in PRIVACY.md understates the entitlements |
| privacy-claims-6 | hardening | info | not re-verified | Documentation gaps and stale statements in PRIVACY.md and NETWORK.md |

## Details

### Surface: core-net

#### core-net-1: Persisted/logged provider error text is scrubbed only by a heuristic redactor, so an echoed key can survive
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/net.rs:469-508; core/src/net.rs:457-466; core/src/engine.rs:2627; core/src/engine.rs:2652; core/src/engine.rs:1866; core/src/engine.rs:2475

error_for_status/error_message put the first 200 chars of a server's JSON error message into AutoPaperError::InvalidResponse after net::redact(). redact() only recognises tokens that start with sk- (>=20), AIza (>=30), or are >=32 chars, contain a digit and a letter and no '.', and are not name-like. In engine.rs record_failure the error is run through the same heuristic only (net::redact(&error.to_string())), then logged via tracing::warn! and persisted into the Generation row (error: Some(detail)). The Console path is stronger because ConsoleHttp collects the exact credential header values and replaces them (engine.rs:1284-1291, 1334, 1370-1371), but record_failure and the other tracing sites do not use that exact-match list. A key that is shorter than 32 chars, contains a dot, is letters/hyphens only, or has an unrecognised shape is not redacted.

**Attack scenario.** The user points the OpenAI-compatible provider at a gateway or local server (or a compromised one) that answers a 4xx with a body like {"error":{"message":"Invalid API key: <the Bearer value it just received>"}}. The key is not matched by redact(), appears in the failed Generation's stored error text (SQLite), in the app log, and is shown in the UI/history. The server already holds the key, so this is local persistence/shoulder-surf exposure only, not a new remote disclosure.

**Evidence.** Standalone copy of net::redact() run in scratch: 'gw-abcdef1234567890abcdef12345678.sig' (dotted, 38 chars), 'aB3dE5gH7jK9mN1pQ3rS5tU7vW9x' (28 chars mixed) and 'correcthorsebatterystaplecorrecthorse' were all returned unchanged; a 32-char hex key and a hyphenated key with digits were redacted. engine.rs:2627 uses redact() alone; engine.rs:2652 stores the result.

**Recommended fix.** Make error scrubbing exact-match first: apply RunCapture's remembered credential values (or have the provider/HttpClient layer scrub the request's credential header values from any error detail) in error_for_status callers and in record_failure, keep redact() as a second layer. Optionally drop error bodies entirely for UserEndpoint providers that carry a key.

#### core-net-2: `.local` / `.localhost` names are treated as local without checking what they resolve to, so key-bearing plain http (and the proxy bypass) can reach a non-local address
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/net.rs:49-65; core/src/net.rs:297-307; core/src/net.rs:226-259; core/src/net.rs:167-169

For HostPolicy::UserEndpoint, plain http is allowed when the host is a literal local IP, or any DNS name equal to localhost or ending in .localhost or .local. The decision is made on the name alone. Nothing validates the addresses the resolver returns, for reqwest (client_for -> direct, no proxy, for the same names) or for the ComfyUI WebSocket (TcpStream::connect("{host}:{port}")). On platforms/resolvers where .local or .localhost falls back to unicast DNS (common on Linux without nss-mdns, some Windows setups, split-DNS/VPN), the name can resolve to a routable address.

**Attack scenario.** A user configures an OpenAI-compatible server as http://studio.local:1234 (the documented, tested form). An on-path or rogue DNS responder (public Wi-Fi, hostile LAN) answers the unicast query for studio.local with its own address; AutoPaper sends the Bearer key and prompts in cleartext to it, bypassing the system proxy. Requires the user to have chosen a .local/.localhost http endpoint plus a DNS attacker, hence low.

**Evidence.** check_url accepts 'http://studio.local:8188' (net.rs tests line 536, passing in the 21-test run of `cargo test -p autopaper-core --lib net::`); the code path has no post-resolution check. is_local_destination (net.rs:297) applies the same name-only test.

**Recommended fix.** Resolve the name yourself (reqwest dns_resolver / resolve_to_addrs, and resolve before the WebSocket connect) and refuse plain http unless every returned address passes is_local_ip; or drop .local/.localhost from the http allowlist and require https or an IP literal. At minimum, warn in the UI that .local over http sends the key in clear text.

#### core-net-3: Response bodies are buffered with a large pre-allocation and several full copies; a hostile OpenAI-compatible server can force a few hundred MB of transient memory
- Kind / severity: hardening / info  
- Platforms:   
- Locations: core/src/net.rs:355-366; core/src/providers/openai_compat.rs:29-30; core/src/providers/openai_compat.rs:336-343; core/src/engine.rs:1396-1401

read_body does Vec::with_capacity(declared Content-Length) up to max_response_bytes (72 MB for images) before any byte arrives. The b64_json path then parses the whole body into a String, copies it again in decode_base64 (compact), decodes it (about 54 MB), and in a captured run ConsoleHttp additionally parses the whole response into a serde_json::Value for sanitising. Peak is roughly 250-350 MB per request. The caps (72 MB response, 50 MB image, whole-request timeout) do bound it, and gzip/brotli decoding is not compiled in (no async-compression in Cargo.lock), so there is no decompression bomb.

**Attack scenario.** A malicious or buggy local server the user chose declares Content-Length: 75000000 (rejected) or 72000000 and streams slowly or fully, causing a transient memory spike on each image request. Only memory pressure on a user-chosen server, so info.

**Evidence.** net.rs:359 `Vec::with_capacity(declared.unwrap_or(0))` with declared <= max_response_bytes; Cargo.lock has no async-compression/brotli/zstd, so reqwest sends no Accept-Encoding and does not decode.

**Recommended fix.** Cap the pre-allocation (e.g. min(declared, 1 MB)) and let the Vec grow; avoid the extra `compact` copy in decode_base64 by decoding with a whitespace-tolerant engine or only filtering when needed; skip Value parsing in the Console for bodies over a few MB.

#### core-net-4: Cleartext Bearer keys to private/LAN addresses are accepted by design
- Kind / severity: hardening / info  
- Platforms:   
- Locations: core/src/net.rs:49-65; core/src/net.rs:72-90; core/src/providers/openai_compat.rs:66-78

With UserEndpoint, http is allowed to any RFC1918, unique-local, link-local (including 169.254.169.254) and loopback address, and the OpenAI-compatible provider attaches `Authorization: Bearer <key>` there. Anyone on the same LAN segment can read it. This matches the documented policy (SECURITY.md, net.rs header), and the key is account-scoped to the endpoint origin (model.rs:365-373) so it is never sent to a different server.

**Attack scenario.** User runs a LAN-hosted gateway over http with a real upstream key as the Bearer value; a LAN neighbour sniffs it. Needs the user to choose http on a shared network.

**Evidence.** net.rs tests accept http://192.168.4.25:8188, http://10.0.0.5, http://169.254.1.1 (lines 533-540); openai_compat.rs:76 pushes the authorization header for any UserEndpoint base.

**Recommended fix.** Optional: show a one-time notice in Settings when a key is saved for an http origin, and recommend https; no code change needed for the stated threat model.

**Checked and found sound:**
- Hosted providers (OpenAI, Google): HostPolicy::Hosted enforces https, an exact lower-cased DNS host, no explicit port, no userinfo, and no IP literals (core/src/net.rs:40-48); the test hosted_policy_allows_only_https_to_named_hosts passes, including the api.openai.com.evil.example and 104.18.0.1 cases.
- Redirects: automatic redirects are disabled (net.rs:151) and followed by hand, at most 3, only if the target passes check_url, has the same host as the ORIGINAL URL, and is not an https->http downgrade (net.rs:328-342). Credential headers (authorization, proxy-authorization, cookie, x-goog-api-key, x-api-key, api-key) are dropped on any scheme or port change (net.rs:200-204). Verified by passing tests: does_not_follow_redirects_to_other_hosts_or_disallowed_urls, drops_credentials_when_a_redirect_changes_port, follows_at_most_three_redirects, next_hop_rules (21/21 net:: tests pass, run in scratch with CARGO_TARGET_DIR outside the repo). Every provider credential header (openai.rs:103, google.rs:142, openai_compat.rs:76) is in CREDENTIAL_HEADERS; no other secret-bearing header exists.
- Cookies/referer/cache: no cookie store, referer(false) (net.rs:152), no cache; confirmed by follows_same_host_redirects_without_keeping_cookies.
- TLS: reqwest 0.13 with rustls and rustls-platform-verifier (Cargo.lock); no danger_accept_invalid_certs, custom roots or min-version overrides anywhere in core/src; TLS failures map to InvalidResponse (net.rs:389-408, tested by tls_failure_is_an_invalid_response).
- Decompression bombs: no gzip/brotli/zstd/async-compression in Cargo.lock for reqwest, so Content-Encoding is neither requested nor decoded; body size is enforced both on Content-Length and chunk by chunk (net.rs:355-365), tested by aborts_responses_over_the_size_cap.
- Hangs: a single tokio::time::timeout wraps connect, redirects and body read (net.rs:215-224); 10 s connect timeout; WebSocket connect+handshake also timeout-bound (net.rs:256-257); the ComfyUI wait loop has an overall deadline plus stall windows and re-checks them on every message (comfyui.rs:359-412); max WebSocket message and frame size are set to the request cap (net.rs:241-243).
- URL building / injection: Google model IDs are whitelisted to [A-Za-z0-9-_.], <=128 chars before going into the path (google.rs:651-666), and server-listed IDs are filtered the same way for OpenAI/Google (openai.rs:453, google.rs:664); the Google pageToken is form-urlencoded (google.rs:175); OpenAI paths are constants; ComfyUI builds URLs with path_segments_mut().extend and query_pairs_mut so server-supplied filename/subfolder/type are percent-encoded (comfyui.rs:236-241, 445-452, test at comfyui.rs:2051-2062), and prompt_id is restricted to [A-Za-z0-9_-] before being used in /history and /cancel URLs (comfyui.rs:316-321). Server-supplied filenames are only ever sent back to the same server's /view; they are not used as local paths in this surface.
- Keys never in URLs: OpenAI uses the Authorization header, Google uses x-goog-api-key (not ?key=), compat uses Authorization; check_url rejects userinfo in any URL and header_map refuses CR/LF values without echoing them (net.rs:35-37, 309-323, test rejects_unsendable_headers_without_echoing_them).
- Key scoping: OpenAI-compatible keys are stored per origin (model.rs:365-373), so changing the base URL does not send an old key to a new server; image URLs returned by a compat server are fetched only if scheme, host and port equal the configured origin (openai_compat.rs:169-187), and redirects there are same-host only.
- Debug/Display leaks: HttpRequest has a hand-written Debug that redacts credential headers and omits the body (ports.rs:87-119); transport errors are logged without_url() and through redact (net.rs:371-374); the User-Agent contains no secrets; no tracing/println site in core/src logs headers or keys.
- Console capture: ConsoleHttp records only origin+path (no query/userinfo), no headers, replaces exact credential values and Bearer-stripped values in bodies, redacts sensitive JSON keys, omits image bytes and non-JSON bodies, and keeps the secrets list in memory only (engine.rs:1273-1376, 1183-1189); size-bounded to 255 events / 448 KiB per run (engine.rs:1241).
- Retry logic: there are no transport-level automatic retries in the HTTP layer; provider retries (Google thinking-level) re-send through the same checked client with a fresh key read; Retry-After is capped at 3600 s (net.rs:421-431). OpenAI 429 insufficient_quota is not retried.
- Custom ComfyUI workflow templates: placeholders are JSON-escaped (serde_json::to_string) and substituted prompt-last, so a model-written prompt cannot break out of its JSON string or inject nodes (comfyui.rs:1018-1039); /view, /history, /prompt and the WebSocket all use HostPolicy::UserEndpoint; the WebSocket is plain ws:// to local destinations only, uses the core's own TCP stream (never a proxy), and redirects during the handshake are not followed (net.rs:226-259).
- Settings validation: base_url for every selection is passed through check_url(UserEndpoint) on save and again on every request (engine.rs:3013, net.rs:214).
- Hosts: no macOS/Windows/Linux host implements its own HttpClient or networking; ReqwestClient is the only implementation (engine.rs:312), so there is no host-side bypass of the policy in this surface.

### Surface: core-parse

#### core-parse-1: Image decode allows roughly 1.5 GB RSS from a few-MB file (8 bytes/pixel budget, 64 MP, plus metadata allocations)
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/imaging.rs:68-76; core/src/imaging.rs:101-127; core/src/imaging.rs:220-227; core/src/engine.rs:2196-2218

decode() caps the file at 50 MB, the header dimensions at 16,384 px a side and 64 MP, and sets image::Limits.max_alloc to 64 MP x 8 bytes + 16 MB (about 528 MB). A 16-bit RGBA PNG at 16384x3900 is accepted and decodes to 512 MB. The engine then runs phash, thumbnail and render on it, with to_rgb8 and resize copies. A PNG iCCP/eXIf chunk can add up to a further 528 MB inside the same limit. An animated WebP with a 16384x3900 canvas is accepted at 90 bytes and allocates the full canvas. The cap is bounded and matches the documented limits (NETWORK.md says 64 MP). No provider in the app returns 16-bit images, so the 8 bytes/pixel budget is looser than it needs to be. All of it is attacker-controlled by whatever supplies the image bytes: a compromised provider, or a malicious ComfyUI, Ollama or OpenAI-compatible server the user chose.

**Attack scenario.** A malicious or compromised image source returns a 2.4 MB all-zero RGBA16 PNG at 16384x3900. The core accepts it and the generation pipeline peaks near 1.5 GB RSS. On a small-memory Windows or Linux machine, or when another app is also heavy, the OS kills the process or swaps. The same bytes arrive on every scheduled run, so the app can be pushed into a repeated crash or OOM loop. This needs a hostile image source, which SECURITY.md treats as largely out of scope. It is also bounded: it is not an unbounded allocation, and there was no panic, hang or decompression-bomb blow-up.

**Evidence.** Scratch harness core/examples/bomb.rs, built --release and run under /usr/bin/time -l. Memory: png16 16384x3900 (2,484,417-byte file) gave maxrss 1,556,692,992; png8 8000x8000 (944 KB) gave 648,790,016; jpeg 8000x8000 (1.7 MB) gave 842,334,208; lossless webp 8000x8000 (27 KB) gave 969,359,360. Decode itself took 8-45 ms; phash plus thumbnail plus render took about 1.7 s. examples/webpanim.rs: a hand-built 90-byte animated WebP with a 16384x3900 canvas decodes to 16384x3900 at 453 MB RSS. A 500,000-frame (23 MB) animated WebP gave 501 MB RSS and decoded in 48 ms. examples/pngmeta.rs: a 2 MB file with a zlib-bombed iCCP chunk gave 540 MB RSS; the same chunk with the 64 MP RGBA16 pixels gave 1.57 GB RSS at decode alone. A zTXt/iTXt bomb of 2 GB decompressed was rejected cheaply (17 MB RSS).

**Recommended fix.** After building the decoder and before from_decoder, check decoder.color_type(). Reject 16-bit and float colour types, which the app never needs, or lower MAX_BYTES_PER_PIXEL to 4. Consider lowering max_pixels, for example to about 40 MP, since the app renders at most display size. Cap ancillary-chunk memory separately from the pixel budget. For animated WebP/APNG, reject files where the container reports more than one frame.

#### core-parse-2: Data directory, SQLite history and image files are created with default umask permissions (0755/0644)
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/engine.rs:945-951; core/src/store.rs:343-345; core/src/imaging.rs:297-310; apps/linux/src/desktop.rs:52-54

Engine::open_inner calls fs::create_dir_all for the data, images, thumbs and renders folders. Store::open calls Connection::open, and write_atomic creates files with OpenOptions::create_new. Nowhere in core is a mode, ACL or umask set. On Unix the result is whatever the process umask gives, normally 0755 folders and 0644 files. The database holds every concept and prompt, ratings, taste, keywords and moods, and the Console run records (provider request and response bodies, redacted for keys). The images folder holds the wallpapers. API keys are not stored there. They live in the Keychain, Credential Manager or Secret Service. On macOS the App Sandbox container already blocks other users. In Flatpak the folder is under ~/.var/app, which is user-private in practice. The exposure is therefore the native Linux build, in $XDG_DATA_HOME/autopaper, when the home directory is world-traversable. That is the default on some distributions (0755 home). Other local users could then read the personal history.

**Attack scenario.** On a shared Linux host with a 0755 home directory, another local user reads ~/.local/share/autopaper/autopaper.sqlite3 and learns the owner's keywords, mood names, generated prompts and Console request bodies. No key is exposed.

**Evidence.** core/examples/perm.rs calls Store::open and imaging::write_original in a fresh temp dir under umask 022. Output: 755 images, 644 autopaper.sqlite3, 644 images/a.png, 755 root. No set_permissions or PermissionsExt appears anywhere in core/src or apps/linux/src (grep).

**Recommended fix.** On Unix, create the data root with DirBuilder mode 0700, or set_permissions(0o700) after creation if it already exists. Open the DB with a restrictive umask or chmod 0600 on the .sqlite3, -wal and -shm files, and create image files with OpenOptionsExt::mode(0o600). On Windows, rely on the per-user LocalAppData ACL, which is already private.

#### core-parse-3: Provider image bytes are stored verbatim and later handed to OS image decoders, so core validation does not cover what the apps actually parse
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/engine.rs:2204-2214; core/src/imaging.rs:129-131; apps/macos/AutoPaper/Views/WallpaperImage.swift:45; apps/windows/AutoPaper/Views/NowPage.xaml.cs:289; apps/linux/src/wallpapers.rs:88-94

The core validates provider images with the Rust image crate: sniff, header dimensions and a re-decode. It then writes the original provider bytes unchanged (write_original) next to a re-encoded thumbnail. The apps display the original (imagePath / image_path preferred over the thumbnail) through ImageIO, WIC or gdk-pixbuf, which are different parsers. They are handed attacker-controlled ICC profiles, EXIF, extra chunks, trailing data and polyglot content that the core never inspected. The display renders (render_for_display) are re-encoded, but the original is not. A parser differential between the Rust decoder and an OS decoder is possible: bytes the core accepts as PNG/JPEG/WebP could exercise a bug in the platform decoder. This is defence in depth only. No exploit is demonstrated, and OS decoders already process far more hostile content in normal use.

**Attack scenario.** A compromised hosted provider or a malicious local image server returns a PNG, JPEG or WebP crafted to trigger a memory-safety bug in the platform decoder, with benign-looking structure for the image crate. The core stores it. The History or Now view then opens it with the OS decoder, outside the core's limits and outside Rust's memory safety.

**Evidence.** engine.rs:2207-2214 writes `bytes` (the provider's raw response) via write_original after decode succeeded. App code prefers the original: macOS WallpaperImage.swift:45 and NowView.swift:66, Windows NowPage.xaml.cs:289, Linux wallpapers.rs:88-94 (the Linux line preferring the thumbnail was seen; original used elsewhere at 94 and 268). Fuzzing the Rust side found no panics (see checked_ok), so this is about the other parsers only.

**Recommended fix.** Store a re-encoded, metadata-stripped copy of the image (decode, then encode to PNG/JPEG at original size) as the canonical original, or at least display only the core's own re-encoded thumbnail or render in the UI. If byte-exact originals are needed, keep them but never load them in-process from the UI.

#### core-parse-4: Parsed model fields other than title and prompt are unbounded in length and are stored, embedded and hashed in full
- Kind / severity: hardening / info  
- Platforms:   
- Locations: core/src/composer.rs:1073-1126; core/src/composer.rs:1167-1176; core/src/embed.rs:32-47; core/src/taste.rs:71-84

parse() truncates only the title (MAX_TITLE_CHARS = 60) and caps list lengths (MAX_ITEMS = 12). summary, setting, subject, time_of_day, weather, season, style, composition, each list item and prompt are kept at any length up to the transport limit (4 MB for hosted providers, MAX_TEXT_RESPONSE_BYTES). check() only flags prompt > 2500 chars, as a problem to retry. A candidate with a huge summary or element is still scored, embedded, turned into taste features, stored in the generations table and shown by the apps. The costs are linear in the input except `check`/`repair`, which are linear per Avoid/Must keyword, up to 64. For a 4 MB answer this is well under a second of CPU, so it is not a DoS. It is a storage and UI-size gap and a cheap way for a hostile local text server to bloat the database over time.

**Attack scenario.** A malicious OpenAI-compatible or Ollama server the user selected returns a 4 MB summary and 12 x 4 MB list items per run. History rows grow by megabytes per wallpaper. The UI may stall rendering a multi-megabyte label. The server must be chosen by the user, which SECURITY.md lists as out of scope when it stays within the network rules.

**Evidence.** core/examples/big.rs fed 4M-character fields (a 28 MB synthetic JSON, bigger than any real response) to parse, repair, check, concept_text, HashingEmbedder and taste::features. parse 32 ms, repair 1.54 s, check 3.04 s, concept_text 50 ms (28 MB string), hash-embed 393 ms, features 222 ms; peak RSS 1.4 GB. Scaled to the real 4 MB response cap, about 1/7 of that. No panic or hang.

**Recommended fix.** Apply shorten()/one_line() caps in read_candidate to every field, for example 400 chars for short fields, 1,000 for summary and 2,500 for prompt, and cap per-item length in read_list. This also bounds what later flows into echo prompts and taste labels.

#### core-parse-5: ComfyUI progress watcher keeps one map entry per node id sent over /ws and treats any event as activity
- Kind / severity: hardening / info  
- Platforms:   
- Locations: core/src/providers/comfyui.rs:657-680; core/src/providers/comfyui.rs:353-414

JobWatch::on_message inserts a BTreeMap<String,(u32,u32)> entry for every distinct data.node value in `progress` messages for the job's prompt_id. Messages may be up to 8 MB (MAX_SOCKET_MESSAGE_BYTES), and the map has no bound. Any message also refreshes last_activity, so a server that keeps sending events defeats the stall detector, and only the 30-minute minimum ceiling (MIN_CEILING) ends the wait. A ComfyUI server (possibly remote over https) that is compromised, or has a hostile custom node, can therefore grow the client's memory for up to the ceiling. The server is one the user chose, and ComfyUI already receives the full prompt, so this is mostly a DoS-against-the-client edge. Not exercised dynamically. Reasoned from the code.

**Attack scenario.** A hostile ComfyUI custom node, or a compromised remote ComfyUI, sends a flood of `progress` events with unique node names after the job id is known. The client's node map grows until the ceiling, and the stall timer never fires.

**Evidence.** Code read at comfyui.rs:659-672: self.nodes.insert(node, ...) with node from the message, no size check. The loop in wait_for_image uses `biased` select with the socket first. The deadline check is the only bound.

**Recommended fix.** Ignore progress for node ids that are not keys of the submitted graph (the client has `graph.nodes`), or cap nodes.len() at graph size. Count only genuine progress (an increasing value) as stall-resetting activity.

#### core-parse-6: Embedding model files are loaded without an integrity check and the Linux build can load them from an env var or a compile-time source path
- Kind / severity: hardening / info  
- Platforms:   
- Locations: core/src/embed.rs:113-143; apps/linux/src/desktop.rs:56-71; scripts/fetch-model.sh:1-40

CandleEmbedder::load only checks that config.json, tokenizer.json and model.safetensors exist, then parses them (serde_json for the config, tokenizers::from_file, and VarBuilder::from_buffered_safetensors). The SHA-256 verification lives only in scripts/fetch-model.sh at build time. At run time on Linux, desktop::model_dir() honours AUTOPAPER_MODEL_DIR. It also falls back to a path built from env!("CARGO_MANIFEST_DIR")/../../models, baked into the binary at compile time, when the installed copy lacks model.safetensors. On macOS and Windows the model is inside the signed bundle or MSIX, and in Flatpak it is under read-only /app, so tampering there needs a compromised install. A tampered tokenizer or config could cause large allocations or regex-heavy work, but not code execution (safetensors is a data format; no `unsafe` is used). Anyone able to set the environment or write the model directory can already run code as the user, so this is not a vulnerability. It is listed as defence in depth.

**Attack scenario.** Only through a compromised install or a hostile environment, for example a .desktop launcher or a shell profile that sets AUTOPAPER_MODEL_DIR to a crafted directory. The effect is a crash or large allocation in the embedder. No key theft or code execution path was found.

**Evidence.** Code read. `grep unsafe core/src` finds only a comment in embed.rs. The two tests at embed.rs:384-399 show that wrong files give a Storage error. A tampered-model run was not performed, as it needs write access to the model directory.

**Recommended fix.** Optionally verify SHA-256 of the three files against constants compiled in at load time (the sums are already pinned in fetch-model.sh). Do not fall back to a CARGO_MANIFEST_DIR path in release builds (gate it with cfg(debug_assertions)).

**Checked and found sound:**
- composer::parse (core/src/composer.rs:1016-1126): fed 5,300,000 random structured JSON values plus fenced, prose-wrapped and double-encoded strings (core/examples/fuzz.rs), including Unicode edge cases such as İ, ß, ǅ, ﬃ, U+202E and NUL. No panics. Recursion depth is bounded: string-following stops at depth 2, and serde_json caps nesting at 128. The byte slicing in embedded_json uses ASCII brace positions, so it is on char boundaries.
- composer::repair/check/without_denials/without_field_labels (composer.rs:542-675, 1167-1197) ran in the same 5.3M-iteration sweep with Must/Avoid keywords including İstanbul, ß and 'no people'. No panic. The one byte slice (`&first[c.len_utf8()..]`, line 563) follows `first.chars().next()`, so it is safe.
- Model-text sanitising: clean() (composer.rs:1128-1132) replaces all control characters with spaces, so terminal escapes and newlines cannot reach the stored text or logs. Residual: non-Cc format characters such as bidi overrides pass through. This is cosmetic.
- text.rs (words, stem, mentions, feature_key, normalize_keyword, normalize_mood_name, text.rs:13-190): exercised on the same random Unicode corpus. No panics. Keywords and mood names strip control characters, collapse whitespace and enforce length limits in characters.
- imaging::decode (core/src/imaging.rs:101-127): sniff checks magic bytes and the decoder's guessed format must equal the sniffed one, so mislabelled content is rejected. The byte cap is checked first, so 98 MB, 72 MB and 72 MB noise JPEG/WebP/PNG were rejected in microseconds. Header dimensions are checked before pixels are allocated. Orientation is applied after the limit checks.
- Image mutation fuzzing (core/examples/fuzz.rs, `img` mode): 500,000 mutated PNG/JPEG/WebP inputs, including 300,000 structure-aware PNG mutations with CRC repair (IHDR fields, injected iCCP/tEXt/zTXt/iTXt/eXIf/acTL/fcTL/fdAT/PLTE/tRNS chunks, duplicated, reordered and truncated chunks) and RIFF-size-repaired WebP. Each accepted image went through phash, write_thumbnail and render_cover. 0 panics, slowest single iteration 168 ms, no large allocations beyond the budget in core-parse-1.
- Extreme aspect ratios (core/examples/extreme.rs): images 1x1, 1x2, 16384x1, 1x16384, 16384x2, 3x16384, 641x1, 5x5000 and 16384x3900 rendered to 10 display sizes (1x1 up to 16384x3900, 640x2560, 641x2561), plus thumbnail and phash. 150 renders, 0 panics. cover_crop, scale_round and the thumbnail height clamp (imaging.rs:135-148, 256-275) hold.
- Decompression bombs: zTXt/iTXt/iCCP bombs of 2 GB decompressed from 2 MB files were rejected or capped at 540 MB RSS. An all-zero RGBA16 64 MP PNG compressed to 2.4 MB decodes in 32 ms. A 45 MB eXIf chunk and 1M tEXt chunks used under 250 MB. Animated WebP with 500,000 frames decoded in 48 ms. Decode-time CPU is small, so there is no hang.
- Atomic file writes (imaging.rs:297-323): temp file created with create_new beside the target, synced, then renamed. An existing file or symlink at the temp name is never followed, and rename replaces a symlink at the destination rather than writing through it. Temp file is removed on failure.
- File paths are never built from model or provider text. Image names are `<uuidv7>.<ext>` where ext comes from sniff() (engine.rs:2207-2208). Thumbnails are `<uuid>.jpg` and renders are `<uuid>-<w>x<h>.jpg`. render_for_display (engine.rs:662-683) resolves the id through the database first and validates display sides. Orphan and prune deletion only touches names parsed as UUIDs (engine.rs:2800-2830). The only deletes of DB-stored paths (engine.rs:2731, 2760) use paths the engine wrote itself.
- SQLite store (core/src/store.rs): all dynamic values use bound parameters. The format! uses at lines 376-389 and 2153 interpolate only constant SQL fragments (ORPHANS) or test column names. No user or model text is concatenated into SQL. Embedding blobs are length-checked (store.rs:1775-1790). Settings are rebuilt field by field with defaults for bad values (store.rs:1568-1590). Schema version newer than the binary is refused. Activity bounds are validated before indexing (store.rs:916-922). NaN-safe sorting uses total_cmp throughout the novelty, engine and taste code.
- novelty.rs, echo.rs, taste.rs, embed.rs (HashingEmbedder/cosine/l2_normalize): no indexing on untrusted lengths. Mismatched embedding lengths yield 0 (embed.rs:76-84, novelty.rs:169), NaN is handled, and zero or non-finite vectors are refused. No `unsafe` anywhere in core/src (grep).
- ComfyUI template filling (core/src/providers/comfyui.rs:1018-1040): width, height and seed are substituted first as plain numbers and the prompt last, JSON-escaped via serde_json. A scratch property test (attack module appended to a scratch copy of comfyui.rs) ran 80,000 hostile prompts (quotes, backslashes, braces, injected `","class_type":...` fragments, U+2028, NUL, ", `{{prompt}}`, `{{seed}}`) against 4 templates, including a quoted, an embedded, a doubled and a wrapped-/prompt-body template. The node set, every node's class_type and every node's input keys were identical to the benign fill, and the prompt text was preserved intact in the text input. 200,000 mutated templates through fill_placeholders/api_graph: no panics. A prompt cannot add nodes, change classes or break out of its string.
- ComfyUI server answers: prompt_id is restricted to [A-Za-z0-9_-] before use in URLs (comfyui.rs:307-326). Path segments and the /view query are percent-encoded via url::Url (comfyui.rs:237-243, 446-463). Server-supplied filename and subfolder only go into a query to the same server, never to the filesystem. Result images go through imaging::decode. JSON responses are capped at 16 MB, images at 50 MB, ws messages at 8 MB. Progress numbers are clamped to u32 and finite (comfyui.rs:661-668).
- OpenAI-compatible image-by-URL fallback (providers/openai_compat.rs:167-185) fetches only from the configured server's own origin, re-checked through check_url, so a model response cannot redirect the key or trigger SSRF to a third host.
- Console redaction: provider headers are never recorded. Bodies are walked as JSON, credential-named fields and base64 image fields are masked, URL userinfo/query/fragment are stripped, and the literal key values from the request headers are replaced (engine.rs:1286-1380, net.rs:468-500). Empty secrets are filtered out, so replace("") cannot happen. Output is capped at 65,536 bytes on a char boundary (engine.rs:1321-1330).
- Prompt injection from keywords, moods and model output: keywords/moods are control-stripped and length-limited, and every list item embedded in prompts goes through one_line()/quoted() with a 400-char cap (composer.rs:896-931). The ComfyUI prompt is JSON-escaped as above. File names are UUID-based. The only effect of injected text is on what the model writes or paints, plus stored titles and labels shown in the UI. No path, shell, SQL or URL sink is reachable from model or keyword text in core.
- Dependency advisories: cargo audit -n -f Cargo.lock against the local RustSec database (HEAD dated 2026-08-06) found no vulnerabilities in the 474 locked crates (image 0.25.10, png 0.18.1, image-webp 0.2.4, zune-jpeg 0.5.15, rusqlite 0.40.2/libsqlite3-sys 0.38.2, tokenizers 0.22.2, safetensors 0.8.0, serde_json 1.0.151). The single warning is RUSTSEC-2024-0436, `paste` unmaintained (transitive via tokenizers/gemm), which is not exploitable. The DB was used offline and was not refreshed from the network.

### Surface: secrets

#### secrets-1: Error and log redaction is heuristic only; an echoed key can reach system logs, and JSON field names are never scrubbed
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/net.rs:469-505; core/src/net.rs:458-466; core/src/engine.rs:1345-1380; core/src/engine.rs:2627; apps/macos/AutoPaper/App/AppModel.swift:669; apps/windows/AutoPaper/Services/Log.cs:23-37; apps/linux/src/app.rs:474

Exact-value scrubbing of the request's credential headers exists only inside the Console path (ConsoleHttp / console_body_with_secrets). Everything else that reaches logs relies on net::redact(), a pattern heuristic. It only catches sk-* (20+ chars), AIza* (30+ chars), and 32+ char alphanumeric tokens with no '.'. error_message() and error_for_status() build AutoPaperError::InvalidResponse detail strings with only that heuristic. Those errors are logged verbatim: macOS `log.error(String(describing: error), privacy: .public)`, Windows Log.Error writes error.ToString() to LocalState\logs\autopaper.log, and Linux `tracing::warn!(%error)` goes to stderr. A key that does not match the patterns (a short OpenAI-compatible key, or a dotted JWT-style key) therefore survives when the server's non-401 JSON error message quotes it. PRIVACY.md says the diagnostics contain 'No keys', and error.rs says detail 'never contain keys', but that rests on the heuristic. Separately, console_body_with_secrets::sanitize rewrites JSON values but not object keys, so a response whose field name is the key is stored in the runs table and in Copy Details / Export JSON unredacted.

**Attack scenario.** The user points the OpenAI-compatible provider at an LLM gateway or proxy that returns errors such as {"error":{"message":"Invalid request for key <the Bearer value>"}} with a 400, 404 or 422 status (a 401 maps to InvalidKey with no text). The key goes into the public macOS unified log, the persistent Windows log file, or the Linux journal. It can then be read by other local admin or diagnostic tooling, or pasted into a support bundle. A server that echoes the key as a JSON field name gets the key into an exported Console report that the user shares. The server already holds the key, so this is a leak to local logs and shared reports, not to a new remote party. It needs a server that echoes the key.

**Evidence.** Scratch PoC (copy of core, test added to engine.rs; repo untouched). With secrets=["Bearer lm-studio-key-1234","lm-studio-key-1234"], console_body_with_secrets on {"lm-studio-key-1234":1,...} printed the key unchanged as the field name '"lm-studio-key-1234": 1'. The same key as a value was replaced with [redacted]. net::redact("bad token lm-studio-key-1234") returned the text unchanged. net::redact on a dotted JWT-shaped key returned it unchanged. net::error_message({"error":{"message":"Invalid key eyJhbGci..."}}) returned Some("Invalid key eyJhbGci...") unredacted. Test name: engine::secrets_poc::hostile_console_bodies.

**Recommended fix.** Make exact-value redaction the primary mechanism everywhere. Have the provider layer (or GuardedSecrets / a shared redactor) keep the key values it has handed out and pass them to error_message / error_for_status and to the tracing and host-visible detail strings. Also scrub JSON object keys in console_body_with_secrets::sanitize. Optionally add a 'Bearer <anything>' pattern to net::redact and drop the no-'.' exemption for long tokens.

#### secrets-2: Local database with prompts, history and Console traces is created with the default umask (no 0600/0700)
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/engine.rs:897; core/src/engine.rs:949; apps/linux/src/desktop.rs:51-53; cli/src/main.rs:159-162

The data directory and autopaper.sqlite3 (with its WAL and journal files) hold every full prompt, the Console request and response bodies, keywords and history. They are created with create_dir_all and SQLite defaults. A grep of core, cli and apps/linux found no set_permissions, PermissionsExt or mode(0o...) call. On Linux the directory is $XDG_DATA_HOME/autopaper, which is not isolated outside Flatpak. The CLI's default .scratch/cli-data sits inside the repository checkout. Keys are not in this database (I verified they only live in the platform stores and in process memory), so this concerns prompts and privacy rather than credentials.

**Attack scenario.** On a multi-user Linux host where home directories are world-readable (traditional 0755), another local user reads autopaper.sqlite3 and recovers the owner's keywords, prompts and Console traces. On macOS and Windows the app container or profile ACLs already cover this. In Flatpak the data directory is private.

**Evidence.** git grep for set_permissions, PermissionsExt and mode(0o in core/src, cli/src and apps/linux/src found no use on the data directory or database. This is a code-reading result with no dynamic proof.

**Recommended fix.** On Unix, create the data directory with mode 0700 and open the database with a 0600 umask (or chmod the db, -wal and -shm files) in Engine::open.

#### secrets-3: oo7 silently falls back to an unencrypted Secret Service session; keys are never zeroised in memory
- Kind / severity: hardening / info  
- Platforms:   
- Locations: apps/linux/src/secrets.rs:108; core/src/providers/openai.rs:383-394; core/src/providers/google.rs:580-590; apps/macos/AutoPaper/Engine/KeychainSecretStore.swift:19-41; apps/windows/AutoPaper/Services/CredentialStore.cs:44

Two defence-in-depth gaps, neither with a remote path. (1) On non-Flatpak Linux, oo7::Keyring::new() uses dbus::Service::new(). If the encrypted DH session negotiation fails with a D-Bus method error, it silently falls back to Algorithm::Plain (oo7-0.6.0 src/dbus/service.rs:51-62), so the secret crosses the session bus in the clear. The session bus is per-user, so this only matters to a process that already runs as the user. (2) Keys are handled as plain String / Data / managed string. The macOS memo keeps every key for the process lifetime, the Rust core copies keys into header Strings (HttpRequest is Clone), and Windows PasswordBox.Password / Encoding.Unicode.GetString yield managed strings. Only the Windows write path zeroes its byte[] blob (CryptographicOperations.ZeroMemory). Memory dumps, swap or crash dumps of the process could therefore contain keys.

**Attack scenario.** A same-user process that can read process memory or the session bus can already read the keychain items through the same OS APIs, so there is no meaningful added exposure. A crash dump shared by the user could contain a key.

**Evidence.** oo7-0.6.0 src/dbus/service.rs lines 51-62 (`Self::encrypted().await` ... `=> Self::plain().await`). No Zeroizing or zeroize crate use in core, apps/linux/src/secrets.rs or KeychainSecretStore.swift.

**Recommended fix.** Optional. Call dbus::Service::encrypted() directly via oo7 and refuse to store keys over a plain session. Use zeroize::Zeroizing for key Strings in the providers and Linux layer, and avoid the macOS process-lifetime memo (or keep it as Data and wipe it).

**Checked and found sound:**
- macOS KeychainSecretStore.swift:1-120: generic passwords under service com.autopaper.credentials, no kSecAttrSynchronizable (not iCloud-synced), no access-group sharing, no plaintext fallback, update-in-place so a failed save keeps the old key, log lines carry only the account name and OSStatus (never the value). The app is sandboxed with the hardened runtime, and the entitlements add no keychain-sharing, DYLD or get-task-allow exceptions (AutoPaper.entitlements, project.yml:94-98).
- Windows CredentialStore.cs:28-104: Credential Manager generic credentials, CRED_PERSIST_LOCAL_MACHINE (not roamed), PasswordVault deliberately avoided, blob size-checked, the write buffer zeroised, no plaintext file fallback. SignIn.cs is only the startup task and handles no secrets. Windows Log.cs logs exception text only (no key material in any call site I grepped), is bounded to 256 KB, and goes to a per-user LocalState folder.
- Linux secrets.rs: Secret Service via oo7. In Flatpak the portal-keyed file keyring is used (oo7 writes it 0600 inside a 0700 dir, oo7-0.6.0 src/file/api/mod.rs:137,155). There is no plaintext file fallback anywhere, and a failure means no key rather than an unprotected store. Errors and tracing carry only the account name and oo7's error text. The Flatpak manifest grants no filesystem access and no extra D-Bus names beyond the tray watcher.
- Linux, Windows and macOS key fields: adw::PasswordEntryRow, Windows PasswordBox and SwiftUI SecureField, so values are masked. The accessibility names are labels only (AutomationProperties.Name, accessibilityLabel) and do not expose the value. No key-reveal toggles were found.
- Core key routing: hosted providers only talk to a fixed HostPolicy::Hosted host over https with no port (net.rs:20-55). OpenAI-compatible keys are bound to the server origin (model.rs:365-374, openai_compat.rs:65-78), and image URLs from the server are fetched only from the same origin (openai_compat.rs:164-184). Redirects are manual and capped at 3, same host only, with no https-to-http downgrade, and credential headers are dropped on a scheme or port change (net.rs:196-206, 329-342). There is no cookie store and no referer. Credential headers are marked sensitive in reqwest (net.rs:317), and the Google key is only ever in the x-goog-api-key header, never the URL.
- HttpRequest Debug (ports.rs:100-120) redacts credential headers and omits the body, and a unit test covers it. transport_error uses without_url() and redacts before logging (net.rs:373). The Rust core has no Debug or Clone on any type that holds a key apart from HttpRequest, and ProviderSelection and Settings carry no secrets. GuardedSecrets (engine.rs:1046-1072) catches panics from a host store without logging values.
- Console and report: the Console records no request headers. It records the address as origin plus path only, so query strings, userinfo and fragments are dropped (engine.rs:1286-1310). Credential header values, with and without a 'Bearer ' prefix, are remembered in memory per run (never persisted) and substituted in text and JSON values. Fields named authorization, apikey, xgoogapikey, token, password, cookie and similar are blanked, base64 and data URIs are omitted, and size limits apply. run_report only serialises that stored sanitised RunRecord (engine.rs:333-336), and core/tests/engine.rs:2394 asserts the report has no key or 'Authorization'. Prompts and responses in Console and the export are by design and are disclosed in PRIVACY.md:72.
- Clipboard: Copy Details copies only the sanitised report (macOS ConsoleView.swift:206 and 614, Windows ConsolePage.xaml.cs:427, Linux console.rs:328). No key is ever placed on the clipboard, and no password-field copy path was found.
- Developer CLI cli/src/main.rs:223-293: .env is gitignored (.gitignore:2-4) and .env.example is blank. Keys are read into an in-memory map only and never written back. Missing-key errors name the variable, not the value. Terminal output is stripped of control sequences (printable/say!). The default env path is the repo's own .env, and --env-file is user-chosen.
- Secret scan of tracked files and history: git grep for sk-, AIza, PEM private keys and ghp_ tokens found only obviously synthetic test fixtures. No .env, .pem, .p12 or gnupg file was ever added in history (all 20 commits checked).
- Redaction tests with hostile inputs (scratch copy of core, cargo test --offline): a key echoed as a JSON value, inside a URL-style 'key%3D...' string, or in a header-named field is replaced with [redacted] in the Console path. The known gaps are in secrets-1.

### Surface: macos

#### macos-1: Console renders model-written text as Markdown, so a model can plant clickable links of any URL scheme
- Kind / severity: vulnerability / low  
- Platforms:   
- Locations: apps/macos/AutoPaper/Views/ConsoleView.swift:535-555; apps/macos/AutoPaper/Views/ConsoleView.swift:557-590; apps/macos/AutoPaper/Support/ConsoleJSON.swift:100-125; core/src/composer.rs:719-745

The Console's 'User prompt' disclosure parses the recorded prompt with MarkdownBlock.parse. Each line goes through AttributedString(markdown:, interpretedSyntax: .inlineOnlyPreservingWhitespace) and is shown in a SwiftUI Text (ConsoleView.swift:587-589). That parse produces .link attributes, and SwiftUI Text opens them on click. The user prompt is not only the user's keywords. core/src/composer.rs:719-745 puts the previous wallpapers' model-written 'summary' fields into it as 'Recent wallpapers…' and 'too close' lists. The composer only shortens summaries and checks them for Avoid keywords, so Markdown is not stripped. A summary written by a malicious or prompt-injected provider, or by a malicious Ollama or OpenAI-compatible server, therefore comes back in the next run's prompt and renders as a clickable link inside AutoPaper's own Console. Other views (History, Now, notifications) show these strings verbatim, so only this view is affected.

**Attack scenario.** A malicious or compromised writing endpoint (for example an OpenAI-compatible server the user configured, or injected content in a provider answer) returns summary = 'A quiet harbour. [Verify your OpenAI key](https://evil.example/login)'. The summary is stored. On the next generation the 'instructions' Console event contains it in the 'Recent wallpapers' list. If the user opens Console, expands 'User prompt' and clicks the link, the default handler opens the URL. Phishing is the main risk. file:// and custom-scheme links also launch via LaunchServices, and I confirmed those schemes are accepted. It needs the user to open Console and click, and the sandbox and Gatekeeper still apply, so the impact is limited.

**Evidence.** Scratch test (compiled Swift, same options as ConsoleView.swift:588). Input '1. A quiet harbour. [Verify your OpenAI key](https://evil.example/login)' gives visible text '1. A quiet harbour. Verify your OpenAI key' with link https://evil.example/login. Input '[x](file:///Applications/Calculator.app) [y](brewbrowser://bundle/evil) <https://evil.example>' yields all three as links. I did not click-test the SwiftUI rendering; the link attribute is what SwiftUI Text acts on.

**Recommended fix.** Do not interpret Markdown for model-derived text. Either render the user prompt with Text(verbatim:), or after parsing remove every .link run (set attributes[.link] = nil). If you keep formatting, only allow the instruction lines the app itself writes. Alternatively, have the core escape or strip '[', ']', '(', ')', '<', '>' in summaries when it quotes them back into the prompt.

**Verifier: downgraded (severity info).** The code path is real. composer.rs:896-905 only collapses whitespace and shortens, so Markdown in the stored summaries stays intact. ConsoleView.swift:557-590 parses the lines with inline Markdown and shows them in a SwiftUI Text, with no OpenURLAction override. A link therefore appears and is clickable, and the reviewer's scratch test showed the parser produces link attributes. The impact is very narrow. The link text is shown in the Console's 'User prompt' disclosure, which is collapsed by default and is a developer-style view of the prompt. The user has to open Console, expand that disclosure and click a link. The attacker also needs control of the user's own configured writing endpoint or an injected provider answer. Anyone who controls that endpoint already controls the wallpapers and the displayed summaries, and could put a plain-text lure in History or Now, which also show the summary. The only extra step here is that the lure is clickable in one hidden view. The macOS sandbox and Gatekeeper still apply to anything a link opens. I did not click-test SwiftUI. This is a hardening nit (strip or escape Markdown from untrusted text, or render it as plain text), not a vulnerability.

#### macos-2: Untrusted provider image bytes are stored as-is and then re-parsed by Apple ImageIO inside the app process
- Kind / severity: hardening / low  
- Platforms:   
- Locations: apps/macos/AutoPaper/Views/WallpaperImage.swift:44-76; apps/macos/AutoPaper/Views/HistoryView.swift:202-206; apps/macos/AutoPaper/Views/HistoryView.swift:733-737; core/src/imaging.rs:101-134; core/src/imaging.rs:137-141

The core validates provider and server images with the Rust `image` crate (magic bytes, header dimensions, limits). It then writes the original bytes unchanged with write_original (imaging.rs:137-141). Wallpapers shown on screen use re-encoded renders (DesktopService DecodedImage), which is safe. The Now, History and Moods views use WallpaperImage, which tries the original first (WallpaperImage.swift:45-47) and decodes it with CGImageSourceCreateThumbnailAtIndex in the app process. Quick Look also previews the original (HistoryView.swift:204,735). So bytes from a malicious ComfyUI, OpenAI-compatible server or compromised provider are parsed a second time by a different, much larger decoder stack (ImageIO PNG, JPEG, WebP, ICC and metadata parsers) that the Rust checks do not model. I found no concrete bug. The weakness is that the sandboxed app depends on ImageIO being memory-safe for attacker-chosen files, when the app already holds a clean re-encoded 640 px thumbnail and a re-encoded render.

**Attack scenario.** A hostile local or LAN image server returns a crafted but Rust-valid PNG, JPEG or WebP that triggers an ImageIO parsing flaw (past ImageIO CVEs exist), or an oversized ICC or metadata payload that the Rust decoder skips. The user opens History or the Now view and the app process decodes it. The App Sandbox limits the blast radius, but the key-handling process is the one that parses the file.

**Evidence.** imaging.rs:137 'Writes bytes as-is (already validated by decode)'. WallpaperImage.swift:45 puts the original first. NowView.swift:66 uses thumb then original. I did not run a fuzzing or exploit test.

**Recommended fix.** Have the core re-encode the original through the decoded DynamicImage (strip metadata) before saving it. Alternatively, make the app prefer thumbPath or the re-encoded render for all in-app display (preferThumbnail: true) and only hand the original to Quick Look or export when the user asks.

#### macos-3: Sparkle feed is unsigned and verify-before-extraction is not enabled
- Kind / severity: hardening / low  
- Platforms:   
- Locations: apps/macos/project.yml:65-77; apps/macos/AutoPaper/Info.plist; scripts/appcast.py:37-49; scripts/release.sh:186-191

Updates are protected by an EdDSA signature on the zip (SUPublicEDKey, project.yml:71). The appcast is not signed. Info.plist sets neither SURequireSignedFeed nor SUVerifyUpdateBeforeExtraction. Sparkle 2.10.0 in the built bundle contains both keys (checked with `strings` on the framework). appcast.py writes the feed by hand, and sign_update is run only on the zip. An attacker who controls the feed (GitHub Pages account or repo takeover, or a TLS break) cannot ship a malicious build without the private key. They can still control everything else in the item: the enclosure URL, sparkle:version, minimumSystemVersion, release-notes HTML and link, and the version and length metadata. This allows pointing at older validly signed builds, or forcing update prompts. Whether Sparkle's expected-version check blocks the replay of an older signed build was not tested.

**Attack scenario.** Pages or repo takeover, followed by a feed pointing at an older signed AutoPaper zip with an inflated sparkle:version, or with attacker HTML in the notes. The EdDSA check passes because the zip was really signed. This only matters if an old build has a known flaw, or if bundle-version matching is incomplete.

**Evidence.** strings of Sparkle.framework/Versions/B/Sparkle lists SURequireSignedFeed and SUVerifyUpdateBeforeExtraction. The Info.plist of the exported app has neither. Distribution itself checked out (see checked_ok). I did not attempt a replay.

**Recommended fix.** Sign the feed as part of release.sh (Sparkle's sign_update supports signing the appcast; check the Sparkle 2.10 docs for the exact flag) and add SURequireSignedFeed: true. Add SUVerifyUpdateBeforeExtraction: true. Test an update from the previous release to the new one before shipping, because turning these on without a signed feed breaks updates.

#### macos-4: release.sh passes the Apple app-specific password on the command line
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/release.sh:31-33; scripts/release.sh:62-63; scripts/release.sh:68

notarize() runs `xcrun notarytool submit … --password "$APPLE_PASSWORD"` and later `notarytool log … --password …`. On macOS any local user or process can read other processes' argv (`ps -ww`), so the password is visible for as long as notarization waits, often minutes. The script comment also suggests sourcing a credentials .env file into the shell environment.

**Attack scenario.** Another unprivileged account or a process running as the user polls `ps` while the maintainer runs a release and captures APPLE_ID and the app-specific password. That gives access to the Apple ID's notarization service and app-specific-password scope, which is a supply-chain foothold. The window is short and needs a local observer.

**Evidence.** Code read only; not run. release.sh lines 62-63 and 68 contain the --password arguments.

**Recommended fix.** Use `xcrun notarytool store-credentials AutoPaper-notary` once, then `--keychain-profile AutoPaper-notary` and drop APPLE_PASSWORD from the script. This keeps the secret out of argv and out of the environment.

#### macos-5: Sparkle, the update-trust root, is pinned only by tag in a git-ignored project
- Kind / severity: hardening / low  
- Platforms:   
- Locations: apps/macos/project.yml:24-26; apps/macos/.gitignore (via /.gitignore:23 '*.xcodeproj/'); scripts/release.sh:98-105

project.yml pins Sparkle with `exactVersion: 2.10.0`. The SwiftPM revision pin (Package.resolved, revision eef1a539…) lives in the generated AutoPaper.xcodeproj. That folder is git-ignored (`git check-ignore` confirms), and nothing in the repo records the revision or the checksum. A maintainer's fresh clone, or a CI or contributor build, therefore trusts whatever commit the 2.10.0 tag points at on GitHub. Sparkle ships the code that verifies updates and runs the installer, and release.sh also runs Sparkle's sign_update binary from the same package artifact with access to the signing key. A moved or forged tag would reach both the shipped app and the signing tool.

**Attack scenario.** The upstream Sparkle repo or tag is tampered with, or the maintainer's Package.resolved is lost (clean checkout, new Mac). The next release build resolves a different commit, and that build signs and ships with it. Low likelihood, but high impact.

**Evidence.** git ls-files shows no Package.resolved or Package.swift. Local Package.resolved at apps/macos/AutoPaper.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved pins revision eef1a539a373c1f1a320624b1130fc5de7b2e100 but is ignored.

**Recommended fix.** In project.yml use `revision: eef1a539a373c1f1a320624b1130fc5de7b2e100` (XcodeGen supports revision), or commit a Package.resolved and have release.sh refuse to build if the resolved revision differs. Optionally verify the Sparkle framework and tools' code signature or hash in release.sh.

#### macos-6: Sparkle private signing key is usable without a prompt by any same-user process via the 'Always Allow' ACL
- Kind / severity: hardening / info  
- Platforms:   
- Locations: scripts/release.sh:186-190; scripts/release.sh:101-105

The release script tells the maintainer to choose 'Always Allow' the first time sign_update reads the EdDSA key from the login Keychain. After that, the Keychain ACL trusts the sign_update binary at build/SourcePackages/artifacts/sparkle/Sparkle/bin. That path is in the user-writable repo and is re-downloaded by SwiftPM. Any process running as the user can run that binary, or replace it, and obtain EdDSA signatures over arbitrary zips with no prompt. Combined with a feed or GitHub release compromise, that is arbitrary code on every user's Mac. SECURITY.md puts an already-compromised account out of scope, so this is a note on key custody, not a defect in the shipped app.

**Attack scenario.** Malware running as the maintainer signs a trojaned AutoPaper zip and publishes it, or tampers with the release. This needs an already-compromised user session.

**Evidence.** Code and comment read. I did not read or touch the Keychain.

**Recommended fix.** Keep the key requiring a prompt (do not choose Always Allow), or move it to a hardware token or a dedicated release keychain with a password. Build the signing tool from a pinned, verified source rather than from the writable build/ folder.

**Checked and found sound:**
- Release entitlements (codesign -d --entitlements on build/release/export/AutoPaper.app): only app-sandbox, network.client, files.user-selected.read-write (needed: workflow open and Console report export, ConsoleView.swift:106 and ProvidersSettings.swift:231) and the two Sparkle mach-lookup exceptions (com.autopaper-spks/-spki, as Sparkle's sandbox guide requires). No get-task-allow, JIT, unsigned-memory, disable-library-validation or DYLD entitlements on the release. Nothing is broader than needed.
- Hardened runtime and notarization: flags=runtime, Developer ID, stapled ticket, `spctl -a -vv` says 'accepted, Notarized Developer ID', `codesign --verify --deep --strict` passes, and Sparkle's Updater.app and Installer.xpc validate. Every dependency is an Apple system framework or the bundled Sparkle (otool -L). The only LC_RPATH is @executable_path/../Frameworks, and release.sh:143-145 rejects unexpected linked libraries.
- Debug build entitlements contain get-task-allow, but only the Developer ID Release goes to users, and release.sh:150 refuses get-task-allow. macos-install.sh Debug is an explicit developer option.
- Info.plist: SUFeedURL is https, with the right release feed (project.yml:68, release.sh:157). SUPublicEDKey is set (no DSA key, so no legacy path). SUEnableInstallerLauncherService is set. SUEnableJavaScript, SUEnableSystemProfiling and SUAllowsAutomaticUpdates are not set, so release-notes JavaScript is off and no system profile is sent. The app asks before automatic checks (Updates.swift). Updates.swift adds no feed or key overrides.
- Fixed URLs only: every NSWorkspace.open, Link and openURL call uses a hardcoded https URL, the x-apple.systempreferences links (AutoPaperApp.swift:115,117) or brewbrowser://bundle/<local-llm|image-gen> built from an enum (ProvidersSettings.swift:580). No model, server or file text reaches a URL, a Process or NSAppleScript call (grep found none). The brewbrowser:// scheme can be claimed by any app, but AutoPaper sends no data over it and only the maintainer's two fixed bundle ids are used.
- DesktopService.swift: it never writes outside the sandbox container. It only calls NSWorkspace.setDesktopImageURL with the core's own render files (renders/<id>-<w>x<h>.jpg re-encoded by the core). Restore uses a path the app recorded from desktopImageURL itself. isRender() (Decisions.swift:114-119) resolves symlinks and standardises the path and uses a trailing-slash prefix test, so 'renders-evil' cannot match. mayReplace() never overwrites a picture the user chose. No shell-outs, no temp files, no AppleScript.
- AppleSystemModel.swift: schemaJson comes from the core and is decoded to a GenerationSchema. The model's JSON is returned as a string and parsed by the core composer (the same path as any provider). The semaphore plus Task.detached is called from a core blocking thread, never the main thread. The ResultBox is one write then one read. Errors map to typed problems and the detail is not echoed to the UI. There is no code path from model output to files or URLs.
- CoreBridge.swift: the engine is called off the main thread. Settings writes go through a serial queue in order. Progress callbacks only carry values to a @Sendable handler that hops to the main actor. The data dir is the sandbox container (drwx------ on ~/Library/Containers/com.autopaper).
- KeychainSecretStore.swift: generic passwords in the login Keychain, never in UserDefaults or on disk. The memo is a Mutex. Failed reads are not cached as 'absent'. Updates are in place. The service name is fixed. Account names come from the core, which scopes OpenAI-compatible keys to their origin (a changed address gets no key). I never read a key.
- Notifier.swift: content is only the app's own text and generation ids. Action handling only calls rate(id:) or opens Settings on a fixed enum (SettingsPaneID(rawValue:)). Notifications are off by default.
- Scheduler.swift: it listens to the public com.apple.screenIsUnlocked distributed notification, which any local process can post. The only effect is a call to runIfDue, which the engine gates on is_due and failure backoff (engine.rs:528-552), so a spoofed event cannot force a wallpaper, a provider call or spending.
- ComfyUI workflow import (ProvidersSettings.swift:505-528, ComfyUI.swift:96-134): the file picker uses a sandbox-granted security-scoped URL, only .json, read once as UTF-8 and checked for {{prompt}} and API-format shape. No path from the file is used on disk. The workflow is sent to ComfyUI by the core, which is the user's own server. Residual: there is no size cap on the string kept in UserDefaults, but the user picks the file themself, so this is self-inflicted.
- Console JSON highlighting (ConsoleJSON.swift) works on event bodies the Console already cuts at 65,536 bytes. The regex has no nested quantifiers, so there is no ReDoS. Other views render model text (title, summary, prompt) with Text(String) or Text(verbatim:), which does not interpret Markdown.
- Intents.swift: the intents act on the app's own model (new wallpaper, like, dislike, switch mood). They take no file or URL parameters, and the mood is chosen from an app-provided entity list. New Wallpaper still goes through the monthly budget check.
- scripts/fetch-model.sh: pinned Hugging Face revision, https-only with --proto =https and --tlsv1.2, SHA-256 per file. scripts/macos-bundle-model.sh and release.sh:163-166 check the bundled model byte for byte. macos-install.sh and release.sh use `set -euo pipefail`, quote all paths, build into build/, and install by `ditto` into /Applications.
- release.sh verification gate: refuses a mismatch between the Keychain's Sparkle public key and SUPublicEDKey, an http feed, a missing hardened runtime or Developer ID on every Mach-O, get-task-allow, and missing mach-lookup exceptions. appcast.py escapes version fields with html.escape and quoteattr; only the maintainer's own notes go into CDATA.

### Surface: windows

#### windows-1: Signing VM has the whole Mac home directory mounted next to a live Azure signing session
- Kind / severity: hardening / low  
- Platforms: windows  
- Locations: scripts/windows-release.ps1:5-20; scripts/windows-release.ps1:200-217; scripts/build-core-windows.ps1:107

The release is built and signed inside the "Windows 11" Parallels VM. windows-release.ps1 builds the Rust core (cargo runs third-party build.rs and proc-macro code) and the app (NuGet packages with MSBuild targets) in the same VM session where `az login` or AZURE_CLIENT_* hold the 'Artifact Signing Certificate Profile Signer' role. The VM is configured with 'Host defined sharing: User home directory', so \\Mac\Home is mounted in the guest. I listed it by name only, without reading any file: it exposes .gnupg, .config, .claude, Library, .cargo, Documents-style folders and more. Any compromised build-time dependency could therefore sign arbitrary code as the real publisher (the package identity is pinned only by the publisher DN) and read or modify the host's home directory, not just the repo checkout. The script already copies the repo with robocopy, so the share is not needed during the build itself.

**Attack scenario.** A malicious or hijacked crate, build script or NuGet package that is pulled in during `windows-release.ps1` runs inside the VM. It reads the Azure CLI token cache or the service-principal environment, calls the Artifact Signing endpoint to sign its own MSIX with the genuine publisher subject, and uploads it to the feed path that windows users auto-update from, or it reads host secrets through \\Mac\Home.

**Evidence.** `prlctl list -i "Windows 11"` shows 'Host Shared Folders: (+)' and 'Host defined sharing: User home directory'. `prlctl exec "Windows 11" cmd /c dir /a /b \\Mac\Home` listed the host's home entries (.gnupg, .config, .claude, Library...). The script header documents `az login` and the robocopy-from-share flow; the signing step (Invoke-Sign, lines ~200-217) runs after the cargo and dotnet builds in the same session.

**Recommended fix.** Build in the VM with the home-directory share disabled (share only a dedicated release-exchange folder). Do the `az login` or service-principal sign step as a separate, short final step after the build has finished, from a clean session that only has the unsigned packages, and sign with a short-lived credential. Verify signing inputs against the build output hashes before signing.

#### windows-2: Release build does not enforce lockfiles (cargo --locked, NuGet lock/hash pinning)
- Kind / severity: hardening / low  
- Platforms: windows  
- Locations: scripts/build-core-windows.ps1:107; apps/windows/AutoPaper/AutoPaper.csproj:82-90

The core is built with `cargo rustc -p autopaper-core --lib --release --target ... --crate-type cdylib` without `--locked`/`--frozen`, so if Cargo.toml and Cargo.lock ever disagree cargo silently re-resolves instead of failing. The C# project pins direct PackageReferences to exact versions, but there is no packages.lock.json, no RestorePackagesWithLockFile/RestoreLockedMode, and no Directory.Packages.props. Transitive NuGet dependencies and the content hashes of restored packages are not pinned. By contrast the signing tools in windows-release.ps1 are SHA-256 pinned, which shows the intent. The shipped package is self-contained (Windows App SDK 2.5.1, .NET 10.0.12, onnxruntime.dll, DirectML.dll), so what restores is what ships.

**Attack scenario.** A compromised or yanked-and-replaced transitive NuGet package (or an out-of-sync Cargo.lock) changes what is compiled into the signed bundle without any diff in the repository.

**Evidence.** build-core-windows.ps1:107 shows the cargo invocation; `find apps/windows -name packages.lock.json` and a grep for RestoreLockedMode/RestorePackagesWithLockFile returned nothing.

**Recommended fix.** Add `--locked` to the cargo command. Enable `RestorePackagesWithLockFile` and `RestoreLockedMode` (CI/release) and commit packages.lock.json, or move to central package management with transitive pinning.

#### windows-3: Full-trust package gives no isolation: Credential Manager keys and the data folder are open to any same-user process
- Kind / severity: hardening / info  
- Platforms: windows  
- Locations: apps/windows/AutoPaper/Package.appxmanifest:70-77; apps/windows/AutoPaper/Services/CredentialStore.cs:73-85; PRIVACY.md:103-106

The app is a runFullTrust desktop-bridge app, so it runs with the user's normal token and is not in an AppContainer. API keys are written as generic credentials 'AutoPaper:<account>' with CRED_PERSIST_LOCAL_MACHINE. Credential Manager has no per-application ACL, so any process running as the same user can CredRead them. The declared capabilities (picturesLibrary, unvirtualizedResources) are labels for App Installer's prompt and do not restrict the process, so the PRIVACY.md statement that picturesLibrary is used 'for nothing else' is a code-discipline claim, not an enforced boundary. This matches the SECURITY.md out-of-scope rule for an already compromised account, so it is not a vulnerability, only context for users. The VM check also confirmed the real data folder ACL is tight (user, SYSTEM, Administrators and the package SID only).

**Attack scenario.** Malware that already runs as the user reads the Credential Manager entries and the sqlite history. This needs a compromised account, which is out of scope.

**Evidence.** icacls on C:\Users\michael\AppData\Local\Packages\AutoPaper_nef1hbbyd9msy\LocalState in the Windows 11 VM (a 0.1.0 dev install) showed only the package SID, the user, SYSTEM and Administrators with full control; the capabilities are in Package.appxmanifest:70-77.

**Recommended fix.** Optional: say in PRIVACY.md that the capabilities are not enforced limits for a full-trust app. For stronger key protection on a hostile-process threat model, consider encrypting the key with DPAPI plus app-specific entropy (a modest gain only).

#### windows-4: autopaper_core.dll is built without Control Flow Guard and embeds a build-machine path
- Kind / severity: hardening / info  
- Platforms: windows  
- Locations: scripts/build-core-windows.ps1:107

The PE header of the shipped x64 autopaper_core.dll has DYNAMICBASE, HIGHENTROPYVA and NXCOMPAT but not GUARD_CF (DllCharacteristics 0x160). AutoPaper.exe, onnxruntime.dll and the .NET runtime DLLs have CFG. The DLL is the component that parses untrusted provider JSON and images, so it is the most exposed native code. Panic and location strings also contain the builder's cargo registry path (C:\Users\michael\.cargo\...), which is a minor information leak.

**Attack scenario.** A memory-corruption bug in a native image or JSON dependency would not meet a CFG check. Exploitation also needs such a bug to exist, so this is defence in depth only.

**Evidence.** A PE header dump of build/windows/0.1.3/AutoPaper_0.1.3_x64_arm64.msixbundle -> x64.msix showed DllCharacteristics 0x160 for autopaper_core.dll against 0xc160 or 0x4160 for the others, and `strings` found C:\Users\michael\.cargo\registry paths.

**Recommended fix.** Build with RUSTFLAGS='-C control-flow-guard' (and consider /CETCOMPAT via link-arg) and `--remap-path-prefix` for the cargo home and repo paths.

**Checked and found sound:**
- Update channel: AutoPaper.appinstaller is HTTPS-only (Uri and MainBundle Uri under https://msitarzewski.com/app-updates/autopaper/), has no ForceUpdateFromAnyVersion (Windows will not downgrade), and its MainBundle Publisher is the full certificate DN. publish-windows.sh validates the feed URIs, uploads the bundle and verifies its SHA-256 before it uploads the feed, and refuses to put a DEVELOPMENT BUILD feed on the real path (scripts/publish-windows.sh:90-120).
- Signed bundle (build/windows/0.1.3, unzipped read-only into scratch): the bundle Identity and both inner AppxManifest.xml files carry Publisher 'CN=Michael Sitarzewski, O=Michael Sitarzewski, L=Dallas, S=tx, C=US', Version 0.1.3.0 and uap10:PackageIntegrity Enforcement=on. The AppxSignature.p7x chains to Microsoft Identity Verification Root 2020 with a Microsoft Public RSA timestamp, and the signing certificate's short validity (Oct 8-11 2026) is covered by the timestamp. The winget installer.yaml InstallerSha256 and SignatureSha256 match the bundle (7CAB1613... and FE62DEB0...). No pdb, pfx, cer, .env or key files or credential patterns are in the package.
- Signing script: the Artifact Signing client and SDK build tools are SHA-256 pinned (windows-release.ps1:~60-70), credential sources are limited to a service principal or the Azure CLI (ExcludeCredentials list), the signer subject is compared with the package Publisher after signing, and the signature is timestamped. The dev certificate PFX and password are git-ignored (.gitignore:42) and not tracked.
- Manifest capabilities: runFullTrust, picturesLibrary and unvirtualizedResources only. There is no broadFileSystemAccess, no internetClient/server declared, no URI or file-type associations and no protocol activation, so there is no external launch surface other than the toast COM server and StartupTask (Package.appxmanifest). Both are the standard per-user kinds and the arguments only trigger like/dislike/show/budget/keys actions (App.xaml.cs:OnNotification).
- URL and protocol launching: every Launcher.LaunchUriAsync and NavigateUri target is a hard-coded https or ms-settings: constant (Links.cs:7-16, ProvidersPage.xaml.cs:28-29,790, Notifications.cs:92, XAML NavigateUri attributes). No model, server or file-derived text reaches a launcher. The provenance hyperlinks only navigate inside the app (NowPage.xaml.cs:133-137). Model-provided toast text goes through AppNotificationBuilder.AddText and SecurityElement.Escape (Notifications.cs:49-60,103-106), so toast XML cannot be injected.
- Process creation: the only Process.Start is `explorer.exe /select,"path"` with a path from the engine's own database that passes File.Exists (WallpaperActions.cs:170-176). Windows file names cannot contain quotes, and the renders are named from generation.id (core/src/engine.rs:669), so there is no traversal or argument-injection route.
- Wallpaper, lock screen and registry code (Desktop.cs, OwnWallpaper.cs, OwnLockScreen.cs): registry access is read-only (Registry.CurrentUser/LocalMachine OpenSubKey + GetValue for Spotlight, lock-screen kind and policy flags) with no writes. File writes and deletes go only to LocalState\own-wallpaper, own-lockscreen and workflows with names derived from a SHA-256 of the monitor id or fixed names. The restore targets come from own.json in the user-and-package-SID-only data folder (icacls confirmed in the VM). A swap needs the same user, which SECURITY.md puts out of scope. IDesktopWallpaper and SHParseDisplayName pointers are freed in finally blocks.
- P/Invoke and DLL loading: the P/Invoke surface is small (CsWin32 for the shell COM, Cred* and WTS*, MessageBox, SystemParametersInfo; Program.cs/TrayWindows.cs call ole32/user32/kernel32 system DLLs only). autopaper_core is loaded by bare name with the default search path. In an MSIX the package directory is read-only and ACLed, and the DLL ships in the package root, so there is no writable search location, no SetDllDirectory and no LoadLibrary with a user path.
- Credential handling: CredentialStore uses CRED_TYPE_GENERIC with a bounded blob size, zeroes the plaintext blob, never logs the value, and Log.cs records only exception text (Key_SaveFailed path logs the Win32 error, not the key). There is no OAuth, ChatGPT or device flow in the Windows app, since SignIn.cs is only the StartupTask wrapper, so there are no redirect URIs, state/PKCE or token storage to attack.
- Workflow import: the file picker path reads a .json capped by MaxWorkflowBytes, validates it with OwnWorkflow.Read before use, and stores it in LocalState\workflows (ProvidersPage.xaml.cs:660-705). The console export writes through FileSavePicker/FileIO (the user chooses the destination).
- Single-instance and activation redirection (Program.cs, App.xaml.cs): AppInstance key redirection carries only activation kinds the app handles (launch, StartupTask, AppNotification), and unhandled kinds fall through to showing the window.
- PE hardening: AutoPaper.exe has ASLR, high-entropy VA, NX and CFG, and the app.manifest sets PerMonitorV2 with no requireAdministrator or uiAccess. The bundled model files sit in the read-only package directory under PackageIntegrity.

### Surface: linux

#### linux-1: Signed Flatpak releases rest on unauthenticated CI output (checksum from the same artifact, no provenance, mutable action and container refs)
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/publish-flatpak.sh:115-133; scripts/publish-flatpak.sh:177; scripts/linux-release.sh:196-200; scripts/linux-release.sh:~210; scripts/linux-release.sh:~220; scripts/linux-release.sh:~230; scripts/linux-release.sh:~240; scripts/linux-release.sh:~250

publish-flatpak.sh downloads the bundles with `gh run download` and checks them only against the .sha256 that sits in the same CI artifact, so the check catches corruption, not tampering. It never confirms that the run's head commit is the tag's commit, and the workflow produces no build-provenance attestation. The maintainer's key then signs whatever the bundle holds, so the signature vouches for CI output that was never authenticated. The workflow (.github/workflows/flatpak.yml:34-35,38,50,59) builds inside `ghcr.io/flathub-infra/flatpak-github-actions:gnome-51`, a mutable third-party image tag, with `--privileged`. Its actions are pinned to tags (checkout@v7, cache@v6, upload-artifact@v7), not commit SHAs. `restore-keys` on the cache (line 54) lets a tag build restore state saved under a different manifest hash. The build is otherwise well constrained: contents:read only, an offline `--locked` cargo build, crates pinned by SHA-256, and the model pinned by revision and SHA-256.

**Attack scenario.** An attacker who compromises the container image, one of the actions, or a repo collaborator's ability to run workflow_dispatch gets a malicious bundle into the artifact. They ship a matching .sha256 with it. Running `publish-flatpak.sh --run ID` or `--tag` then signs the bundle with the repository key, and every installed client accepts it as a normal update. This needs a prior compromise of a third-party image or action, or of write access to the repo, so it is a narrow path.

**Evidence.** Code read only. publish-flatpak.sh:133 runs `shasum -c` on a file taken from the same artifact. Nothing compares the run's headSha with the tag, and there is no `gh attestation verify` or SLSA step. The `--run ID` option accepts a run from any event, including workflow_dispatch.

**Recommended fix.** Pin every `uses:` to a full commit SHA and the container image to an `@sha256:` digest. Add `actions/attest-build-provenance` to the workflow, and have publish-flatpak.sh run `gh attestation verify` and check that the run's headSha equals `git rev-parse <tag>^{commit}` before signing. Reject `--run` for non-tag runs, or require a typed confirmation. Drop `restore-keys` for release builds, or key the cache on the full manifest and lock hash only.

#### linux-2: Repository signing key has no passphrase and no expiry, and no revocation or rotation path is documented
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/flatpak-repo-key.sh:11; scripts/flatpak-repo-key.sh:43; scripts/publish-flatpak.sh:157-165; packaging/flatpak/autopaper-repo.gpg

The only trust anchor for every installation is one RSA-4096 key. It was generated with `--passphrase '' ... sign never` and lives unencrypted in ~/.config/autopaper/flatpak-gnupg on the maintainer's Mac. The design comments acknowledge this and rely on FileVault and folder permissions. The key is embedded in every .flatpakref, .flatpakrepo and downloaded bundle (the remote config gets gpg-verify=true and gpg-verify-summary=true). Nothing in the repo describes a revocation certificate, a rotation procedure, or offline or hardware storage. The signed summary carries no freshness data (`ostree summary --view` shows only Last-Modified, no expires field).

**Attack scenario.** Malware or another process running as the maintainer on the Mac reads the key directory, which has no passphrase and does not expire. The attacker then signs malicious commits and summaries. To serve them they also need to compromise or redirect the update host, since TLS to msitarzewski.com still applies. The same signed material can be replayed later, because nothing makes an old signed summary go stale. Clients cannot be moved to a new key without a manual remove and re-add.

**Evidence.** scripts/flatpak-repo-key.sh:43 `gpg --batch --quiet --passphrase '' --quick-gen-key ... rsa4096 sign never`. `ostree summary --view` on the published repo lists no expiry key.

**Recommended fix.** Keep the signing subkey on a hardware token (YubiKey or similar), or at least protect it with a passphrase in the macOS keychain. Create and store offline a revocation certificate, and document a rotation plan (a new key shipped in an app update, with overlap). Consider an expiry on the signing subkey with a primary key kept offline, and a short summary-expiry if the tooling supports it.

#### linux-3: App data directory permissions follow the process umask; AutoPaper does not restrict them itself
- Kind / severity: hardening / low  
- Platforms:   
- Locations: core/src/engine.rs:897; core/src/engine.rs:949; apps/linux/src/desktop.rs:318; apps/linux/src/app.rs:1079; apps/linux/src/desktop.rs:56-58

The Linux app and the core create the data directory tree with create_dir_all and default file creation, and never call set_permissions or set a mode. The tree holds the SQLite history, original images, thumbnails, display renders, run reports, and the copy of the user's own previous wallpaper. In the Scratch VM the modes were not uniform: autopaper/ was 0750 and the originals 0640, but renders/*.jpg were 0644. The mix shows that modes come from whatever umask each process had, not from the app. Under the common umask 022 the whole tree would be 0755/0644 inside Flatpak's 0755 ~/.var/app/<id>. API keys are not in this tree (the Secret portal's file keyring is created 0600/0700 by oo7), so the exposure is the user's keywords, generated ideas, prompts and images.

**Attack scenario.** Another local user on a multi-user machine whose distro leaves home directories traversable (Debian, and older Ubuntu and Fedora setups) lists and reads ~/.var/app/io.github.msitarzewski.AutoPaper/data/autopaper/. They learn the user's keywords, prompts and wallpaper history. Single-user desktops and distros with 0700/0750 homes are not affected.

**Evidence.** Observed in the Scratch VM with `find -printf '%M'`: directory 750, images/*.png 640, renders/*.jpg 644 (mixed modes). A grep of core/src and apps/linux/src finds no permission calls.

**Recommended fix.** Create the data directory with mode 0700 (DirBuilder mode(0o700) plus a one-time chmod of an existing directory). This protects everything below it even if individual files are 0644.

#### linux-4: Flatpak cache and state restore plus floating container tag in the release workflow (CI cache poisoning surface)
- Kind / severity: hardening / info  
- Platforms:   
- Locations: .github/workflows/flatpak.yml:50-54

The workflow restores flatpak-builder's state directory from actions/cache with a `restore-keys` fallback to any earlier `flatpak-<arch>-` cache. flatpak-builder re-checks source checksums and rebuilds modules whose inputs changed, so a poisoned cache would have to match the manifest hash. The remaining risk is a cache written by a workflow_dispatch run on a non-default branch by a collaborator, which a later tag build can restore.

**Attack scenario.** A user with write access who already has other ways to tamper with the release can seed the cache with a crafted state directory. This adds nothing meaningful beyond linux-1.

**Evidence.** Code read only. Not exercised.

**Recommended fix.** Do not use restore-keys for tag builds, or build tags with no cache at all. The cache is a convenience, not a requirement.

**Checked and found sound:**
- Flatpak finish-args match the manifest and are minimal. `flatpak info --show-permissions` in the Scratch VM shows only shared=network;ipc, sockets=wayland;fallback-x11, devices=dri, and talk to org.kde.StatusNotifierWatcher (packaging/flatpak/io.github.msitarzewski.AutoPaper.yml:19-37). There is no --filesystem, no --talk-name for org.freedesktop.secrets, Notifications or flatpak, no --device=all, and no --socket=session-bus or system-bus.
- Sandbox boundary tested dynamically. From `flatpak run --command=sh`, gdbus ListNames on the session bus returns only org.freedesktop.DBus, org.freedesktop.portal.Desktop, org.freedesktop.portal.Documents, org.freedesktop.portal.IBus, the app's own name, org.kde.StatusNotifierWatcher and unique names. /run/host exposes only fonts, os-release and share, not user files.
- Update chain verified end to end in the Scratch VM. A remote added with the committed autopaper-repo.gpg (gpg-verify=true, gpg-verify-summary=true) installs the real repo. A rogue repo carrying the same bundle but signed with another key is refused (`Can't check signature: public key not found`). A repo with its signatures removed is refused (`GPG verification enabled, but no summary signatures found`). The client uses summary.idx and summaries/*.idx.sig, so the legacy `summary` file is not the trust path. The key fingerprint in the shipped remote (6D6197DA7830EF13C5C25AC1005CC961E908702D) matches the committed public key.
- Install files are sound. AutoPaper.flatpakref and autopaper.flatpakrepo use https URLs and embed the GPG key. Release bundles carry the https repo URL and the Flathub runtime-repo (scripts/publish-flatpak.sh:157-175).
- Supply chain pins are good. cargo-sources.json has 470 crates.io archives with SHA-256, matching the 470 registry entries in Cargo.lock, with no git sources. The build runs `cargo build --offline --locked`. The embedding model is pinned to a Hugging Face revision with SHA-256 for all five files (manifest lines 49-66). The shipped config.json, model.safetensors and tokenizer.json in the installed 0.1.3 bundle hash to exactly those pins. The model is safetensors, not pickle.
- Shipped bundle contents are clean. The file list of the installed 0.1.3 aarch64 app has no setuid files, dotfiles other than .ref, .env or key material. A strings scan of the binary finds no key-shaped literals or home-directory paths. Source dirs for the build (core, cli, uniffi-bindgen, apps/linux) contain no .env or stray files.
- Secrets: apps/linux/src/secrets.rs uses oo7. Inside Flatpak this is the file backend keyed by the Secret portal. oo7 creates the keyring file 0600 and its directory 0700 (oo7-0.6.0 src/file/api/mod.rs:137,155). Keys are not logged (errors carry the account name only). Outside the sandbox it uses the Secret Service with attributes (application, account).
- Wallpaper setting (apps/linux/src/desktop.rs:155-181) uses the Wallpaper portal with an open file descriptor (build_file). There is no subprocess, gsettings CLI, dbus-send or shell, and no string-built path or command. The GNOME restore path writes only a fixed whitelist of org.gnome.desktop.background keys through gio::Settings, and is disabled when sandboxed (desktop.rs:239-243, 326-333). The core writes images through an exclusive temp file plus rename (core/src/imaging.rs:294-320), so planted symlinks are not followed.
- URL and file handling. Every desktop::open_uri call site passes a compile-time constant or a fixed brewbrowser:// URI (welcome.rs:225-237, preferences.rs:154-160, 1036-1039, window.rs:963). Model-influenced text such as idea titles, summaries and model lists is shown in gtk::Label (plain by default), in Adw rows with use_markup(false), in Toasts with use_markup(false), or is explicitly escaped (window.rs:523, welcome.rs:117,269, console.rs:552,565). Model-list entries render through plain Labels (preferences.rs:1391-1421). Tray labels escape '_' mnemonics (tray.rs item/text). The workflow file chooser and the run-export dialog use FileDialog, which goes through the FileChooser portal. Export uses FileCreateFlags::PRIVATE.
- ComfyUI workflow substitution JSON-escapes the prompt (core/src/providers/comfyui.rs:38-39, 1018-1032), so a prompt-injected model response cannot add nodes or break out of the string.
- Desktop, D-Bus and metainfo. The .desktop file (Exec=autopaper-gtk, DBusActivatable=true) and the D-Bus service file (Exec=@bindir@/autopaper-gtk --gapplication-service) are fixed, with no user-controlled substitutions beyond the install prefix. The metainfo URLs are all https. The app has no listening sockets, and its only command-line option is --background.
- Flatpak publish script. rsync and ssh targets are fixed and confined to /srv/www/msitarzewski.com/app-updates/autopaper/ with umask 027. The remove-test path deletes only three named paths. The signing key is copied into container memory from a read-only mount, with no secret printed. The ordering (objects, then indexes, then deletions) stops clients seeing a summary that names missing objects.
- CI workflow hygiene. permissions is contents:read, only tag pushes and workflow_dispatch trigger it, the tag is passed through env and not interpolated into the shell, and there are no pull_request triggers or secrets exposed.
- Network credential logging. net.rs:318 marks auth header values sensitive, and error text is passed through redact() before logging, so RUST_LOG debug does not print keys.

### Surface: supply-chain

#### supply-chain-1: GitHub Actions pinned to mutable tags/branches, not commit SHAs
- Kind / severity: hardening / low  
- Platforms:   
- Locations: .github/workflows/ci.yml:30-33,54,57,127; .github/workflows/flatpak.yml:38,50,59; .github/workflows/pages.yml:31,36,37,49

Every third-party action is referenced by a movable tag (actions/checkout@v7, gitleaks/gitleaks-action@v3, Swatinem/rust-cache@v2, actions/cache@v6, upload-artifact@v7, deploy-pages@v5) and dtolnay/rust-toolchain by the floating branch @stable. A compromised or retagged upstream action runs with the job's token. Blast radius is limited: workflows are contents: read, use no secrets beyond GITHUB_TOKEN, and nothing is signed in CI. The one sensitive job is flatpak.yml, whose output the maintainer later signs, and pages.yml, which has id-token: write.

**Attack scenario.** An upstream action repo (e.g. Swatinem/rust-cache or dtolnay/rust-toolchain) is compromised and its tag/branch moved. The flatpak.yml build job runs it inside the container that builds the bundle. The malicious step swaps the Flatpak bundle's contents. The maintainer's publish-flatpak.sh then signs it with the repository GPG key (see supply-chain-2). Needs an upstream compromise.

**Evidence.** grep 'uses:' shows only @vN / @stable refs; no 40-hex SHAs anywhere. No .github/dependabot.yml exists to bump pins.

**Recommended fix.** Pin each action to a full commit SHA with a version comment, add dependabot for github-actions, and replace dtolnay/rust-toolchain@stable with a pinned SHA plus an explicit toolchain version.

#### supply-chain-2: Flatpak publish signs CI bundles on a checksum that comes from the same artifact (integrity, not authenticity)
- Kind / severity: hardening / medium  
- Platforms:   
- Locations: scripts/publish-flatpak.sh:110-122; scripts/publish-flatpak.sh:127-139; scripts/linux-release.sh:196; scripts/publish-flatpak.sh:184-200; .github/workflows/flatpak.yml:3-9,59-66

publish-flatpak.sh downloads the bundle and its .sha256 from the same workflow artifact and checks one against the other. That detects corruption only. Anyone who can alter the artifact can alter both files. The run is chosen by `gh run list --workflow flatpak.yml --branch $TAG --event push --status success`. A branch named like the tag also matches `--branch`. The script then imports the bundle and signs it into the live repository with the unattended, passphrase-less key, which every Flatpak install of AutoPaper trusts. No provenance check ties the bundle to the tagged commit or to the expected workflow: no artifact attestation, no check of run headSha against the tag, and no rebuild.

**Attack scenario.** An attacker with repo write access (stolen PAT or collaborator), or a compromised action in flatpak.yml (see supply-chain-1), produces a successful push-event run whose artifact is a backdoored bundle with a matching .sha256. This can be a run on a branch named v0.x.y, or a tag pushed at a malicious commit. The maintainer runs publish-flatpak.sh --tag, the script signs the bundle, and every Flatpak user receives it as a trusted update. Needs write access or a CI supply-chain compromise, so it is not a vulnerability by itself.

**Evidence.** Code read only; no PoC. The checksum check at lines 127-139 uses the file downloaded in the same `gh run download`. The run lookup at lines 115-117 filters by branch name, not by head SHA or ref type.

**Recommended fix.** Add actions/attest-build-provenance in flatpak.yml and run `gh attestation verify` in publish-flatpak.sh before signing. Also check the run's headSha equals `git rev-parse $TAG^{commit}` and headBranch is a tag, and only publish bundles whose commit is on main. Protect v* tags with a tag ruleset and require review.

#### supply-chain-3: Sparkle dependency and its sign_update tool are resolved by version tag with no committed resolution (Package.resolved is not tracked)
- Kind / severity: hardening / low  
- Platforms:   
- Locations: apps/macos/project.yml:22-26; scripts/release.sh:96-102; scripts/release.sh:164-166; .gitignore:36-38

The Sparkle framework that ships inside the notarized app, and the sign_update/generate_keys binaries run on the signing Mac against the EdDSA private key in the login Keychain, both come from the SwiftPM package `exactVersion: 2.10.0`. Package.resolved is not committed: the xcodeproj and xcworkspace are gitignored, and `git ls-files` shows no Package.resolved. The upstream git revision and binary-target checksum are therefore not pinned in this repository, and a retagged or compromised upstream tag would be picked up silently at the next release. By contrast the Rust side (Cargo.lock with --locked), the embedding model (SHA-256), the NuGet signing tools (SHA-256) and the flatpak-cargo-generator (SHA-256) are all pinned.

**Attack scenario.** Sparkle's GitHub tag 2.10.0 is force-moved, or its release artifact is replaced. The next release.sh run on the maintainer Mac embeds the altered framework in the signed app, and runs the altered sign_update with Keychain access to the update-signing key. Needs an upstream compromise.

**Evidence.** git ls-files has no Package.resolved. `exactVersion` pins a tag, not a revision. release.sh runs `xcodebuild -resolvePackageDependencies` into build/SourcePackages at release time.

**Recommended fix.** Use XcodeGen's `revision:` pin for Sparkle, or commit a Package.resolved and pass -disableAutomaticPackageResolution. In release.sh, verify the sign_update binary against a recorded SHA-256 or Sparkle's published checksum.

#### supply-chain-4: Notarization app-specific password and dev-signing PFX password passed on command lines (visible in process list)
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/release.sh:62; scripts/release.sh:68; scripts/windows-release.ps1:400

release.sh passes APPLE_PASSWORD as `notarytool --password`, and APPLE_ID/team on the same line. Command-line arguments are readable by other local processes via ps while notarytool runs, which can take minutes because of `--wait`. windows-release.ps1 passes the development PFX password to signtool with /p. The Apple password is an app-specific password for the maintainer's Apple ID, usable for notarization and App Store Connect-style APIs. The Windows dev PFX is only trusted on dev machines and VMs (windows-app.ps1:26 imports it into LocalMachine\TrustedPeople), and its password sits in a file next to it under the gitignored AppPackages/.

**Attack scenario.** Another local user or malware running as a different user on the release Mac or the Windows dev VM samples the process list during notarization or signing, and captures the password. Needs a local foothold on the release machine.

**Evidence.** Lines 62 and 68 show `--password "$APPLE_PASSWORD"` literally. The ignore rules cover AppPackages/, so the PFX and its password file are not committed.

**Recommended fix.** Store notary credentials once with `xcrun notarytool store-credentials` and use `--keychain-profile`. For the dev PFX, use signtool /csp /kc with a certificate imported into the store, or accept the risk since the key is dev-only.

#### supply-chain-5: Sparkle appcast is unsigned: release-note HTML and release-page links are unauthenticated
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/appcast.py:50-62; apps/macos/project.yml:66-71; site/static/appcast.xml:1-15; .github/workflows/pages.yml:7-18

Update payloads are protected by Sparkle's EdDSA enclosure signature, so a feed attacker cannot install code. The feed itself is not signed (SURequireSignedFeed is not set; Info.plist keys are limited to SUFeedURL, SUPublicEDKey and the installer launcher). Everything else in each item is therefore unauthenticated: the inline <description> CDATA HTML shown in the update window, fullReleaseNotesLink (opened in the browser), and sparkle:version/minimumSystemVersion. SUVerifyUpdateBeforeExtraction is also not set, so the archive is unpacked before the signature check on older Sparkle behaviour. The feed is published from main by the Pages workflow, so the trust root for those fields is anyone able to push to main or alter the Pages deployment.

**Attack scenario.** An attacker with push access to main, or a hijacked Pages deployment, edits appcast.xml to add HTML with a phishing link in the update window, or sets sparkle:version very high with no valid signature. That causes at most a nag, a phishing link, or an update-check failure loop (DoS of updates). No code execution without the EdDSA key.

**Evidence.** appcast.py interpolates `notes` into <![CDATA[...]]> unescaped and unsigned. project.yml sets no SURequireSignedFeed or SUVerifyUpdateBeforeExtraction.

**Recommended fix.** Sign the appcast (Sparkle 2.9+ generate_appcast/sign_update support signed feeds) and set SURequireSignedFeed plus SUVerifyUpdateBeforeExtraction in project.yml. Protect main with required reviews.

#### supply-chain-6: pages.yml grants pages: write and id-token: write to the build job that runs pandoc and repo scripts
- Kind / severity: hardening / info  
- Platforms:   
- Locations: .github/workflows/pages.yml:18-21; .github/workflows/pages.yml:27-37

Permissions are set at workflow level (contents: read, pages: write, id-token: write), so the build job, which runs apt-get, pandoc and scripts/build_site.py, also holds the OIDC token permission it does not need. Only the deploy job needs pages/id-token write. The workflow triggers only on push to main and workflow_dispatch, not on PRs, so forks cannot reach it.

**Attack scenario.** A compromised step in the build job (e.g. a poisoned apt package) mints the OIDC token and deploys arbitrary content to the Pages site, which includes the Sparkle appcast. Needs a CI compromise, and the EdDSA signature still protects installs.

**Evidence.** permissions block at workflow scope; the deploy job has no separate permissions.

**Recommended fix.** Set `permissions: contents: read` on the build job, and `pages: write, id-token: write` on the deploy job only.

#### supply-chain-7: uniffi-bindgen-cs installed from a git tag at build time (build tool for the Windows C# bindings)
- Kind / severity: hardening / info  
- Platforms:   
- Locations: .github/workflows/ci.yml:131; scripts/build-core-windows.ps1:29,119-122

The generator that writes the C# bindings for the shipped Windows core is `cargo install --git ... --tag v0.11.0+v0.31.0 --locked`. The tag is mutable, `--locked` only constrains its dependencies, and no revision (`--rev`) or checksum is pinned. The generator output is compiled into the release app, so a moved tag would inject code into the Windows build. In CI it runs with no secrets. On the maintainer's machine it runs before signing.

**Attack scenario.** NordSecurity/uniffi-bindgen-cs tag is moved, or the repo is compromised. The next Windows release build generates and compiles attacker-controlled bindings, and the maintainer signs the result. Needs an upstream compromise.

**Evidence.** Both call sites use `--tag`, not `--rev`.

**Recommended fix.** Install with `--rev <full commit sha>` and record the expected commit in build-core-windows.ps1's version check.

#### supply-chain-8: Flatpak repository signing key is an unencrypted GPG key with no expiry, kept on the release Mac and mounted into Docker
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/flatpak-repo-key.sh:9-14,37-41; scripts/publish-flatpak.sh:145-152; packaging/flatpak/autopaper-repo.gpg

The repository key (RSA 4096, fingerprint 6D6197DA7830EF13C5C25AC1005CC961E908702D) has no passphrase and no expiry. It sits in ~/.config/autopaper/flatpak-gnupg, protected only by file permissions and FileVault. It is bind-mounted read-only into a Docker container using an image built from debian:trixie-slim with unpinned apt packages (flatpak, ostree, gnupg). The design is documented and mitigated: the secret is never printed, the mount is read-only, the container is --rm. There is no revocation certificate or rotation path, and every installed remote embeds this one key through the .flatpakref/.flatpakrepo GPGKey, so a leak means clients keep trusting it until manually re-added.

**Attack scenario.** Malware running as the maintainer, or a compromised Debian package in the publish image, reads the key and signs a malicious OSTree commit. An attacker who also controls msitarzewski.com, or can MITM past TLS, then pushes it to all Flatpak users. Needs a compromised release machine or the pipx host, which is also a trust-root compromise.

**Evidence.** flatpak-repo-key.sh header: 'no passphrase', 'no expiry'. publish.Dockerfile installs from apt without version pins.

**Recommended fix.** Keep the key on a hardware token or macOS Keychain-backed GPG agent, or generate a subkey for signing with an expiry. Store a revocation certificate offline. Pin the Debian base image by digest.

#### supply-chain-9: Flatpak build cache restored with a prefix restore-key; cache key excludes source
- Kind / severity: hardening / info  
- Platforms:   
- Locations: .github/workflows/flatpak.yml:48-56

The flatpak-builder state dir (downloaded sources and previously built modules) is restored with `restore-keys: flatpak-<arch>-`, so any earlier cache with that prefix matches, including caches written by manual workflow_dispatch runs on other branches by anyone with write access. Downloaded sources are checksum-checked by flatpak-builder, but cached built-module output is not independently re-verified. Fork PRs cannot reach this workflow because it triggers only on v* tags and workflow_dispatch.

**Attack scenario.** A user with write access runs workflow_dispatch on a branch and writes a poisoned flatpak-x86_64-<hash> cache. A later tag build restores it by prefix and ships stale or poisoned module output. This needs write access, and the same user could simply push a tag, so the gain is stealth.

**Evidence.** cache step uses `restore-keys: flatpak-${{ matrix.arch }}-` and the key hashes only packaging/flatpak/*.yml and cargo-sources.json.

**Recommended fix.** For tag release builds, skip cache restore (or use exact-key only) so the shipped bundle is built from a clean state.

**Checked and found sound:**
- CI workflow permissions are least-privilege: top-level `contents: read` in ci.yml:14-15 and flatpak.yml:17-18. No pull_request_target anywhere. CI jobs read no repo secrets beyond the default GITHUB_TOKEN, so fork PRs cannot reach any.
- No untrusted-input injection into run: steps. The only github-context use in a run is the tag, passed through `env: TAG` and quoted (flatpak.yml:41-48). Other `${{ }}` uses are matrix values and concurrency group names only.
- Artifact upload (flatpak.yml:59-66) contains only the bundle and its .sha256 from build/flatpak/out. No .env, credentials or cache dirs are uploaded, and `if-no-files-found: error` is set.
- Gitleaks: `gitleaks git --redact` over all 20 commits across all local branches (main, codex/console-runs, local-inference-timeouts) reports no leaks, both with .gitleaks.toml and with it disabled. A grep of tracked files for PRIVATE KEY, AKIA, ghp_ and xox tokens finds nothing. No deleted .env, .pfx, .key, .pem or gnupg files in history. .env, .env.*, AppPackages/ (dev PFX and its password file) and build/ are gitignored. The allowlists in .gitleaks.toml are narrow exact-shape fixtures, and the allowlisted Sparkle value is a public key, not a private one.
- Sparkle trust root: apps/macos/project.yml:71 embeds SUPublicEDKey. release.sh:96-102 refuses to release if the Keychain key does not match it, requires an https feed URL, hardened runtime, sandbox, and no get-task-allow, and checks every nested executable's signature and linked libraries. The enclosure URLs in site/static/appcast.xml are https GitHub release URLs and every item carries edSignature. Updates are version-ordered by Sparkle, so no downgrade.
- Windows update chain: windows-release.ps1 verifies the bundle signature (signtool verify /pa) and the signer thumbprint. The App Installer feed is built with Publisher from the signing certificate subject and ASCII-only (535-543). publish-windows.sh:80-96 confines both URIs to the base URL by whitelist regex with no `..`, requires the bundle name to match the version, refuses a dev-build feed at the real address (L100-105), uploads the bundle and checks its SHA-256 on the server before the feed goes up last (L131-149), and never replaces a published release. Remote rm paths are built only from regex-validated names. Winget manifests carry InstallerSha256 and SignatureSha256. Signing tools are NuGet packages pinned by version and SHA-256 (windows-release.ps1:166-167,226-231). Azure auth is restricted to env service principal or az login via ExcludeCredentials (L405-413).
- Flatpak: the manifest pins the embedding model by Hugging Face commit and SHA-256 (io.github.msitarzewski.AutoPaper.yml:52-66), builds offline from cargo-sources.json which is cross-checked against Cargo.lock (linux-release.sh:108-119), and pins flatpak-cargo-generator by commit and SHA-256 (L32-33,126). finish-args are narrow (network, wayland/x11, dri, ipc, one tray bus name). `.flatpak` bundles are published only through signed OSTree commits.
- Shell script hygiene: release.sh, publish-flatpak.sh, publish-windows.sh, pipx-app-updates.sh and fetch-model.sh all use `set -euo pipefail` (or `set -eu`), quote variables, and use mktemp -d with traps. There is no eval, no curl|sh, and no StrictHostKeyChecking=no. ssh uses BatchMode, rsync runs under umask 027, and server paths are constants under /srv/www/msitarzewski.com/app-updates/autopaper. fetch-model.sh uses https-only with SHA-256 verification before the files are renamed into place. pipx-app-updates.sh refuses to run as root, backs up the Caddy config and restores it if validation fails, and only touches its marked block.
- Dependencies: Cargo.lock is committed and CI/builds use `--locked`. Windows NuGet references use exact versions. Sparkle is exact-versioned (but see supply-chain-3).
- Static site: site/layout.html and site/pages contain no `<script>` elements, no external subresources, and no third-party requests beyond ordinary links. The footer states it loads nothing from elsewhere, which matches the code. build_site.py html-escapes titles and descriptions, and its inputs are repo-authored markdown and fragments. GitHub Pages cannot set a CSP header, and there is no inline script to need one.
- SECURITY.md claims about updates (Sparkle EdDSA, signed MSIX plus App Installer feed, Flatpak) and NETWORK.md's update section (no system profiling, EdDSA verification, GPG-signed Flatpak repo) match project.yml, appcast.py, windows-release.ps1 and publish-flatpak.sh. The disclosure channels are present and workable.
- Public Flatpak key packaging/flatpak/autopaper-repo.gpg contains only a public key (RSA 4096, matching the fingerprint documented in scripts).

### Surface: deps

#### deps-1: Windows release builds the C# bindings with an unlocked, tag-pinned `cargo install` from git
- Kind / severity: hardening / low  
- Platforms:   
- Locations: scripts/build-core-windows.ps1:29; scripts/build-core-windows.ps1:117-126

The generator whose output (autopaper_core.cs) is compiled into the signed Windows app is installed by hand with `cargo install uniffi-bindgen-cs --git ... --tag v0.11.0+v0.31.0`. There is no `--locked`, and a git tag can be moved. The installed version is checked only via `cargo install --list` against a version string, and a missing entry is a warning (line 126), not an error. The generator's own transitive dependencies are resolved fresh at install time, outside AutoPaper's Cargo.lock.

**Attack scenario.** If the NordSecurity tag were retargeted, or one of the generator's unlocked transitive crates were compromised, the generated C# would contain attacker code. That code would be built into the signed Windows release. This needs an upstream compromise, so it is a supply-chain hardening gap, not a directly exploitable bug.

**Evidence.** Read in code; not demonstrated. `--locked` is absent, the pin is a tag rather than a commit hash, and the `listed` check at line 126 only warns.

**Recommended fix.** Pin to a commit with `--rev <sha> --locked`, or vendor the generator's Cargo.lock. Make a missing or mismatched install a hard error. Record the generated .cs hash or review the generated diff at release time.

#### deps-2: CI and release builds use floating action tags and an unpinned Rust toolchain, and there is no dependency-audit gate
- Kind / severity: hardening / low  
- Platforms:   
- Locations: .github/workflows/ci.yml:30,53,54,57,100,101,122,123,127; gitleaks/gitleaks-action@v3 (ci.yml:33); .github/workflows/flatpak.yml:38,50,59; .github/workflows/pages.yml:31-49; scripts/build-core-windows.ps1:107; scripts/build-xcframework.sh:111

Every GitHub Action is referenced by a mutable major tag (checkout@v7, rust-toolchain@stable, rust-cache@v2, gitleaks-action@v3, and so on) rather than a commit SHA. There is no rust-toolchain.toml, so CI uses `stable` and the release scripts use `rustup run stable`. The workspace only declares rust-version 1.92. No workflow runs cargo-audit or cargo-deny, and there is no deny.toml, dependabot or renovate config. Cargo.lock is committed and CI uses `--locked`, which is good. The macOS and Windows release scripts do not use `--locked`: build-xcframework.sh:111 runs `cargo build` without it, and build-core-windows.ps1:107 runs `cargo rustc` without it.

**Attack scenario.** A moved action tag, such as a compromised third-party action, runs in jobs that hold the repository checkout. Of these workflows only pages.yml has a deploy-pages job. It holds `pages: write` and `id-token: write`, though the signing keys are not in CI. Separately, a newly published RustSec advisory would not fail any build. If Cargo.lock is out of sync at release time, the release scripts silently re-resolve dependencies instead of failing.

**Evidence.** cargo audit against Cargo.lock was clean apart from one unmaintained warning (see checked_ok). Nothing in .github/ runs cargo audit or cargo deny.

**Recommended fix.** Pin actions to full commit SHAs and let Dependabot bump them. Add a rust-toolchain.toml. Add a cargo-audit or cargo-deny CI job, scheduled, with a deny.toml. Add `--locked` to the release-script cargo invocations.

#### deps-3: NuGet dependencies for the Windows app are not locked, and transitive versions float
- Kind / severity: hardening / low  
- Platforms:   
- Locations: apps/windows/AutoPaper/AutoPaper.csproj:82-91; apps/windows/AutoPaper.Tests/AutoPaper.Tests.csproj:142-143

The direct PackageReferences use exact versions (WindowsAppSDK 2.5.1, CsWin32 0.3.346, CommunityToolkit.Mvvm 8.4.2, WinUIEx 2.9.3, and others). There is no packages.lock.json (RestorePackagesWithLockFile is unset), no Directory.Packages.props and no nuget.config with source mapping or signature validation. Transitive dependencies such as the Microsoft.WindowsAppSDK.* packages therefore resolve at restore time to the lowest applicable version, and different machines can produce different results. A restore uses the default NuGet sources with no package-source mapping, which leaves room for dependency confusion.

**Attack scenario.** This needs a poisoned feed or a package compromise at release-build time. It is a narrow supply-chain path. The impact would be code inside the signed MSIX.

**Evidence.** git ls-files shows no packages.lock.json, nuget.config or Directory.Packages.props. I could not run `dotnet list package --vulnerable --include-transitive`, because dotnet is not installed on the host. NuGet advisory status for these versions is therefore not verified with a tool.

**Recommended fix.** Set RestorePackagesWithLockFile=true and commit packages.lock.json. Restore with `--locked-mode` in CI and release. Add a nuget.config with packageSourceMapping and signatureValidationMode=require.

#### deps-4: The embedding model is verified at fetch and in Flatpak, but not when it is bundled or loaded
- Kind / severity: hardening / info  
- Platforms:   
- Locations: scripts/macos-bundle-model.sh:18-35; scripts/windows-release.ps1:434-436; core/src/embed.rs:112-146

The pinned Hugging Face revision and SHA-256s in scripts/fetch-model.sh:19-24 are enforced when the model is downloaded. The Flatpak manifest independently pins URL and sha256 (packaging/flatpak/io.github.msitarzewski.AutoPaper.yml:52-59). release.sh:95 fetches and verifies the model for macOS releases. The macOS dev bundle script and the Windows release script only check that the files exist. The runtime loader reads the files without a hash check. The weights are read with `VarBuilder::from_buffered_safetensors`, so there is no mmap and no pickle, which limits the parser attack surface.

**Attack scenario.** A modified models/ directory on a build machine would be bundled and signed without any check. A local attacker who can write the install directory could swap the model, but the same attacker could replace the binary, so the runtime check adds little.

**Evidence.** I hashed the three local files in models/bge-small-en-v1.5. They match the pinned SHA-256s in fetch-model.sh. The models/ directory is git-ignored.

**Recommended fix.** Have windows-release.ps1 and macos-bundle-model.sh re-verify the three SHA-256s (or call `fetch-model.sh <dir>`, which is idempotent and verifying) before packaging.

#### deps-5: RUSTSEC-2024-0436: `paste` 1.0.15 is unmaintained
- Kind / severity: hardening / info  
- Platforms:   
- Locations: Cargo.lock (paste 1.0.15)

This is the only cargo-audit result across 474 crates. `paste` is a proc-macro that runs at compile time only. It is pulled in transitively and is not part of any runtime path, so the advisory is unmaintained status, not a vulnerability.

**Attack scenario.** There is no runtime attack path. The residual risk is only that it will not be patched.

**Evidence.** cargo-audit 0.22.2 against a copy of Cargo.lock with the advisory DB at 1296 advisories: `1 allowed warning found`, RUSTSEC-2024-0436. There were no vulnerabilities and no yanked crates.

**Recommended fix.** Track upstream migration to `pastey`, and record the advisory as an explicit ignore with a reason once deny.toml exists.

#### deps-6: No `#![forbid(unsafe_code)]` guard in the workspace crates
- Kind / severity: hardening / info  
- Platforms:   
- Locations: core/src/lib.rs; apps/linux/src/main.rs; cli/src/main.rs

The workspace's own Rust crates contain no `unsafe` blocks and no `extern` blocks. The only grep hit is a comment at core/src/embed.rs:112. Nothing in the source prevents unsafe code being added later. The C# project sets AllowUnsafeBlocks=true (AutoPaper.csproj:28), which CsWin32 and the UniFFI C# bindings need, and a grep of the hand-written C# found no `unsafe`, DllImport or Marshal use.

**Attack scenario.** None today. This is a regression guard.

**Evidence.** grep for `unsafe` and `extern "` over core, apps/linux, cli and uniffi-bindgen found only the comment.

**Recommended fix.** Add `#![forbid(unsafe_code)]` to core, cli and apps/linux, if the generated UniFFI scaffolding allows it (use `deny` with a scoped allow if not).

**Checked and found sound:**
- cargo audit (0.22.2, advisory DB 1296 entries, Cargo.lock 474 crates, also checks yanked): no vulnerabilities, no yanked crates. Only RUSTSEC-2024-0436 (paste, unmaintained, build-time proc-macro).
- Key networking and decoding crates are current: rustls 0.23.45, rustls-webpki 0.103.15, aws-lc-sys 0.45.0, hyper 1.11.1, h2 0.4.19, tungstenite 0.30.0, quinn-proto 0.11.19, tokio 1.53.2. The image decoders are image 0.25.10, png 0.18.1, zune-jpeg 0.5.15, image-webp 0.2.4. Bundled SQLite via libsqlite3-sys 0.38.2.
- Cargo.lock is committed (git ls-files). Every source is the crates.io registry, with no git or path dependencies other than workspace members. CI runs clippy and test with `--locked` (ci.yml:57,59,etc.). The Flatpak build is `--offline --locked` (flatpak.yml, packaging/flatpak/io.github.msitarzewski.AutoPaper.yml:82), with cargo-sources.json cross-checked against Cargo.lock by linux-release.sh:100-135. The flatpak-cargo-generator is pinned by commit and SHA-256.
- uniffi is pinned with `=0.31.2` (Cargo.toml:15) and the generator version is explained in a comment.
- The workspace crates have no unsafe blocks or FFI declarations beyond UniFFI-generated scaffolding. UniFFI's own scaffolding is not hand-written.
- UniFFI callback boundary: the host-supplied SecretStore is wrapped in GuardedSecrets, which catch_unwinds a foreign exception (core/src/engine.rs:1044-1065). Progress observers are wrapped in catch_unwind too (engine.rs:1649, 2841), so a failing host callback cannot take down the core. Neither the release profile nor any other profile sets panic=abort.
- The only build.rs in the workspace is apps/linux/build.rs. It runs glib-compile-resources and glib-compile-schemas on in-repo data, copies the schema into OUT_DIR, and has no network access. A scan of the build.rs files in the local cargo registry found no download or network code in the dependencies. The proc-macros are uniffi_macros and thiserror (compile-time only).
- Untrusted image decoding is bounded by DecodeLimits (16384 px a side, 64 MP) in core/src/imaging.rs:49-62,101,170.
- Model supply chain: scripts/fetch-model.sh pins the Hugging Face revision (5c38ec7c...), allows only https and TLS 1.2+, caps redirects, and verifies SHA-256 for each file before an atomic rename. The three local model files match the pinned hashes. The Flatpak manifest pins sha256 independently. The weights load via from_buffered_safetensors with no mmap (core/src/embed.rs:112,139).
- Sparkle is pinned with exactVersion 2.10.0 (apps/macos/project.yml:24-26), and the feed URL is https (project.yml:105). I did not check Sparkle 2.10.0 or the NuGet versions against an advisory feed; no dotnet or Swift advisory tooling was available. This is a gap in what was verified, not a finding.
- The Windows Win32 surface is limited to the CsWin32 list in apps/windows/AutoPaper/NativeMethods.txt (wallpaper COM, Credential Manager, session notifications, MessageBox). No hand-written P/Invoke or Marshal use was found.
- Windows release refuses to ship without the model unless -DevCert is passed (scripts/windows-release.ps1:434-436).

### Surface: privacy-claims

#### privacy-claims-1: Mood keywords are written to system logs although PRIVACY.md says no keywords appear in any diagnostic log
- Kind / severity: vulnerability / low  
- Platforms: Linux, macOS  
- Locations: core/src/engine.rs:2331-2335; core/src/engine.rs:2629; core/src/error.rs:42; apps/linux/src/main.rs:27-29; apps/macos/AutoPaper/App/AppModel.swift:668-669; PRIVACY.md (Diagnostics row of 'What's stored on your computer'); site/pages/index.html:242 (never logs keys)

PRIVACY.md says every diagnostics channel carries 'No keys, keywords or prompts'. That is untrue in two places. (1) The core logs `tracing::warn!(problems = ?said, ...)` with `problem_text()` output such as `the prompt leaves out the Must keyword “mountains”` (engine.rs:2334, problem_text at :3117-3120). It also logs the final failure text at :2629, and AutoPaperError::KeywordNotFollowed prints the keyword (error.rs:42). The Linux app installs a stderr subscriber at default filter `info,zbus=error` (main.rs:27-29), so these WARN lines go to stderr. Stderr of a GNOME or Flatpak launched app is normally captured by the user journal (not verified in a VM). (2) The macOS `report()` logs every engine error with `privacy: .public` (AppModel.swift:669). `String(describing: KeywordNotFollowed(keyword:...))` therefore lands unredacted in the persistent unified log. Windows is not affected: AppModel.cs:833 skips AutoPaperException when logging. Provider error text (up to 200 chars from a server's JSON error.message, net.rs:459-465) is also relayed into these logs with only pattern-based redaction. A short custom key, such as an OpenAI-compatible server token, is not caught by `redact()`, which is a hardening gap. Impact is limited to someone who can read the user's logs, but it contradicts a documented promise.

**Attack scenario.** A user with a Must keyword the writer keeps ignoring (for example a family member's name) triggers a failed generation. The keyword, with its weight, is persisted in `log show` on macOS or in the journal on Linux. A support bundle, `journalctl` output, a sysdiagnose or a pasted log then discloses it. The doc says it cannot.

**Evidence.** Reproduced the core behaviour in scratch (copy of core and cli, tracing-subscriber added with the same filter as apps/linux/src/main.rs, writer = my local fake OpenAI-compatible server returning prompts without the Must keyword). `autopaper generate` stderr: `WARN autopaper_core::engine: the text model's candidates couldn't be used problems=["the prompt leaves out the Must keyword “mountains”", ...]` and `WARN ... a wallpaper couldn't be made status=Failed error=none of the text model's ideas followed the keywords: Must keyword "mountains"`. Scratch: /private/tmp/claude-501/-Users-michael-Clean-autopaper/8ebed984-7495-4efc-b80f-6f0aae7b74d6/scratchpad/poc/gen.err. The macOS path is established by code reading only (not run).

**Recommended fix.** Do not put keyword text in tracing fields: log counts and problem kinds only (`MissingMust`, `MentionsAvoid`) and drop `keyword` from the Display of KeywordNotFollowed, or make the Display generic and keep the keyword in a typed field. In macOS `report()`, log a redacted variant (error case name only) and use `privacy: .private` for anything else. Redact exact secret values (not only patterns) before provider error text enters any log. Alternatively, amend PRIVACY.md to say keywords can appear in error logs.

**Verifier: downgraded (severity info).** The behaviour is real. The core logs mood keywords at WARN on a failed generation (engine.rs:2334 and :2629, with the keyword in error.rs:42). The Linux app sends tracing output to stderr (main.rs:27-29). The macOS report() logs String(describing: error) with privacy: .public (AppModel.swift:669). It is reached from the generation error paths (e.g. :525). A UniFFI error enum's description normally includes its associated values, so the keyword probably lands in the unified log, but I did not run that. PRIVACY.md line 90 states "No keys, keywords or prompts in any of them", so the documentation is inaccurate. Real exposure is narrow. It needs a failed generation where the writer ignored a Must or Avoid keyword. It needs a reader of the user's own logs, meaning the same user or an admin, or a log the user chooses to share. Linux has no log file, and stderr reaching the journal is unverified. No key or credential is exposed and there is no remote vector. This is a doc-versus-behaviour mismatch plus a privacy-hygiene gap, not a vulnerability. The fix is to log only a keyword count or index, use privacy: .private on macOS, or amend the PRIVACY.md row.

#### privacy-claims-2: 'Never leaves your computer' and 'not even your keywords' claims are broader than the data flows
- Kind / severity: vulnerability / low  
- Platforms: macOS, Windows, Linux  
- Locations: PRIVACY.md (short version bullet 'What never leaves your computer: ... history, ratings'); site/pages/index.html:307; site/pages/help.html:117; core/src/composer.rs:656-770 (user_instructions: recent wallpapers, taste hints, keywords); core/src/composer.rs:542 (repair adds a missing Must keyword to the prompt); core/src/composer.rs:1167 (check requires every Must keyword in the prompt)

Three user-facing statements are contradicted by the code. (a) index.html:307 says 'Your keywords, moods, history, taste ... stay on your computer'. Keywords go to the writer in the clear, up to 20 past wallpaper descriptions (history) go with every request, and taste hints derived from ratings (up to 6 liked and 6 disliked phrases) do too. The same page says so two bullets earlier. (b) PRIVACY.md's 'What never leaves' list includes 'history' and 'ratings'. Descriptions from history and ratings-derived phrases are sent; the 'What's sent' section lists them, so the document contradicts itself. (c) help.html:117 says the On this Mac writer is 'private (nothing leaves your Mac, not even your keywords)'. The composer requires every Must keyword's words to appear in the prompt that is then sent to the chosen painter (composer.rs:1167 check, :542 repair), so with a hosted painter the Must and Maybe keywords do leave the Mac. PRIVACY.md qualifies this with 'for that step'; the help page does not. No undocumented data type is involved, but the summary claims overstate the guarantee.

**Attack scenario.** A user reads 'your keywords and taste stay on your computer', or 'not even your keywords' for the Apple writer, and puts sensitive text in keywords (a person's name, a place) while using a hosted painter. That text is sent to OpenAI or Google in the painting prompt.

**Evidence.** Captured in scratch with the core CLI against a local fake OpenAI-compatible server. The painting request body was `{"prompt":"Snowy mountains above a misty harbour ...","size":"2048x1152","n":1,"response_format":"b64_json"}` for Must keyword 'mountains'. The writing request contained the Must and Avoid keywords and the previous wallpaper's description under 'Recent wallpapers'. Headers sent: accept, authorization, host, user-agent, content-type and content-length only. There was no Referer, cookie or Accept-Language. Log: scratchpad/poc/requests.log.

**Recommended fix.** Reword index.html:307 and the 'What never leaves' bullet to the precise list: images, mood names, ratings as such, spending, timings and settings stay; keywords, short descriptions of recent wallpapers, taste phrases and the finished prompt go to the providers you pick. Change help.html:117 to 'the writing step stays on your Mac; if you pick a hosted painter, the finished prompt (which includes your Must keywords) is sent to it'.

**Verifier: downgraded (severity info).** The wording inconsistencies are real, but this is a documentation-accuracy nit rather than a vulnerability. Every data flow is disclosed in detail elsewhere: PRIVACY.md lines 12 and 48-50, and index.html two bullets above line 307. The index.html:307 and PRIVACY.md:13 summary lines (moods, history, taste, ratings) are loose. They mean that the stored records and the raw database stay local. The ratings and history do feed derived hints that are sent, and those are disclosed. The help.html:117 claim "not even your keywords" for the Apple writer is the most literal overstatement. A Must keyword the writer leaves out gets added to the prompt (composer.rs:542), and the check requires every Must keyword in the prompt (composer.rs:1167). That prompt then goes to a hosted painter. PRIVACY.md:11 limits the claim with "for that step", and PRIVACY.md:12 says the finished prompt is sent to paint. No undocumented data type is involved and no attacker is involved. A user would have to put sensitive text into their own keywords and ignore the privacy page. This is a copy fix, not a security finding, and "low" overstates it. Suggested fix: change help.html:117 to "the writing step stays on your Mac, but the finished prompt, which includes your Must keywords, goes to a hosted painter". Also change PRIVACY.md:13 "history, ratings" to "your stored history and ratings, apart from the short descriptions and taste hints listed above".

#### privacy-claims-3: Descriptions of recent wallpapers from every mood are sent to the writer, so other moods' content reaches a provider
- Kind / severity: vulnerability / low  
- Platforms: macOS, Windows, Linux  
- Locations: core/src/store.rs:1020-1026 (recent_summaries has no mood filter); core/src/engine.rs:2400-2407; core/src/composer.rs:752-766; PRIVACY.md ('short descriptions of recent wallpapers, up to 20'; 'Not sent: the mood's name')

`recent_summaries` selects the newest summaries of all successful generations, not just the current mood's, and the composer sends up to 20 of them as 'Recent wallpapers' (MAX_RECENT=20, 400 chars each). Summaries are model-written descriptions of scenes built from that mood's keywords. Composing for one mood therefore discloses the content of every other mood, including a private one, to the hosted writer. The docs never say the sample is global; PRIVACY.md says only that the mood's name is not sent. Taste hints are similarly cross-mood, and help.html says so ('across every mood'). Recent descriptions are not described that way.

**Attack scenario.** A user keeps a 'Family' mood with a child's name and hometown, and a 'Work' mood for office wallpapers on a hosted OpenAI-compatible or Gemini writer. Each 'Work' run sends the Family wallpaper descriptions to that provider (and into its retention window), although the user thinks the mood is separate.

**Evidence.** In scratch I made a Demo wallpaper under mood 'Family' (Must "Ava's 7th birthday party", Maybe "grandma's kitchen in Dallas"). I then switched to mood 'Work' (Must 'mountains') with the writer pointed at my local server. The captured chat-completions request for 'Work' contained 'Recent wallpapers, newest first ... 1. A watercolour illustration of rolling hills ... Ava's 7th birthday party, grandma's kitchen in Dallas ...'. Log: scratchpad/poc/requests.log.

**Recommended fix.** Scope `recent_summaries` to the active mood for the repetition hint (novelty against all moods can stay local via embeddings), or add a per-mood 'keep separate' option. If the global behaviour is intended, say it in PRIVACY.md ('recent wallpapers from all your moods').

**Verifier: downgraded (severity info).** The mechanism is real, but it is not a vulnerability. store.rs:1020-1026 `recent_summaries` filters only on `status = 'ok'`, with no mood filter. engine.rs:2400 passes it straight into `ComposeContext.recent`, so descriptions from every mood go to the hosted writer. But the same provider already received those descriptions' source material. Running the 'Family' mood sent its keywords and its generated prompt to that writer and painter in an earlier request. The 'Work' run adds no new party and no new data class. It only places the Family text in a later request to the same provider the user chose for both moods. A mood in AutoPaper is a keyword preset, not a privacy boundary. No document promises separation, and PRIVACY.md only says the mood's name is not sent. The reviewer's 'Not sent: the mood's name' claim is accurate. The real gap is that PRIVACY.md line 49 says 'recent wallpapers' without saying the sample is global, unlike the taste hints, which the help text describes as 'across every mood'. There is no attacker beyond the provider already trusted with the data. The one narrow case is a user who uses different providers for different moods. This only arises if per-mood provider selection exists, and I did not verify that. In that case Family text would reach the provider that mood was never set to use. That belongs in hardening or documentation: clarify PRIVACY.md, or scope recent summaries by mood for context isolation. I did not rebuild the reviewer's PoC. The unfiltered query and the documentation wording are plain in the code and docs, and the PoC adds nothing to the threat-model argument.

#### privacy-claims-4: macOS sends Locale.current.identifier(.bcp47) with Unicode extension subtags, more than 'language and region'
- Kind / severity: hardening / info  
- Platforms: macOS  
- Locations: apps/macos/AutoPaper/Engine/CoreBridge.swift:81; core/src/composer.rs:933-960 (language_label keeps up to 35 chars of the tag); PRIVACY.md ('your language and region, such as en-US')

The locale string comes from `Locale.current.identifier(.bcp47)`, which includes `-u-` extension keywords for customised regional preferences such as calendar, first weekday, hour cycle, measurement system and temperature unit. `language_label` keeps alphanumerics and dashes up to 35 characters, so the tag is placed in the prompt as e.g. 'English (en-US-u-ca-gregory-fw-mon-hc-h23-ms-metric)'. Unlike a plain language tag, these subtags reflect user customisation and act as a small fingerprint. The doc promises only language and region.

**Attack scenario.** A user who set 24-hour time and Monday-first weeks in Language & Region sends a distinctive preference string with every writing request to a hosted provider.

**Evidence.** I ran `Locale(identifier: "en_US@calendar=gregorian;hours=h23;measure=metric;fw=mon").identifier(.bcp47)` with swift on this Mac. Output: `en-US-u-ca-gregory-fw-mon-hc-h23-ms-metric`. `Locale.current` on this Mac prints plain `en-US`, so the exposure depends on the user's settings.

**Recommended fix.** Build the tag from `Locale.current.language.minimalIdentifier` plus `Locale.current.region?.identifier`, or strip everything from `-u-` and `-x-` in `language_label` or in the host.

#### privacy-claims-5: macOS sandbox description in PRIVACY.md understates the entitlements
- Kind / severity: hardening / info  
- Platforms: macOS  
- Locations: apps/macos/AutoPaper/AutoPaper.entitlements; PRIVACY.md ('Permissions: macOS'); apps/macos/AutoPaper/Views/ConsoleView.swift:106 (fileExporter)

PRIVACY.md says the app 'asks for only outgoing network connections and read access to a file you choose'. The entitlements file grants `com.apple.security.files.user-selected.read-write` (needed for Console's Export JSON), `network.client`, and a `temporary-exception.mach-lookup.global-name` for the two Sparkle installer-launcher services (`-spks`, `-spki`). The write access and the mach-lookup exception are not mentioned. Both are explainable, but the claim is not literally true and SECURITY.md lists the sandbox's entitlements as in scope.

**Attack scenario.** None beyond a misleading permission description. A reviewer relying on the doc would miss that the app can write to user-chosen locations and talk to Sparkle's installer services.

**Evidence.** The entitlements file lists exactly: app-sandbox, files.user-selected.read-write, network.client, mach-lookup exceptions for $(PRODUCT_BUNDLE_IDENTIFIER)-spks and -spki. The only save UI is ConsoleView.swift:106 (.fileExporter) and the only open UI is ProvidersSettings.swift:231 (.fileImporter).

**Recommended fix.** Document the three entitlements, including 'write to a location you pick (Export JSON)' and 'Sparkle installer services'.

#### privacy-claims-6: Documentation gaps and stale statements in PRIVACY.md and NETWORK.md
- Kind / severity: hardening / info  
- Platforms: macOS, Windows, Linux  
- Locations: PRIVACY.md:5 and NETWORK.md:5 ('describes AutoPaper 0.1.0'; UA examples show 0.1.0; code is 0.1.3 per Cargo.toml); apps/macos/AutoPaper/Settings/AboutSettings.swift:56 vs NETWORK.md Sparkle table ('downloaded only when you choose to install'); apps/windows/AutoPaper/Services/OwnLockScreen.cs:51,100-102 vs PRIVACY.md storage table; apps/linux/data/io.github.msitarzewski.AutoPaper.metainfo.xml:45-57 (screenshots on raw.githubusercontent.com)

Four documentation inaccuracies. (1) Both documents are labelled as describing AutoPaper 0.1.0 while the workspace version is 0.1.3 and the text already covers 0.1.3 features (Apple Intelligence writer, appearance sentence). (2) NETWORK.md says the update .zip is downloaded 'only when you choose to install an update', but Settings → About has a 'Download and install updates automatically' toggle (AboutSettings.swift:56) that Sparkle honours. (3) On Windows, OwnLockScreen.Keep writes a copy of the user's current lock-screen picture to `own-lockscreen\lockscreen.<ext>` in the data folder (read through the picturesLibrary capability). The storage table lists only `own-wallpaper\`. (4) The Linux metainfo points software centres at `raw.githubusercontent.com` screenshots (pinned to v0.1.0). GNOME Software or Discover fetch them, not AutoPaper, but the Flatpak egress table does not list that host.

**Attack scenario.** None. These are accuracy gaps, not a leak: nothing is sent that the user did not enable, and the lock-screen copy stays in the app's own data folder.

**Evidence.** Direct reads of the cited lines. No undocumented host is contacted by the app's own code: reqwest is used only in core/src/net.rs, there is no HttpClient or URLSession in the apps, and no Command::new anywhere in core, cli or apps/linux.

**Recommended fix.** Bump the version label or drop it. Add 'unless you turn on automatic download' to the Sparkle row. Add `own-lockscreen\` to the Windows storage table. Mention the software-centre screenshot host, or self-host the screenshots.

**Checked and found sound:**
- Core egress is single-pathed: reqwest is referenced only in core/src/net.rs, and the only other socket is the ComfyUI ws:// TcpStream in net.rs:240-262. There is no TcpStream, UdpSocket, process spawn or lookup_host anywhere else in core/, cli/ or apps/linux/ (grep). The core has no tracing subscriber, telemetry crate or crash reporter (Cargo.lock has no sentry, hf-hub, ureq or curl).
- Native apps make no network requests of their own. macOS has no URLSession, URLRequest, WKWebView or AsyncImage. Windows has no HttpClient, WebView or remote BitmapImage (only ms-appdata or local file URIs, Notifications.cs:139-151). Linux uses only gtk::UriLauncher (desktop.rs:213) for fixed https links and brewbrowser:// via the default handler. Every opened URL is a constant from the documented list (Get a key, Brew Browser, Ollama and ComfyUI downloads, the project site, GitHub issues).
- Clipboard is write-only: NSPasteboard.setString (ConsoleView.swift:206,614), gdk Clipboard.set_text (console.rs:328) and DataPackage Clipboard.SetContent (ConsolePage.xaml.cs:427-430) only copy Console reports. There is no clipboard read on any platform.
- Hardware and system info is limited to the documented User-Agent client string: macOS major.minor (CoreBridge.swift:78-83), Windows major.minor.build (AppModel.cs:331-334), Linux os-release NAME/VERSION_ID (desktop.rs:90-103). The display size is used only for the image-size choice; the aspect ratio-matched size is sent and the doc says 'from your display's shape'. Captured outgoing headers from the real core: accept, authorization (only for the key's own origin), host, user-agent, content-type and content-length. No Referer, cookie, Accept-Language or device ID. The UA was 'AutoPaper/0.1.3 (cli; +https://github.com/msitarzewski/AutoPaper)'.
- Keys are bound to their origin and not sent elsewhere. With the key saved for 127.0.0.1:18080 and the server set to :18091 the Authorization header was absent (secret account is origin-keyed). With the key saved for :18091, a 302 to :18092 (same host, new port) was followed and the second request carried no Authorization (net.rs:156-166). Hosted policy requires https, a domain name from the allow-list and no explicit port (net.rs:35-55). Keys travel only in headers (openai.rs:103, google.rs:142; no `key=` in URLs).
- Prompt contents match the documented list: composer.rs:675-790 sends keywords, Surprise, up to 20 recent summaries, up to 8 'too close' summaries, up to 6+6 taste hints, language, one appearance sentence, fixed instructions, and for echoes the original concept. No file paths, username, hostname, device name, mood name, dates or time of day, spend, or display size appear in the captured request. OpenAI sends store:false and no `user` field (openai.rs:253). The image request carries only prompt, size, n and response_format for compat servers.
- Appearance reporting is exactly one light or dark sentence (composer.rs:757-770). The macOS system-profile claim holds: Info.plist sets no SUEnableSystemProfiling and the feed is a static appcast on github.io with EdDSA enclosures (appcast.xml). The macOS Apple Intelligence writer uses only LanguageModelSession on SystemLanguageModel.default (AppleSystemModel.swift:9,45).
- Update channels match the docs. The Windows .appinstaller (build/windows/0.1.3/AutoPaper.appinstaller) points only at msitarzewski.com with OnLaunch HoursBetweenUpdateChecks=0 and AutomaticBackgroundTask. The Flatpak finish-args are --share=network plus display, GPU and portals, with no filesystem access. The model fetch script pins a Hugging Face revision with SHA-256 checks and --proto =https (fetch-model.sh:20-90). No scripts ship in the app bundles.
- Site claim 'loads nothing from anywhere else' holds. There are no `<script>`, @font-face, CDN, analytics or iframe in site/, and style.css has no url(). External hrefs are plain links only.
- Local storage and key handling: Windows keys use CRED_PERSIST_LOCAL_MACHINE and not PasswordVault, so they do not roam (CredentialStore.cs:11-14,70). macOS keys are plain login-keychain generic passwords with no synchronizable flag. Linux keys go through oo7 and the Secret portal. Windows Log.cs skips AutoPaperException; the Windows file log has no keywords. Filesystem writes by the core are confined to the data dir (imaging.rs:303-320, engine.rs removals). Wallpaper-setting writes outside it go through OS APIs only, which the docs describe: NSWorkspace.setDesktopImageURL, LockScreen.SetImageFileAsync and the GNOME Wallpaper portal.
- Network policy as built: plain http only to localhost, .local, .localhost, loopback, private and link-local addresses, and IPv4-mapped IPv6 is unwrapped (net.rs:57-107). Credentials in the URL are rejected (net.rs:33). Redirects: at most 3, same host, no https-to-http (net.rs:~300). WebSocket is ws:// to local addresses only and is connected directly. The OpenAI-compatible image-URL follow is restricted to the server's own origin (openai_compat.rs:168-182). Responses are size-capped while streaming (net.rs read_body). The proxy applies to non-local hosts only (net.rs client_for).

---

## Follow-up: fixes and re-review (2026-10-10)

Two further agent passes attacked the fixes (the second round of fixes got its own pass; 8 agents, ~1.1M tokens). Results are listed here; the sections above are the first review as written.

**Fixed, with a test that fails without the fix where the behaviour is testable**
- Credentials echoed in provider errors: a process-wide registry of every credential the app used (`net::remember_secret`, filled by the secret store reads and the credential headers) is removed by exact value in `redact`, and error text is scrubbed before it is cut to 200 characters. Covers stored failures, run details, logs and the error returned to the host (`core/tests/engine.rs`, `core/src/net.rs` tests).
- Keywords out of diagnostic logs: the core logs counts, `KeywordNotFollowed`'s message names no keyword (hosts word it from the typed fields), the Mac log writes the error kind only. The CLI still names it on the owner's terminal.
- Data folder owner-only (0700, Unix), only for a folder AutoPaper made or one that already holds its database.
- `.local` / `localhost` names are reached only at local addresses, for HTTP and the ComfyUI WebSocket (including IPv6 literals); a name with some public answers is reached at its local ones only.
- Console: model-written Markdown links are shown as words (Mac). Linux notification bodies escape markup.
- Locale sent to writers: language and region only. Model-written fields capped at 4,000 characters; embedding input capped; response buffer pre-allocation capped; `#![forbid(unsafe_code)]` in the core.
- Docs: PRIVACY.md, NETWORK.md, README.md and the site now match the code (all-moods recent descriptions, sandbox entitlements, Console retention, diagnostics, Windows lock-screen copy, update downloads).
- Release chain: Actions pinned to commit SHAs (Dependabot keeps them current), the Flatpak builder image pinned by digest, bundle attestation from a separate unprivileged job and verified by `publish-flatpak.sh` against the tag's ref and commit, notarization with a keychain profile (password on the command line only as a fallback), Sparkle commit pinned in `release.sh`, Sparkle verifies updates before extracting them (asserted in `release.sh`), NuGet lock files for all three Windows projects (locked mode in CI and releases), `cargo install --locked --rev`, narrower Pages permissions, Control Flow Guard on the Windows core.

**Fixed afterwards (the items first left open)**
- Signed Sparkle feed: `release.sh` signs `appcast.xml` with the update key and verifies it (a tampered copy fails verification, tested); the app sets `SURequireSignedFeed` (and `SUVerifyUpdateBeforeExtraction`), and `release.sh` asserts both in the built app. Installs older than 0.1.4 ignore the signature, so they update normally.
- Image decode memory: images with more than 8 bits a channel are refused and the allocation budget is halved (about 0.8 GB worst case instead of 1.6 GB; a test covers both cases).
- Signing VM: the "Windows 11" VM no longer shares the Mac home (or Downloads, or any other folder). It sees only `~/vm-exchange` as `\\Mac\Exchange`; `scripts/vm-exchange.sh push` copies the source (no `.git`, `.env`, other platforms' apps) there. (`--isolate-vm` would also stop `prlctl exec`, so it stays off.)
- Flatpak repository key: a new key (the old one retired, kept with its revocation certificate) whose passphrase is random and held in the login Keychain; `publish-flatpak.sh` fetches it per publish and presets it into the container's agent, so signing is unattended there. Tested end to end against a throwaway key (signing works with the passphrase, fails without it). No expiry is deliberate: an expired repository key stops every client's updates.

**Still open, with the reason**
- The Keychain items for the Sparkle key and the Flatpak key trust `/usr/bin/security` or `sign_update` without a prompt, so another process of the same user can still use them. Removing that trust is a Keychain Access click (see the hand-off notes); it cannot be done from a script.
- Same-user access to keys and data on Windows and Linux (full-trust MSIX, Secret Service): the platform baseline; documented as out of scope.
- Actions cache restore keys on tag builds: needs repository write access to poison.

**Historical question from the last pass**: `pages.yml` no longer has `configure-pages` (its output was unused);
watch the first Pages run after the next push to `main`. Publication was subsequently verified: the final 0.2.0
[Pages run](https://github.com/msitarzewski/AutoPaper/actions/runs/39381286459) succeeded and the live site was checked
on October 21.

## Follow-up: 0.2.0 release checks (2026-10-20/21)

Scope: the shipped `v0.2.0` application source is immutable commit `2f8b79e`. `df7cc19` corrects a stale Windows
test assertion only; final release metadata is `f8f668b`. This addendum records the release checks and dependency
assessment from the [release task](../memory-bank/tasks/2026-10/261018_moss-image-workflow.md), not a repeat of the
full October 10 review. The open Keychain trust, same-user access and Actions cache limitations above remain open.

**Dependency applicability.** OSV scanned `Cargo.lock` and the Windows application lockfile. Four maintenance
notices remain for `paste`, `zbus_names`, `zvariant` and `zvariant_derive`. Three `zip` 8.6.0 extraction advisories
(RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369) were assessed against the locked dependency source:
`zip` is transitive through Candle 0.11, whose optional NumPy/Pickle readers read named archive entries and do not
call `ZipArchive::extract` or `ZipStreamReader::extract`. AutoPaper's `core/src/embed.rs` loads its bundled
safetensors model instead of those archive formats. The affected extraction APIs are not reachable from the
app's model-loading path. This is an applicability assessment, not a clean dependency scan. Track Candle's
eventual zip 9 migration and maintained D-Bus replacements separately; reassess if model-loading paths change.
Details: [dependency scan](../memory-bank/tasks/2026-10/261018_moss-image-workflow.md#release-dependency-scan-2026-10-20).

**Release-chain checks.** macOS production signing, notarization/stapling, bundled model hashes and Sparkle
archive/feed signatures passed. Windows production signatures were timestamped and verified on both architecture
packages and the MSIX bundle; both packaged core DLLs and all six bundled model files matched their checked hashes.
Both Flatpak builds and attestations passed, and a fresh client verified the published repository summary and both
architecture refs using the public key. All nine GitHub release asset digests matched; public update feeds and the
site were verified live. New versioned winget manifests were generated, with no registry submission.

[Final CI](https://github.com/msitarzewski/AutoPaper/actions/runs/39381286539) passed all seven jobs at `f8f668b`.
Local suites passed 761 tests (515 core/CLI, 123 Mac, 37 Linux, 86 Windows; 12 existing core tests ignored).
GUI checks used isolated guest profiles and preserved host data. These results do not establish broad hardware or
full screen-reader coverage. Linux's installed-model popup passed; other GUI/error-state and selection-persistence
checks were not exercised, and eight nonfatal GTK startup assertions remain unexplained. See
[package verification](../memory-bank/tasks/2026-10/261018_moss-image-workflow.md#final-package-verification-2026-10-20)
for the exact evidence and remaining limits.
