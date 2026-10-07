# AutoPaper: Rust core, bindings & Linux toolchain research

Research date: 2026-10-05. Versions come from the crates.io API (`https://crates.io/api/v1/crates/<name>`), GitHub releases/tags (via `gh api`), the Hugging Face API, Flathub (`flatpak remote-ls/remote-info` run in the Scratch VM), Fedora mdapi and gnome-build-meta. Items marked **VERIFIED** were built or run during this research. Everything else comes from the docs or source files cited.

---

## 0. Pinned versions (TL;DR)

| Area | Pin | Why |
|---|---|---|
| UniFFI | `uniffi = "=0.31.2"` (all crates + the bindgen binary crate) | Newest version on the 0.31 line. `uniffi-bindgen-cs` has no 0.32 support yet (issues #176/#183 are open). |
| C# generator | `uniffi-bindgen-cs` tag `v0.11.0+v0.31.0` (2026-06-23) | Targets uniffi-rs 0.31.0. Patch releases in 0.31.x keep the same metadata contract, and 0.31.1/0.31.2 only fixed Kotlin/Swift. |
| Swift packaging | in-repo `uniffi-bindgen-swift` binary (from `uniffi` with the `cli` feature) + `xcodebuild -create-xcframework` | Same version as the core via Cargo.lock. cargo-swift 0.11.1 (uniffi 0.31.1) is optional. |
| gtk4-rs | `gtk4 = { version = "0.11.5", features = ["gnome_50"] }` (implies `v4_22`, `gio/v2_88`) | Ubuntu 26.04 = GTK 4.22.4, Fedora 44 = 4.22.5, GNOME runtime 50 = 4.22.5, GNOME runtime 51 = 4.24.0 |
| libadwaita-rs | `libadwaita = { version = "0.9.2", features = ["v1_9"] }` | Ubuntu 26.04 = 1.9.1, Fedora 44 = 1.9.4, runtime 50 = 1.9.4, runtime 51 = 1.10.0 |
| glib/gio | 0.22.10 (comes with gtk4 0.11) | |
| Portals | `ashpd = "0.13.13"` (features: `wallpaper`, `background`, `settings`, `notification` optional; default `tokio`) | |
| Secrets (Linux) | `oo7 = "0.6.0"` (stable; 0.7.0-beta exists, MSRV 1.95) | Pure Rust. Uses the D-Bus Secret Service on the host and an encrypted file plus the Secret portal in a sandbox. |
| Tray (KDE only) | `ksni = "0.3.6"` (Unlicense) | Optional, for desktops other than GNOME |
| Embeddings | `candle-core/candle-nn/candle-transformers = "0.11.0"` + `tokenizers = "0.22"` (`default-features=false, features=["onig"]`, which matches candle-core's own dependency) | Pure-Rust inference. Model: `BAAI/bge-small-en-v1.5` (MIT, 384-d) |
| DB | `rusqlite = { version = "0.40.2", features = ["bundled"] }` (SQLite 3.53.4 via libsqlite3-sys 0.38.2) | |
| HTTP | `reqwest = "0.13.5"` (rustls is now the default TLS; see section 3) | |
| Runtime | `tokio = "1.53.2"` | |
| Misc | serde 1.0.229, serde_json 1.0.151, thiserror 2.0.21, tracing 0.1.44, tracing-subscriber 0.3.23, chrono 0.4.45 (or time 0.3.55), image 0.25.10, image_hasher 3.1.1 | |
| Rust | stable 1.99.0 (2026-09-28). Flatpak rust-stable//26.08 ships 1.98.0 | gtk4 MSRV 1.92, oo7 MSRV 1.92, tract MSRV 1.91, image MSRV 1.88 |

---

## 1. UniFFI

### Versions
- Latest: **0.32.2** (2026-09-23). The other releases in this range: 0.32.1 (09-09), 0.32.0 (06-30), 0.31.2 (06-17), 0.31.1 (04-13), 0.31.0 (01-14). Source: crates.io versions API; changelog https://github.com/mozilla/uniffi-rs/blob/main/CHANGELOG.md
- 0.32 breaks external generators: metadata contract bump, `ForeignBytes` for `&[u8]`, `GlobalConfig`, and `--config` semantics. `with_foreign` is deprecated in favour of `#[uniffi::export(foreign)]` / `#[uniffi::export(rust, foreign)]`.
- **Decision: pin `=0.31.2`.** Move to 0.32 only after a `uniffi-bindgen-cs` release that supports it ships (PR/issue #183 "Update for 0.32 uniffi", #176).

### Proc-macros vs UDL
- Use **proc-macros with library mode**. `uniffi::setup_scaffolding!()`, `#[derive(uniffi::Record|Enum|Error|Object)]`, `#[uniffi::export]`. There is no UDL to keep in sync. `uniffi-bindgen-swift` "always inputs a library path and runs in library mode", and `uniffi-bindgen-cs --library` supports it too.
  - https://mozilla.github.io/uniffi-rs/latest/proc_macro/index.html
  - https://github.com/mozilla/uniffi-rs/blob/v0.31.2/docs/manual/src/swift/uniffi-bindgen-swift.md

### Async
- Exported `async fn` maps to Swift `async throws`, C# `Task<T>`, and Kotlin `suspend`. The foreign side drives the future, so no Rust event loop is required. To call tokio-based code such as reqwest from an exported async fn, add the `uniffi` feature `tokio` and annotate with `#[uniffi::export(async_runtime = "tokio")]` (feature list in `uniffi/Cargo.toml` at v0.31.2).
- Async methods on foreign traits are supported (`#[async_trait::async_trait]` on the trait).
- UniFFI has no built-in cancellation. Expose an explicit `cancel()` or a token instead. https://mozilla.github.io/uniffi-rs/latest/futures.html

### Foreign traits (host implements SecretStore, WallpaperSetter, …)
- On 0.31 use `#[uniffi::export(with_foreign)] pub trait X: Send + Sync { ... }`. Rust receives `Arc<dyn X>`, and all parameters are by value. (On 0.32 this attribute becomes `#[uniffi::export(foreign)]`.)
- Every method should return `Result<_, E>` where `E` is a `uniffi::Error`. Implement `From<uniffi::UnexpectedUniFFICallbackError> for E`; without it, unexpected foreign exceptions panic. https://github.com/mozilla/uniffi-rs/blob/v0.31.2/docs/manual/src/foreign_traits.md
- Avoid Rust↔foreign reference cycles. UniFFI does not break them.

### Errors
- `#[derive(Debug, thiserror::Error, uniffi::Error)] enum CoreError { Variant { field: T } }` exposes the fields. `#[uniffi(flat_error)]` exposes only variant names plus the `to_string()` message. Swift sees an `Error` enum, and C# sees an exception class hierarchy (`CoreException.Variant`). https://mozilla.github.io/uniffi-rs/latest/proc_macro/errors.html

### VERIFIED on this Mac (Xcode 27.2, Swift 6.4, rustup stable 1.96)
A toy workspace (`scratchpad/xcf-test`) exported the following:
- a `with_foreign` sync trait `SecretStore`
- an async `with_foreign` trait `WallpaperSetter`
- an `Object` with an `async fn tick()` running on `async_runtime = "tokio"`
- a `uniffi::Error` enum

It was built for aarch64 and x86_64, packaged with lipo and an XCFramework, wrapped in a SwiftPM package, and run. The output was `swift setWallpaper(/tmp/x.jpg)` / `tick -> ok:k123`.

**Findings:**
1. The generated Swift **does not compile in Swift 6 language mode** when the trait has async methods: "passing closure as a 'sending' parameter risks causing data races" in `uniffiTraitInterfaceCallAsync`. Compile the *bindings target* with `swiftSettings: [.swiftLanguageMode(.v5)]`. The app target can stay on Swift 6.
2. Local environment quirk: on this macOS 27.2 + Xcode 27.2 beta host, **setting `MACOSX_DEPLOYMENT_TARGET` for a `--release` build makes host proc-macro dylibs fail to load** (`dlopen ... mis-aligned LINKEDIT string pool`, which then shows up as `can't find crate for thiserror_impl`). This happens with both rustc 1.96 (rustup) and 1.98.1 (Homebrew). Workaround: leave the variable unset when building the staticlib. Xcode applies the app's deployment target at final link. Rust's defaults are arm64 11.0 and x86_64 10.12.
3. On this Mac `cargo` on PATH is **Homebrew rust 1.98.1**, which has no x86_64-apple-darwin std. The rustup toolchain (`~/.rustup/toolchains/stable-aarch64-apple-darwin/bin`, 1.96.0) does have both targets. Put rustup's toolchain first in PATH, or `brew unlink rust`, before building the universal library. `rustup update stable` is also recommended because 1.99 is current.

### Exact XCFramework / Swift package commands (verified)
Workspace layout: `crates/autopaper-core` (`[lib] crate-type = ["lib", "staticlib", "cdylib"]`, `name = "autopaper_core"`) and `crates/uniffi-bindgen`, which depends on `uniffi = { version = "=0.31.2", features = ["cli"] }` and has two bins:
```rust
// src/uniffi-bindgen.rs
fn main() { uniffi::uniffi_bindgen_main() }
// src/uniffi-bindgen-swift.rs
fn main() { uniffi::uniffi_bindgen_swift() }
```
Build (macOS):
```sh
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"   # this Mac only (see finding 3)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build -p autopaper-core --release --target aarch64-apple-darwin
cargo build -p autopaper-core --release --target x86_64-apple-darwin
mkdir -p target/macos-universal/release
lipo -create -output target/macos-universal/release/libautopaper_core.a \
  target/aarch64-apple-darwin/release/libautopaper_core.a \
  target/x86_64-apple-darwin/release/libautopaper_core.a

LIB=target/aarch64-apple-darwin/release/libautopaper_core.a   # metadata is arch-independent
rm -rf build/swift && mkdir -p build/swift/include/autopaper_coreFFI build/swift/AutopaperCore/Sources/AutopaperCore
cargo run -p uniffi-bindgen --bin uniffi-bindgen-swift -- $LIB build/swift/AutopaperCore/Sources/AutopaperCore --swift-sources
cargo run -p uniffi-bindgen --bin uniffi-bindgen-swift -- $LIB build/swift/include/autopaper_coreFFI \
  --headers --modulemap --module-name autopaper_coreFFI --modulemap-filename module.modulemap
xcodebuild -create-xcframework \
  -library target/macos-universal/release/libautopaper_core.a \
  -headers build/swift/include \
  -output build/swift/AutopaperCore/AutopaperCoreFFI.xcframework
```
- Do **not** pass `--xcframework` to uniffi-bindgen-swift when using `-library`. That flag emits `framework module …`, which is meant for `.framework` bundles. The headers go in a module-named subfolder (`Headers/autopaper_coreFFI/`), which avoids the "multiple module.modulemap" collision. cargo-swift does the same.
- `build/swift/AutopaperCore/Package.swift`:
```swift
// swift-tools-version:6.0
import PackageDescription
let package = Package(
  name: "AutopaperCore", platforms: [.macOS(.v14)],
  products: [.library(name: "AutopaperCore", targets: ["AutopaperCore"])],
  targets: [
    .binaryTarget(name: "AutopaperCoreFFI", path: "AutopaperCoreFFI.xcframework"),
    .target(name: "AutopaperCore", dependencies: ["AutopaperCoreFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)]),   // see finding 1
  ])
```
- If Rust deps need system frameworks (e.g. `Security`/`SystemConfiguration` for rustls-platform-verifier/system-proxy), add them as `linkerSettings: [.linkedFramework("Security"), .linkedFramework("SystemConfiguration")]` on the target, or pass `--link-frameworks` when generating the modulemap.
- Alternative: `cargo install cargo-swift@0.11.1` then `cargo swift package -p macos` (cargo-swift 0.11.x targets uniffi 0.31; README table). Source: https://github.com/antoniusnaumann/cargo-swift. The manual commands above give more control and are already verified.

---

## 2. C# bindings (WinUI 3)

- **`uniffi-bindgen-cs` v0.11.0+v0.31.0** (published 2026-06-23; `main` == tag, no commits since). Requires **Rust ≥ 1.88** to install. Targets **.NET 8+** (`AllowUnsafeBlocks`). On `NET8_0_OR_GREATER` it uses source-generated `[LibraryImport]`. License MPL-2.0. https://github.com/NordSecurity/uniffi-bindgen-cs , releases: https://github.com/NordSecurity/uniffi-bindgen-cs/releases
- Compatibility table: v0.11.0 ↔ uniffi 0.31.0, v0.10.0 ↔ 0.29.4, v0.9.x ↔ 0.28.3.
- Install: `cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.11.0+v0.31.0`
- Generate (library mode): `uniffi-bindgen-cs --library target\x86_64-pc-windows-msvc\release\autopaper_core.dll --out-dir <proj>\Generated [--config uniffi.toml] [--no-format]`. Formatting uses `csharpier` if it is installed.
- `uniffi.toml` `[bindings.csharp]` keys: `cdylib_name`, `namespace` (default `uniffi.<crate>`), `global_methods_class_name`, `access_modifier` (default `internal`, so set it to `public` if the bindings live in a separate class library), `null_string_to_empty`, `custom_types`, `rename`, `external_packages`, `omit_checksums`. https://github.com/NordSecurity/uniffi-bindgen-cs/blob/main/docs/CONFIGURATION.md
- Feature coverage, based on the v0.11.0 notes and its test suite:
  - async functions and methods (`TestFutures.cs`)
  - callback interfaces and foreign traits (`TestCallbacks.cs`, `TestTraits.cs`)
  - async callback interfaces (`TestAsyncCallbackInterface.cs`, the issue-165 fixture is in the tag)
  - error enums, including object fields
  - records are PascalCase
  - trait methods on records and enums
  - the release fixed several async foreign-future races
- Known limitations and open issues:
  - strings, byte arrays and lists are limited to 2^31 bytes
  - #175: callback vtable codegen crashes under full NativeAOT (reported on iOS/Mac Catalyst). Test before enabling `PublishAot` on Windows.
  - #40: external types are only partially supported
  - #179: a generated handle ctor conflicts with a proc-macro constructor taking `u64`
  - #148: a method named `Finalize` conflicts with the destructor
  - not on crates.io (#118)
- **VERIFIED (docker `mcr.microsoft.com/dotnet/sdk:10.0`, linux-arm64, rustc 1.99, .NET SDK 10.0.401).** The same toy crate on `uniffi =0.31.2` was built as a `.so`. `uniffi-bindgen-cs v0.11.0+v0.31.0 --library … --no-format` generated `autopaper_core.cs` (91 KB), and a console app built with 0 warnings. A C# `SecretStore` (sync) and `WallpaperSetter` (`async Task SetWallpaper`) were passed to Rust, and `await agent.Tick()` printed `cs SetWallpaper(/tmp/x.jpg)` / `tick -> ok:k123`. There were no checksum or contract errors, which confirms the 0.31.2 scaffolding works with this generator. The generated C# names are `SecretStore` (no `I` prefix), `Get`/`Set`/`SetWallpaper`/`Tick`, and the namespace is `uniffi.autopaper_core`. (Test harness: `scratchpad/cs-test/run.sh`.)
- **Implication for UniFFI pinning:** stay on 0.31.x. Before upgrading uniffi, run `gh api repos/NordSecurity/uniffi-bindgen-cs/releases --jq '.[0].tag_name'`.
- Alternatives if it lags:
  - `csbindgen` 1.9.8 (Cysharp, MIT) generates `[DllImport]` for a hand-written `extern "C"` surface. You would hand-roll async, callbacks and errors.
  - Interoptopus 0.16.5 (C# backend crate `interoptopus_backend_csharp` 0.14.25 stable / 0.15.0-alpha.24).
  - Hand-written C ABI + P/Invoke.

  All of these lose UniFFI's shared object/async/trait model. Prefer keeping the FFI surface small and synchronous where possible, so a fallback stays cheap.

---

## 3. Rust crates (latest stable on crates.io, 2026-10-05)

| Crate | Version (date) | License | Notes |
|---|---|---|---|
| rusqlite | 0.40.2 (2026-08-08) | MIT | `bundled` → libsqlite3-sys 0.38.2/`bundled` + `modern_sqlite`. Compiles SQLite **3.53.4** with `cc` (MSVC/clang/gcc), so there is no system dependency. |
| reqwest | 0.13.5 (2026-09-08) | MIT/Apache | 0.13 makes **rustls the default TLS**. The crypto provider defaults to **aws-lc-rs**, and certificate checks use `rustls-platform-verifier` (OS trust store). `rustls-tls` was renamed to `rustls`, and `rustls-no-provider` lets you supply ring yourself. https://docs.rs/crate/reqwest/0.13.5/features , https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md |
| tokio | 1.53.2 (2026-10-03) | MIT | |
| serde / serde_json | 1.0.229 / 1.0.151 | MIT/Apache | |
| image | 0.25.10 (2026-03-10) | MIT/Apache | Use `default-features=false, features=["jpeg","png","webp","rayon"]`. Avoid `avif-native`, which needs the dav1d system lib. |
| image_hasher | 3.1.1 (2026-02-21) | MIT/Apache | Maintained fork of `img_hash`. `img_hash` 3.2.0 has had no release since 2021. Requires image `>=0.25,<0.26`. |
| thiserror | 2.0.21 | MIT/Apache | |
| tracing / tracing-subscriber | 0.1.44 / 0.3.23 | MIT | |
| chrono / time | 0.4.45 / 0.3.55 | MIT/Apache | Both maintained. Pass timestamps over FFI as `SystemTime` (UniFFI `timestamp`, C# `DateTime`). |
| hf-hub | 1.0.0 (2026-07-10) | Apache-2.0 | Optional model downloader. A plain reqwest GET plus SHA-256 check is enough. |
| keyring | 4.2.0 | MIT/Apache | Cross-platform alternative, but the host apps implement SecretStore natively, so it is not needed. |

**TLS build requirements on Windows** (both crypto providers):
- `aws-lc-rs`:
  - x86_64-pc-windows-msvc needs MSVC plus NASM. Prebuilt NASM objects are used as a fallback (env `AWS_LC_SYS_PREBUILT_NASM=1`).
  - aarch64-pc-windows-msvc needs **clang-cl**.

  https://aws.github.io/aws-lc-rs/requirements/windows.html
- `ring` also needs clang on aarch64-pc-windows-msvc.

→ **Install the VS Build Tools component "C++ Clang Compiler for Windows" + "MSVC ARM64 build tools"** on the Windows build machine. With that in place, keep reqwest's default (`rustls` + aws-lc-rs) and set `AWS_LC_SYS_PREBUILT_NASM=1` for x64 if NASM is absent. macOS and Linux need only a C compiler.

### Local sentence embeddings (no system deps)

| Option | Version | Pure Rust? | Windows ARM64 | Notes |
|---|---|---|---|---|
| **candle** (HF) | 0.11.0 (2026-06-26) | Yes. CPU matmul uses the `gemm` crate. candle-core pulls `tokenizers ^0.22` with `onig` (C, built by `cc`). | Expected OK (Rust intrinsics; onig is plain C). Verify in CI. | Reads `model.safetensors` + `tokenizer.json` straight from HF. `candle_transformers::models::bert` exists, with a BERT example in candle-examples. Optional Metal/Accelerate on macOS. https://github.com/huggingface/candle |
| tract (Sonos) | 0.23.8 (2026-09-21) | Yes, with hand-written asm kernels built via `cc`. | Needs **clang** for aarch64-msvc (build.rs forces `compiler("clang")`). x64-msvc uses ml64.exe. | ONNX in, MSRV 1.91, smaller dependency tree. https://github.com/sonos/tract |
| ort (pyke) | 2.0.0-rc.13 (still RC) | No: native ONNX Runtime. | Prebuilt download | Default features `download-binaries` + `copy-dylibs`. You ship onnxruntime per target, which breaks the "no system/native deps" goal. |
| fastembed-rs | 7.1.0 | No by default. | n/a | Default backend is **ort** (`ort-download-binaries-native-tls`). Candle is only used for qwen3/nomic-v2-moe models. |
| model2vec-rs | 0.3.0 | Yes. | Yes | Static embeddings (potion-base-8M, MIT, 30 MB). Very fast, lower quality. A good fallback for low-end hardware. |

**Recommendation:** candle 0.11.0 (`candle-core`, `candle-nn`, `candle-transformers`), CPU only by default. Use `tokenizers = { version = "0.22", default-features = false, features = ["onig"] }`, which is the same tokenizers line candle-core already depends on. Use tract as the fallback if candle compile time or binary size is a problem; it uses the same model's `onnx/model.onnx`.

**Model:** `BAAI/bge-small-en-v1.5`, MIT license
- BERT, 12 layers, hidden size 384, **384-d** output, 512 max tokens
- MTEB avg 62.17, retrieval 51.68 (model card)
- CLS pooling + L2 normalise. The query instruction is optional for v1.5.
- Pin revision `5c38ec7c405ec4b44b94cc5a9bb96e735b38267a`:
  - `https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/5c38ec7c405ec4b44b94cc5a9bb96e735b38267a/model.safetensors`: 133,466,304 bytes, sha256 `3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad`
  - `.../tokenizer.json`: 711,396 bytes
  - `.../config.json`: 743 bytes
- Download on first use into the app data dir, verify the SHA-256, and allow an offline import. Optionally convert to f16 safetensors yourself (about 67 MB) and host it.
- Smaller alternative: `sentence-transformers/all-MiniLM-L6-v2`
  - Apache-2.0, 6 layers, 384-d, mean pooling
  - model.safetensors 90,868,376 bytes, sha256 `53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db`
  - revision `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`

Sources: HF API `https://huggingface.co/api/models/<id>?blobs=true` and `/tree/main`; https://huggingface.co/BAAI/bge-small-en-v1.5

---

## 4. Linux desktop stack

### Platform versions → feature flags
| Target | GTK | libadwaita | GLib | Source |
|---|---|---|---|---|
| Ubuntu 26.04.1 LTS | 4.22.4 | 1.9.1 | 2.88.0 | apt in the VM + `ubuntu:26.04` docker |
| Fedora 44 (current stable) | 4.22.5 | 1.9.4 | 2.88.3 | https://mdapi.fedoraproject.org/f44/pkg/libadwaita |
| Fedora rawhide (→ F45/46) | 4.24.1 | 1.10.0 | 2.90.0 | mdapi rawhide |
| GNOME Flatpak runtime 50 | 4.22.5 | 1.9.4 | — | gnome-build-meta `gnome-50` elements/sdk/*.bst |
| **GNOME Flatpak runtime 51** (current, built 2026-10-04) | 4.24.0 | 1.10.0 | — | gnome-build-meta `gnome-51` |

→ Use gtk4 `gnome_50` (= `v4_22` + `gio/v2_88`) and libadwaita `v1_9`. One binary then runs natively on Ubuntu 26.04/Fedora 44 and in Flatpak runtime 50 or 51. Only raise to `v1_10`/`v4_24` if the app becomes Flatpak-only on runtime 51. libadwaita-rs 0.9.2 supports up to `v1_10`, and gtk4 0.11.5 supports up to `v4_24`/`gnome_50`.

### Widget availability (libadwaita docs "Available since")
| Widget | Since |
|---|---|
| AdwPasswordEntryRow | 1.2 |
| AdwSpinRow | 1.4 |
| AdwSwitchRow | 1.4 |
| AdwNavigationSplitView | 1.4 |
| AdwPreferencesDialog | 1.5 |
| AdwAboutDialog | 1.5 |
| AdwButtonRow | 1.6 |
| AdwSpinner | 1.6 |
| AdwBottomSheet | 1.6 |
| AdwToggleGroup | 1.7 |
| AdwWrapBox | 1.7 |
| AdwInlineViewSwitcher | 1.7 |
| AdwShortcutsDialog | 1.8 |
| AdwSidebar | 1.9 |

All of these are available with `v1_9` (Ubuntu 26.04 has 1.9.1). https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1-latest/ (e.g. `class.ToggleGroup.html`). **VERIFIED:** an AdwToggleGroup smoke app built and rendered in the VM.

### Portals via ashpd 0.13.13 (2026-07-17, MIT, MSRV 1.87)
- **Wallpaper** (`org.freedesktop.portal.Wallpaper` v1). `SetWallpaperURI` says "file: URIs are explicitly not supported". For local files use `SetWallpaperFile`, i.e. `WallpaperRequest::default().set_on(SetOn::Background|Lockscreen|Both).show_preview(false).build_file(&file.as_fd())`.
  - Permission: if `show-preview=false` and the permission-store entry is not YES, the frontend shows a one-time "Allow … to set backgrounds?" access dialog and stores the answer (xdg-desktop-portal `desktop-portal/wallpaper.c`).
  - GNOME backend (xdg-desktop-portal-gnome `src/wallpaper.c`) **ignores `set-on`**. It copies the image to `~/.config/background` and sets `org.gnome.desktop.background picture-uri` **and** `picture-uri-dark`, with zoom. GNOME's lock screen uses the desktop background.
  - KDE backend honours background/lockscreen/both and **defaults show-preview to true**, so always pass `false`.
  - https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Wallpaper.html , https://docs.rs/ashpd/0.13.13/ashpd/desktop/wallpaper/index.html
- **Background** (v2): `Background::request().reason(..).auto_start(true).command(&["autopaper","--background"]).dbus_activatable(false).send().await?.response()?`. `set_status()` takes 96 characters max.
  - For **host (non-Flatpak) apps** the portal grants background automatically. Autostart needs a detected app ID, so call `ashpd::register_host_app(app_id)` first. `SetStatus` is rejected for host apps (xdg-desktop-portal `desktop-portal/background.c`).
  - https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html
- **Settings**: `Settings::new().await?.color_scheme()/accent_color()/contrast()/reduced_motion()` plus `receive_*_changed()` streams. libadwaita's `AdwStyleManager` already follows color-scheme and accent automatically, so ashpd is only needed for non-UI logic (e.g. picking dark-variant wallpapers). https://docs.rs/ashpd/0.13.13/ashpd/desktop/settings/index.html
- **Notifications**: use `gio::Application::send_notification(id, &gio::Notification)` (GNotification). GLib routes it through the notification portal in Flatpak, and through the shell/fdo backends on the host. It needs a `.desktop` file matching the app ID. ashpd's `notification` feature is the low-level alternative.
- **Host-app registration**: call `ashpd::register_host_app(AppID)` early when running unsandboxed (xdg-desktop-portal ≥ 1.19 Registry; Ubuntu 26.04 has 1.21.1), so permissions are keyed by app ID.
- Runtime: ashpd's default feature is `tokio`. Run one tokio runtime and bridge results to GTK with `glib::spawn_future_local`/`MainContext` channels. oo7 and ashpd can share that runtime.

### Secrets: oo7 0.6.0 (2026-02-21, MIT) vs libsecret
- oo7 is pure Rust (`native_crypto` by default, no OpenSSL) and async. `oo7::Keyring::new()` picks the backend automatically:
  - **Host:** D-Bus Secret Service (gnome-keyring, KDE ksecretd, KeePassXC).
  - **Flatpak:** an encrypted libsecret-compatible file in the sandbox, keyed by the **Secret portal**.

  It also includes a host→sandbox migration API. https://github.com/linux-credentials/oo7/blob/main/client/README.md
- The Secret portal backend is provided by gnome-keyring (`gnome-keyring.portal`, `UseIn=gnome`). The KDE story is still changing (ksecretd/oo7 migration, reported Flatpak portal issues; https://planet.kde.org/marco-martin-2026-01-30-kwallet-secretservice-oo7-the-story-so-far/). Test on Plasma.
- libsecret (C) via the `libsecret` crate needs `libsecret-1-dev` and also uses the portal file backend in Flatpak. The only reason to choose it is a C/GObject stack. **Use oo7.**

### Tray (KDE) and GNOME HIG
- `ksni` 0.3.6 (2026-07-15, Unlicense, maintained). StatusNotifierItem over zbus, default `tokio`, `blocking` feature available. It works on KDE. On GNOME it works only with an AppIndicator extension. Ubuntu enables `ubuntu-appindicators` by default; vanilla GNOME/Fedora do not.
  - Flatpak needs `--talk-name=org.kde.StatusNotifierWatcher`.
- GNOME guidance: there are no tray icons (removed in 3.26). Use the **Background portal + notifications**. GNOME 44+ lists portal-registered apps under "Background Apps" in Quick Settings. https://discourse.gnome.org/t/system-tray-icons-in-gtk4/22615 , https://developer.gnome.org/hig/patterns/feedback/notifications.html
- → On GNOME, show no tray and run windowless in the background via the portal, with GNotification for events. On KDE/others, an optional ksni tray, off by default on GNOME.

---

## 5. Flatpak

- Runtimes on Flathub (queried 2026-10-05 from the VM):

  | Runtime | Built from | Commit date | Base |
  |---|---|---|---|
  | `org.gnome.Platform//51` | `51.0-9-g0b400781f` | 2026-10-04 | freedesktop-sdk 26.08 |
  | `org.gnome.Platform//50` | `50.5-1` | 2026-10-03 | freedesktop 25.08 |

  Runtime 49 also exists.
- Rust extension: `org.freedesktop.Sdk.Extension.rust-stable//26.08` for GNOME 51 (ships rust 1.98.0), or `//25.08` for GNOME 50.
- `flatpak-cargo-generator.py` (https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo, last updated 2026-09-21): `python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json`, then build offline with `CARGO_HOME=/run/build/<module>/cargo`, `cargo --offline fetch`, `cargo build --offline --release`, and `append-path: /usr/lib/sdk/rust-stable/bin`.
- Manifest sketch:
```yaml
id: dev.autopaper.AutoPaper
runtime: org.gnome.Platform
runtime-version: '51'        # or '50' + rust-stable//25.08
sdk: org.gnome.Sdk
sdk-extensions: [org.freedesktop.Sdk.Extension.rust-stable]
command: autopaper
finish-args:
  - --share=network          # wallpaper sources / APIs / model download
  - --share=ipc
  - --socket=wayland
  - --socket=fallback-x11
  - --device=dri
  # Portals (Wallpaper, Background, Settings, Notification, Secret, OpenURI, FileChooser) need NO extra permissions.
  # Optional, KDE tray only:  - --talk-name=org.kde.StatusNotifierWatcher
```
- Do **not** add `--talk-name=org.freedesktop.secrets` (oo7 uses the Secret portal) or any `--filesystem` (use the portal fd for wallpapers and the FileChooser for imports).
- The VM has flatpak 1.16.6 and flatpak-builder 1.4.8, with the system-wide flathub remote added. Neither runtime is installed yet (about 400 MB download each). Install with `flatpak install -y --system flathub org.gnome.Sdk//51 org.gnome.Platform//51 org.freedesktop.Sdk.Extension.rust-stable//26.08`.

---

## 6. Linux build environment (Part 2)

### Parallels VM "Scratch" (used for all Linux work; left running)
- Ubuntu **26.04.1 LTS** (resolute), **aarch64**, kernel 7.0.0-34-generic, 8 vCPU, 16 GB RAM, 55 GB free. Parallels Tools 27.0.2.
- Desktop: **GNOME Shell 50.1** (`--mode=ubuntu`), **Wayland**, user **`michael`** (uid 1000), auto-logged-in on seat0/tty2.
- `prlctl exec "Scratch" <cmd>` runs as **root**. Arguments get re-joined into a single shell string, so complex scripts break. Use one of these patterns:
  ```sh
  B=$(base64 < script.sh | tr -d '\n'); prlctl exec "Scratch" "echo $B | base64 -d | bash"
  prlctl exec "Scratch" "sudo -u michael -i bash -lc 'cd ~/proj && cargo build'"     # as desktop user (login shell → ~/.cargo/bin on PATH)
  # GUI apps as the desktop user (Wayland session):
  prlctl exec "Scratch" "sudo -u michael env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus setsid <app> >/tmp/app.log 2>&1 </dev/null &"
  ```
  For long jobs use `setsid nohup … &` inside the guest. When the exec channel drops, a foreground job dies with `PRL_ERR_IO_STOPPED`.
- Screenshots: **`prlctl capture "Scratch" --file shot.png`** works from the Mac (1024×768). `gnome-screenshot` inside the guest fails on Wayland ("Unable to capture a screenshot of any window").
- Versions present or installed:
  - GTK 4.22.4, libadwaita 1.9.1, GLib 2.88.0 (`pkg-config --modversion gtk4 libadwaita-1 glib-2.0`)
  - xdg-desktop-portal 1.21.1, xdg-desktop-portal-gnome 50.0, xdg-desktop-portal-gtk 1.15.3
  - gnome-keyring 50.0, libsecret 0.21.7, orca 50.2
  - build-essential, pkgconf 2.5.1, git 2.53, curl 8.18, desktop-file-utils, appstream 1.1.2 (`appstreamcli`)
- **Newly installed by me** (apt, `--no-install-recommends`, log at `/root/autopaper-install.log`): `libgtk-4-dev 4.22.4+ds-0ubuntu0.1`, `libadwaita-1-dev 1.9.1-0ubuntu0.1`, `flatpak 1.16.6-1`, `flatpak-builder 1.4.8-1`, plus their dependencies (libgraphene-1.0-dev, libvulkan-dev, libappstream-dev, ostree, libostree-1-1, libcomposefs1, debugedit, gir1.2-flatpak-1.0, appstream-compose, …). Also added the **system flathub remote**. apt exit 0.
- Rust: rustup was already installed for `michael`. I ran `rustup update stable` → **rustc 1.99.0 (2026-09-28)**, and rustup self-updated to 1.29.1. There is also a distro `rustc` 1.93 in /usr/bin, which is shadowed by ~/.cargo/bin in a login shell.
- **VERIFIED:** `/tmp/autopaper-smoke` (gtk4 0.11.5 `gnome_50` + libadwaita 0.9.2 `v1_9`, AdwStatusPage + AdwToggleGroup) built in 23 s as michael and rendered on the GNOME desktop ("libadwaita 1.9.1 / GTK 4.22.4").
- Issue: the guest **clock is ~34 h behind** (guest 2026-10-04 11:26 UTC vs RTC 2026-10-05 21:46). `timedatectl` reports NTP inactive and "System clock synchronized: no". chrony is installed but disabled, while timesyncd is enabled. Builds and TLS still worked. Suggested fix (not applied): `sudo timedatectl set-ntp true` or `sudo chronyc makestep`.
- Flatpak runtimes are not installed yet (about 400 MB each).

### Docker alternative (linux/aarch64 on this Mac)
`docker run --rm ubuntu:26.04 …` resolves to **Ubuntu 26.04.1 LTS** with candidates `libadwaita-1-dev 1.9.1-0ubuntu0.1`, `libgtk-4-dev 4.22.4+ds-0ubuntu0.1`, `libglib2.0-dev 2.88.0-1ubuntu0.1`, `flatpak-builder 1.4.8-1`. This matches the VM exactly, so it is good for headless CI builds. It cannot be used for GUI or portal testing.

### "Ubuntu Linux 26" VM (not to be used)
I resumed it and probed it (Ubuntu 26.04 LTS aarch64, GNOME 50.1, user `ttadmin`, libadwaita 1.9.0, GTK 4.22.4, no -dev packages, no rustup). Then I launched an apt install script (`apt-get update` + install of build-essential, pkgconf, libgtk-4-dev, libadwaita-1-dev, flatpak, flatpak-builder, desktop-file-utils, appstream, orca, plus a flathub remote-add), logging to `/root/autopaper-install.log`. The VM was **suspended externally while that script was running** (prlctl returned `PRL_ERR_IO_STOPPED`), so the install may be **partially applied**. The VM is now suspended and I have not touched it since. On its next resume, check `/root/autopaper-install.log`, then `sudo dpkg --configure -a && sudo apt-get -f install` if dpkg was interrupted.
