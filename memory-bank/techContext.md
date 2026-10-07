# Tech Context

Versions verified 2026-10-05 (sources in `docs/research/rust-linux.md`, `docs/research/providers.md`,
`docs/research/windows.md`).

## Core (Rust)
- Workspace: `core/` (`autopaper-core`, crate-type `lib` + `cdylib` + `staticlib`), `cli/` (`autopaper`),
  `uniffi-bindgen/` (bindgen binaries, `cli` feature), `apps/linux/` (GTK app crate).
- **UniFFI `=0.31.2`** (pinned: uniffi-bindgen-cs `v0.11.0+v0.31.0` doesn't support 0.32 yet). Proc-macros,
  no UDL; host-implemented traits use `#[uniffi::export(with_foreign)]`; async exports use
  `async_runtime = "tokio"`. Verified end to end (Swift + C#, sync and async foreign traits) with a toy crate.
  - `#[uniffi::export(name = "default_base_url")]` exports a Rust function under another name (used so Rust hosts
    keep `registry::default_base_url -> Option<&'static str>`). Error variants with enum fields
    (`InvalidInput { reason, detail }`) generate fine in both languages (2026-10-06): Swift `case
    InvalidInput(reason: InvalidInputReason, detail: String)`; C# `AutoPaperException.InvalidInput` with
    `reason`/`detail` properties.
  - **Round 4 check (2026-10-06):** the VM's uniffi-bindgen-cs (v0.11.0+v0.31.0) generated the C# from the Mac's
    *debug dylib* (UniFFI metadata reads the same from Mach-O): it must run inside a Cargo workspace (it calls `cargo
    metadata`; from a bare folder it fails "error running cargo metadata"), so `.scratch/windows/core-step-cs-gen.ps1`
    mirrors the repo to `C:\Users\michael\dev\core-step-cs\repo` and runs there (332,993 B `autopaper_core.cs`);
    `.scratch/windows/core-step-cs-build.ps1` compiles it with a file using every new API
    (`.scratch/core-step/cs/Check`, net10.0-windows): 0 warnings, 0 errors. No Windows Rust build needed for a
    binding check. C# returns arrays (`Mood[] Moods()`, `Generation[] HistoryByMood(...)`), records have PascalCase
    properties (`Mood.Active`, `Generation.MoodName`, `ProgressDetail.SecondsLeft`), exception fields stay lowercase
    (`AutoPaperException.PaintingFailed.model`).
  - uniffi-bindgen-cs on the Mac: not installed permanently. For a check, install it to a throwaway root
    (`rustup run stable cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs
    --tag v0.11.0+v0.31.0 --root <tmp>`, ~1 min) and run it on the macOS dylib (`--library
    target/debug/libautopaper_core.dylib --config core/uniffi.toml --out-dir <tmp> --no-format`): the generated
    C# is the same as from the Windows DLL (metadata, not the platform). There's no dotnet on the Mac; compile
    it in the Windows VM.
- Crates: rusqlite 0.40.2 (`bundled`), reqwest 0.13.5 (rustls default), tokio 1.53.2 (+ `net` since round 4),
  serde/serde_json, image 0.25.10, image_hasher 3.1.1, thiserror 2.0.21, tracing 0.1.44, chrono 0.4.45, async-trait.
  **Round 4:** tokio-tungstenite **0.30.0** (MIT; `default-features = false, features = ["handshake"]`: no TLS, no
  connector — the core opens the TCP connection itself, plain `ws://` only to the local/private destinations plain
  http may reach, never through a proxy) + futures-util 0.3 (`StreamExt`), for ComfyUI's `/ws` progress events
  (`HttpClient::open_socket`, `net.rs`). ComfyUI sends a job's `progress` only to the `clientId` that queued it;
  `/api/jobs/{id}` and `/queue` carry no step progress (checked in the user's ComfyUI 0.36 source:
  `comfy_execution/jobs.py`, `main.py hijack_progress`), so polling alone can't show steps or tell a stall.
- UniFFI exported traits can't have default method implementations (uniffi_macros 0.31.2 `export/item.rs`:
  "uniffi::export'd trait methods can't have a default implementation"), so a new callback method on an existing
  foreign trait breaks every host: round 4's progress detail is a second foreign trait registered on the engine
  (`ProgressDetailObserver`, `Engine::set_progress_detail_observer`).
- Embeddings: **candle 0.11.0** (pure Rust) + tokenizers 0.22, model **BAAI/bge-small-en-v1.5** (MIT,
  384-dim, 133 MB; URL + SHA-256 in `docs/research/rust-linux.md`), fetched by a script, never committed
  (`models/` is ignored). Fallback engine: tract 0.23.8. ort/fastembed rejected (native ONNX Runtime).

## macOS
- Swift 6.4 / Xcode (beta 27) / XcodeGen 2.46; deployment target macOS 26 (as AudioPaper).
- XCFramework: build `aarch64-apple-darwin` + `x86_64-apple-darwin` static libs → `lipo` → `uniffi-bindgen-swift`
  (`--swift-sources`, then `--headers --modulemap --module-name autopaper_coreFFI`) → `xcodebuild
  -create-xcframework`. Exact commands in `docs/research/rust-linux.md`. Don't pass `--xcframework` to bindgen.
- **`scripts/build-xcframework.sh [--smoke]`** (phase 2) does all of it, repeatably (~3 s when nothing changed;
  ~1 min 10 s per architecture for a release build on this Mac) → `apps/macos/Generated/AutopaperCore/` (git-ignored): a local
  Swift package — `Package.swift` (swift-tools 6.2, `platforms: [.macOS(.v26)]`, library `AutopaperCore`),
  `AutopaperCoreFFI.xcframework` (`macos-arm64_x86_64/libautopaper_core.a`, ~57 MB, + `Headers/autopaper_coreFFI/`),
  and `Sources/AutopaperCore/autopaper_core.swift` in a target with `.swiftLanguageMode(.v5)`. The app adds it as
  a local package (XcodeGen `packages: AutopaperCore: path: Generated/AutopaperCore`) and stays Swift 6; a Swift 6
  executable consumes it fine (the smoke test). Release profile, `cargo rustc --crate-type staticlib` (no cdylib).
  - **linkerSettings come from rustc** (`--print native-static-libs`, parsed by the script, both architectures):
    `.linkedFramework("Security")`, `.linkedFramework("SystemConfiguration")`, `.linkedFramework("CoreFoundation")`,
    `.linkedLibrary("iconv")` (libSystem/libc/libm are implicit). Security + CoreFoundation: rustls-platform-verifier
    / security-framework; SystemConfiguration: system proxy settings (reqwest).
  - **C dependencies target macOS 26:** `CFLAGS_aarch64_apple_darwin` / `CFLAGS_x86_64_apple_darwin` =
    `-mmacosx-version-min=26.0` (cc-rs reads them; aws-lc-sys, onig_sys, libsqlite3-sys). Checked with `otool -l`:
    C objects minos 26.0; Rust objects keep rustc's defaults (arm64 11.0, x86_64 10.12); nothing at the host's 27.x.
  - `--smoke` builds and runs a Swift 6 executable against the package (Engine on a temp dir, Demo provider, one
    keyword, one generation). 2026-10-05: `providers: demo / demo`, `title: Lighthouse over Terraced Garden`,
    `image: 3840x2160, memory: hashing-v1`.
  - It unsets `MACOSX_DEPLOYMENT_TARGET` (Xcode exports it to run-script phases) and respects `CARGO_TARGET_DIR`.
- **Generated Swift needs Swift 5 language mode** (`.swiftLanguageMode(.v5)` on the bindings target):
  async foreign-trait methods don't compile in Swift 6 mode. The app target stays Swift 6.
- Don't set `MACOSX_DEPLOYMENT_TARGET` for the Rust release build (link fails: "mis-aligned LINKEDIT
  string pool"); Xcode applies the deployment target at final link.
- **This Mac's Rust:** `/opt/homebrew/bin/cargo` (Homebrew rust 1.98.1) is first on PATH and has no x86_64
  std. Use rustup's toolchain: `rustup run stable cargo …` (rustup is Homebrew's; its proxies aren't on
  PATH). Installed targets: aarch64/x86_64-apple-darwin, x86_64-unknown-linux-gnu.
  - **Gotcha (found in phase 2):** `rustup run stable cargo` runs rustup's cargo 1.96 but cargo then calls the first
    `rustc` on PATH — Homebrew's 1.98.1 (and `cargo clippy` gets clippy 0.1.98). Fine for host tests; for other
    targets put the toolchain first: `PATH="$(dirname "$(rustup which --toolchain stable rustc)"):$PATH"` (the
    XCFramework script does). Phase 2's tests and clippy pass with both rustc 1.98.1 and 1.96.0.
- Icon: `docs/icon/build_icons.py` (AudioPaper's, repurposed: display + Heroicons sparkles), shipped
  **red** (AudioPaper's; user rejected violet as "very AI") → `apps/macos/AppIcon.icon`; `--site` exports the website PNGs. Needs Xcode-beta's `ictool` + Pillow.

## Linux
- **gtk4 0.11.5** (feature `gnome_50`) + **libadwaita 0.9.2** (feature `v1_9`): Ubuntu 26.04 / Fedora 44
  ship GTK 4.22 / libadwaita 1.9; GNOME 51 runtime has 4.24 / 1.10. AdwToggleGroup, AdwPreferencesDialog,
  AdwSpinRow, AdwPasswordEntryRow all available.
- **ashpd 0.13.13** (Wallpaper, Background, Settings portals), **oo7 0.6.0** (Secret Service, in and out of
  Flatpak), **ksni 0.3.6** (KDE tray only; GNOME uses Background portal + notifications).
- Portal facts: use `SetWallpaperFile` (portal rejects `file://` URIs); always `show_preview(false)` (KDE
  previews by default); GNOME ignores set-on and sets light + dark backgrounds; unsandboxed apps must call
  `ashpd::register_host_app` for autostart; KDE's Secret portal is in flux — test oo7 on Plasma.
- Flatpak: `org.gnome.Platform//51` + `org.freedesktop.Sdk.Extension.rust-stable//26.08`; finish-args
  network, ipc, wayland, fallback-x11, dri (portals need no extra permissions).

## VMs (Parallels; user-approved for dev tooling)
- **Scratch** (Linux): Ubuntu 26.04.1 LTS aarch64, GNOME Shell 50.1 Wayland, 8 CPU / 16 GB, user `michael`.
  - `prlctl exec` runs as root and mangles quoting → pass base64 scripts:
    `prlctl exec "Scratch" "echo $B | base64 -d | bash"`; as the desktop user: `sudo -u michael -i bash -lc '…'`.
  - GUI apps: `XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0
    DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus setsid <app>`.
  - Screenshots from the Mac: `prlctl capture "Scratch" --file x.png` (gnome-screenshot fails on Wayland).
  - Installed 2026-10-05: libgtk-4-dev, libadwaita-1-dev, flatpak 1.16.6, flatpak-builder 1.4.8 (+ Flathub);
    rustup stable 1.99.0 for `michael`. Already present: build-essential, git, orca, portals, gnome-keyring.
    No Flatpak runtimes installed yet (~400 MB each).
  - Clock ~34 h behind (NTP inactive); `sudo timedatectl set-ntp true` would fix it (not yet changed).
- Docker `ubuntu:26.04` (linux/aarch64) has the same GTK/libadwaita/flatpak-builder versions — for CI-like builds.
- **Windows 11**: see Windows section (research in progress).
- "Ubuntu Linux 26": NOT used (user switched to Scratch). An apt install was interrupted there by suspend;
  on its next resume: check `/root/autopaper-install.log`, run `sudo dpkg --configure -a && sudo apt-get -f install`.

## macOS test VM (tart, headless — GUI tests without touching this Mac's desktop)
Set up 2026-10-06. Everything visual (screenshots, wallpaper changes, accessibility audits, the app) happens inside
the VM; nothing appears on the Mac's screen.
- **tart 2.40.1**: `brew install openai/tools/tart` (Cirrus Labs' tart now lives at `openai/tart`; the old
  `cirruslabs/cli` tap is stuck at 2.32.1 and fails under Homebrew 7; `brew trust --formula openai/tools/softnet`
  was needed for its dependency). Licence **FSL-1.1-ALv2** (Fair Source, becomes Apache-2.0 after two years): internal
  use permitted; tart.run/licensing: "Usage on personal computers including personal workstations is royalty-free"
  (organisations free up to 100 host CPU cores). The formula's DHCP-lease caveat (a host setting) was not applied.
- **Image** `ghcr.io/cirruslabs/macos-tahoe-base:latest`: macOS **26.6.2 (25G83)**, uploaded 2026-10-03, 27 GB
  download, 50 GB sparse disk; Homebrew, Command Line Tools (Swift 6.4), tart-guest-agent. User **admin / admin**,
  auto-login, passwordless sudo, **SIP disabled**, Gatekeeper assessments on (copied apps carry no quarantine, so
  it doesn't matter). macOS 27 exists as `macos-golden-gate-{vanilla,base,xcode}` (27.0 26A428; base 34 GB): not
  pulled (a second VM later). Image list: github.com/orgs/cirruslabs/packages?q=macos.
- **VMs**: `autopaper-mac26` (working VM: 6 CPU, 16 GB, `--display 2560x1600px` = 2560×1600 at 1x — without `px`
  tart reads points and makes 5120×3200 on this Retina Mac), `autopaper-mac26-clean` (snapshot taken after setup,
  never used for tests), `autopaper-mac26-run` (throwaway). Clones are APFS copy-on-write (`reset` takes 0.1 s).
- **Driver `scripts/macvm/macvm.sh`** (+ `scripts/macvm/vmtool.swift`, compiled in the VM to `~/macvm/vmtool`):
  - `setup` — pull, clone, `tart set`, first boot, SSH key, in-VM settings (below), vmtool, shutdown, clean clone.
  - `start` — `tart run autopaper-mac26 --no-graphics --no-audio --no-clipboard --dir=autopaper:<repo>:ro` in the
    background (log `.scratch/macvm/<vm>.log`), waits for SSH and Finder/Dock: ~2 min 15 s. tart is `BackgroundOnly`
    (no Dock icon) with 0 host windows (checked). Only `scripts/macvm` is shared, read-only, at `/Volumes/My Shared Files/macvm` — never the whole repo (`.env` holds API keys); the app is copied in over SSH.
  - `ssh ['<cmd>']` — `ssh -i .scratch/macvm/id_ed25519 admin@$(tart ip autopaper-mac26)` (key installed at first
    boot with admin/admin via SSH_ASKPASS; host-key checking off — local NAT VM, IPs reused).
  - `gui '<cmd>'` — in the auto-logged-in Aqua session: over SSH, `sudo launchctl asuser $(id -u admin) sudo -u admin
    /bin/bash -c '<cmd>'` (`launchctl managername` → Aqua). No-network alternative: `tart exec autopaper-mac26 <cmd>`
    (guest agent; also Aqua, also AX-trusted).
  - `shot <name>` — `screencapture -x` in the VM → `.scratch/shots/macvm/<name>.png` (2560×1600).
    `winshot <owner> <name>` — window id from `vmtool windows <owner>` (CGWindowListCopyWindowInfo, layer 0) →
    `screencapture -x -o -l <id>`.
  - `tool windows [owner]` · `tool wallpaper get` · `tool wallpaper set <file-in-vm>` (NSWorkspace
    `setDesktopImageURL`, read back with `desktopImageURL(for:)`) · `tool ax <bundle-id|name|pid> [depth]` ·
    `tool press <app> <title>` (AXPress on a button, menu item, radio button or checkbox).
  - `install-app [App.app]` — `ditto` from the share to `/Applications` (default: the Debug build), then
    `codesign --verify --deep --strict`. `launch-app` — `open` in the GUI session, then prints any amfid / syspolicyd
    / kernel / launchservicesd log lines about it. `quit-app` — SIGTERM (an Apple Event quit would need an Automation
    prompt in the VM).
  - `reset` → fresh `autopaper-mac26-run` from the clean clone; use it with
    `AUTOPAPER_MACVM=autopaper-mac26-run scripts/macvm/macvm.sh start` (any command takes the variable).
  - `stop` — `sudo shutdown -h now` over SSH (clean), `tart stop` after 90 s. `status` — VMs, sizes, IP.
- **Demo slow mode for progress UI (round 4):** `AUTOPAPER_DEMO_DELAY_SECS=N` (1–3600) read when the engine opens:
  a Demo wallpaper then takes N s (a quarter writing, the rest painting as 8 reported steps), so the ring and "about
  N s left" can be checked without ComfyUI. macOS: `open --env AUTOPAPER_DEMO_DELAY_SECS=60 -a AutoPaper` (or
  `launchctl setenv` before launching) in the VM's GUI session; Windows: a user environment variable before
  starting the app; Linux: the variable in the launching environment (`flatpak run --env=…` inside Flatpak). The
  second run shows time left from its start (the first records the timing). Off by default. Proved through the CLI
  on the Mac (`AUTOPAPER_DEMO_DELAY_SECS=4 autopaper generate`: 12 → 99 % with time left); passing it to the
  sandboxed app with `open --env` is not yet tried.
- **Accessibility audits work out of the box.** The Cirrus base image's build (`scripts/update-tcc-database.sh` in
  github.com/cirruslabs/macos-image-templates) grants Accessibility, ScreenCapture and PostEvent in the system TCC.db
  to `/usr/libexec/sshd-keygen-wrapper` (the responsible process for everything started over SSH), `osascript`,
  Python and `tart-guest-agent`. So any command-line tool run through `gui`/`tool` gets `AXIsProcessTrusted() ==
  true`; no recovery boot or TCC edits needed. `tool ax com.autopaper 12` prints role, title, description (= the
  VoiceOver label), identifier and value, and lists controls with no label. The images also ran `automationmodetool
  enable-automationmode-without-authentication`, so XCUITest would run without a prompt (needs Xcode in the VM:
  the `-xcode` image, not pulled).
- **Proved 2026-10-06:** SSH, sudo, read-only share, GUI-session commands, full-screen and window-only capture,
  wallpaper set + read back + screenshot, AX queries of Finder, TextEdit and a system dialog. **AutoPaper.app
  (Debug, Apple Development, team 7JQGQ7CRH8, App Sandbox, no provisioning profile) launches in the VM**: signature
  valid, no amfid/syspolicyd rejection, Welcome window audited (25 elements, 0 unlabelled), Demo chosen with
  `tool press com.autopaper "Try it without AI"`, quit. No "Sign to Run Locally" build needed. The working VM still
  has the app and a changed wallpaper; the clean clone has neither. Shots: `.scratch/shots/macvm/01…14`.
- **Gotchas:**
  - **Screen-capture notice:** after the first capture from SSH (and every 30 days) macOS 26 shows "“com.apple.sshd-
    session” is requesting to bypass the system private window picker…" (UserNotificationCenter) in the VM. The
    capture still works. `setup` sets `kScreenCapturePrivacyHintDate` to 2100 for sshd-keygen-wrapper in
    `~/Library/Group Containers/group.com.apple.replayd/ScreenCaptureApprovals.plist`. replayd ignores SIGTERM and
    rewrites the file from memory, so it is SIGKILLed first. If the notice shows anyway, `shot`/`winshot` AX-press
    Allow and capture again.
  - **Stale virtiofs files:** the guest keeps stale entries (old size and date, then "No such file") for files the
    Mac replaced by rename (editor saves, Xcode rebuilds). Remounting fixes it (`refresh_share`: umount, the
    automounter remounts, never forced); `install-app` and `tool` do this first.
  - **Wallpaper read-back lags:** right after `setDesktopImageURL`, `desktopImageURL(for:)` still returned the old
    picture; the new one showed 0.3–3 s later (WallpaperAgent applies it asynchronously). vmtool polls up to 10 s.
    The app's own checks may hit this too.
  - The image was saved with Terminal open, and login reopened it: setup turns off window restoration and closes it.
    `softwareupdate --schedule off` doesn't work on 26; setup turns off automatic update downloads/installs (catalog
    checks stay on, a few KB) and Spotlight indexing.
  - Only `scripts/macvm` is shared (read-only); the repo and its `.env` are not visible in the VM (verified 2026-10-06: 0 `.env*` files under the share).
  - The VM runs on UTC. After the VM stops, `tart ip` still prints its last lease (the script asks `tart list`).
  - No Apple Intelligence in macOS VMs (Apple doesn't support it there), so anything built on it can't be tested
    here. GPU is paravirtual (Metal works, limited GPU family; the base image's opt-in Metal shim needs a host
    `defaults write`: not done). At most two macOS VMs can run at once (Apple's limit); run one (the 16 GB rule).
  - Host isolation: no window, `--no-audio`, `--no-clipboard`, NAT 192.168.64.0/24. SSH from Terminal-launched tools
    is exempt from Local Network privacy, so no prompt on the Mac.
- **Disk:** `~/.tart/cache` 31 GB (image) plus ~4 GB of copy-on-write changes for both VMs ≈ 35 GB actually used
  (`du`/`tart list` show 29–33 GB each because clones share blocks). **RAM:** 16 GB while running, none stopped.
  Remove everything: `tart delete autopaper-mac26-clean && tart delete autopaper-mac26 && tart prune --entries caches`.

## Windows (`docs/research/windows.md`)
- **.NET SDK 10.0.401** (LTS), TFM `net10.0-windows10.0.26100.0`, `TargetPlatformMinVersion` 10.0.22000.0
  (Windows 11 only). **Microsoft.WindowsAppSDK 2.5.1** (SemVer now; 1.8 out of servicing),
  Microsoft.Windows.SDK.BuildTools 10.0.28000.2705, Microsoft.Windows.SDK.BuildTools.WinApp 0.7.1,
  winapp CLI 0.7.1 (`winget install Microsoft.WinAppCli`). Templates: `Microsoft.WindowsAppSDK.WinUI.CSharp.Templates`
  0.0.7-alpha (`dotnet new winui-navview`).
- CommunityToolkit.WinUI.Controls SettingsControls / Segmented / TokenizingTextBox **8.2.251219**,
  CommunityToolkit.Mvvm 8.4.2, **WinUIEx 2.9.3** (`TrayIcon`; H.NotifyIcon.WinUI 2.4.1 is the alternative),
  **Microsoft.Windows.CsWin32 0.3.346** (`IDesktopWallpaper`, `CredWrite`/`CredRead`).
- Secrets: **Windows Credential Manager** (CredWrite/CredRead via CsWin32), not PasswordVault (roams with the
  Microsoft account, 20-entry cap).
- Lock screen: write each image (new file name) under `ApplicationData.Current.LocalFolder`, then
  `UserProfilePersonalizationSettings.TrySetLockScreenImageAsync`, falling back to `LockScreen.SetImageFileAsync`;
  if both fail say it's managed by the organisation. Give `IDesktopWallpaper` the real `LocalState` path
  (packaged apps' `%LOCALAPPDATA%` writes are redirected). Untested live so far.
- Don't use `PublishTrimmed` (template default in Release breaks WinAppSDK/classic COM) unless CsWin32's COM
  source generators are used. Don't enable `PublishAot` until uniffi-bindgen-cs callbacks are tested under it (#175).
- uniffi-bindgen-cs `v0.11.0+v0.31.0`: generated names have no `I` prefix (`SecretStore`); settings in
  **`core/uniffi.toml`** `[bindings.csharp]`: `namespace = "AutoPaper.Core"`, `access_modifier = "public"`,
  `cdylib_name = "autopaper_core"`.
- **`scripts/build-core-windows.ps1 [-Arch arm64|x64|all] [-TargetDir C:\…]`** (phase 2; run inside Windows from a
  local copy, Windows PowerShell 5.1): `rustup run stable cargo rustc -p autopaper-core --lib --release --target
  <triple> --crate-type cdylib` for aarch64-pc-windows-msvc (+ x86_64 when it builds; with `all`, an x64 failure
  is reported as skipped) → `uniffi-bindgen-cs --library <dll> --out-dir … --config core\uniffi.toml
  --no-format` (run from the repo root) → `apps\windows\Generated\autopaper_core.cs`,
  `Generated\win-arm64\autopaper_core.dll`, `Generated\win-x64\autopaper_core.dll` (git-ignored). Checks first:
  rustup targets, uniffi-bindgen-cs version (`cargo install --list`), clang-cl for ARM64 (aws-lc-sys 0.45's cc
  builder needs it: PATH or `<VS>\VC\Tools\Llvm\{ARM64,x64}\bin`), target dir not on a network share. Without
  `nasm`, x64 uses aws-lc-sys's prebuilt NASM objects (`AWS_LC_SYS_PREBUILT_NASM=1`). **Run once in the VM (2026-10-05)**
  from an isolated copy (`C:\Users\michael\dev\autopaper-corecheck`, target dir
  `C:\Users\michael\dev\autopaper-corecheck-target`, both left there as a warm cache): arm64 5 min 41 s, x64
  4 min 55 s (prebuilt NASM), no warnings; `autopaper_core.cs` 250,810 B (`namespace AutoPaper.Core;`, `public class
  Engine`), DLLs 12.8 MB (arm64) / 14.5 MB (x64). Not yet compiled into a C# project.
- Rust on Windows ARM64 needs **clang-cl** (VS Build Tools "C++ Clang tools") for ring/aws-lc-rs and tract;
  on x64 aws-lc-rs needs NASM or `AWS_LC_SYS_PREBUILT_NASM=1`. The VM has clang-cl at
  `C:\BuildTools\VC\Tools\Llvm\{ARM64,x64}\bin` (not on PATH; aws-lc-sys finds it there) and no nasm (checked
  2026-10-05).
- **VM "Windows 11"**: Windows 11 Pro **Insider evaluation build 29680, ARM64** (will expire). Installed
  2026-10-05: .NET SDK 10.0.401 arm64, rustup (rustc 1.99.0, aarch64 + x86_64 MSVC targets) for `michael`,
  uniffi-bindgen-cs, winapp 0.7.1, WinUI templates, AxeWindowsCLI 2.4.2 (`C:\Tools\AxeWindowsCLI-2.4.2`),
  Developer Mode on. Already present: VS Build Tools 2022 17.14 (ARM64/x64 C++, Windows SDK 10.0.26100), Git 2.55.
  - Repo: `\\Mac\Home\Clean\autopaper` (desktop user: `Z:\Clean\autopaper`). **Packages can't be registered
    from the share (0x80073CFD)**: `robocopy \\Mac\Home\Clean\autopaper C:\Users\michael\dev\autopaper /MIR
    /XD .git bin obj target AppX .scratch /XF .env .env.*` (never copy `.env`), build/run/pack there; `CARGO_TARGET_DIR` on local disk too.
  - `prlctl exec "Windows 11" <cmd>` = SYSTEM (elevated: machine installs, cert trust, HKLM);
    `prlctl exec "Windows 11" --current-user <cmd>` = `windows-arm\michael`, not elevated (dotnet, cargo,
    winapp, Add-AppxPackage, UI tests; apps appear on the desktop).
  - Scripts: put `.ps1` in `.scratch/windows/`, run `powershell -NoProfile -ExecutionPolicy Bypass -File
    '\\Mac\Home\Clean\autopaper\.scratch\windows\x.ps1'` (long `-EncodedCommand` fails through prlctl).
  - Screenshots: `prlctl capture "Windows 11" --file out.png` (1465×839). Accessibility: `winapp ui inspect -a
    <proc> -i`, AxeWindowsCLI.
  - Reference smoke projects left in `C:\Users\michael\dev\ApSmoke` and `C:\Users\michael\dev\uniffi-smoke`.

## Local AI on this Mac (for development and the website's examples)
- ComfyUI: `/Users/michael/Software/ComfyUI`, start with `./run.sh` (loopback :8188); docs in
  `~/Clean/local-inference/memory-bank/runtimes.md`. Don't modify the install (uv venv, no pip).
  - Version 0.36.0. Has `POST /api/jobs/{id}/cancel` (ComfyUI PR #14493, 2026-06-19; documented in its
    `openapi.yaml`): interrupts that job if it's running, dequeues it if pending, idempotent `{"cancelled": bool}`.
    Older servers answer 404 there; their `/interrupt` without a body stops whatever is running.
  - Apple silicon VAE bound (2026-10-06): with an 8× VAE (Z-Image Turbo's `ae.safetensors`, Krea 2's
    `qwen_image_vae.safetensors`) VAEDecode fails ("MPSGraph does not support tensor dims larger than INT_MAX")
    once (W/8)(H/8) > 46,340 and ComfyUI didn't slice the attention (it slices only when free memory is short):
    2048×1456 and 2048×2048 failed, 2048×1440 worked, 2544×1632 failed once and passed once. Qwen-Image 2.1's
    16× VAE decodes 2752×1536 and 2048² fine. Probe: `python3 .scratch/examples/size-probe.py <graph> W H [steps]`
    runs a model's real graph (e.g. `.scratch/examples/graphs/*.request.json`, or the bundled template) at 1 step
    and prints whether VAEDecode worked (git-ignored dev tool; check a new model with it before adding a row).
  - **Round 4 live checks (2026-10-06, 768×432, ComfyUI 0.36.0 idle first):** `comfyui::tests::live_progress_…`
    heard all 8 Z-Image Turbo steps on /ws (first at 23 s: model load; then ~1 s apart; 30.6 s in all, cold);
    `live_stall_…` (Qwen-Image 2.1, socket muted after step 3, step window cut to 3 s → 4 × the 2.5 s step = 10 s)
    said `TimedOut` "ComfyUI made no progress for 10 s" and stopped the job (ComfyUI idle 0.5 s later, history
    `execution_interrupted`); `live_stopped_…` (cancelled in ComfyUI after step 3) was `Stopped` 1.4 s after the
    cancel. CLI end to end (Demo writing, `.scratch/core-step/live-data`): first painting reported 12 → 99 % with
    "about 3 s left" from the steps; `estimate` then said 20 s (that cold run: 19.3 s), 137 s at 2048×1152 by pixels;
    the second run showed "0% · about 20 s left" before its first step and took 8.6 s (estimate then 9 s; the
    measured warm 2048×1152 is ~61 s ≈ 8.6 s × 7.1 by pixels). Run them again with
    `cargo test -p autopaper-core --lib providers::comfyui::tests::live_<name> -- --ignored --nocapture --exact`
    (don't use the `live` prefix filter: it includes the 1920×1088 cancel test).
  - Real runs through the CLI (`--display WxH`, Demo writing, ComfyUI painting), 2026-10-06: Z-Image Turbo
    4112×2658 hint → 2032×1312 in 72 s; 3840×2160 → 2048×1152 in 62 s; Krea 2 Turbo (own workflow) 4112×2658 →
    2032×1312 in 140 s (15.2–15.8 s/step); Qwen-Image 2.1 (own workflow, 25 steps) 4112×2658 → 2576×1664 in
    536 s (~20 s/step) — close to the provider's 600 s ComfyUI timeout.
  Qwen-Image 2.1: Qwen Research License (LICENSE dated 2026-09-20: non-commercial = "research or evaluation purposes
  only"; outputs' ownership not addressed; "Built with Qwen" applies only to training models). Not an app default.
  **User decision 2026-10-06: the website's Qwen examples stay as they are** (the user says the Qwen team clarified
  the licence); link that clarification in the credits when the user provides it.
- Ollama (`:11434`) and LM Studio (`:1234`) are installed; LM Studio's `lms` CLI can crash the GUI on macOS 27
  (see local-inference gotchas) — don't script it.

## Website
`site/` (layout + page fragments + static), built by `scripts/build_site.py` into `_site/` (AudioPaper's
pipeline). Example wallpapers in `site/static/examples/` are painted locally with ComfyUI from
`examples.json` (not made by the app; captioned as such).
