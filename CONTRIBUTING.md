# Contributing to AutoPaper

Thanks for considering a contribution. AutoPaper is three native apps over one shared Rust core, and the bar for landing a change is: it matches the patterns already here, keeps the tests green on every platform it touches, and doesn't add a dependency without a reason.

## TL;DR

1. Fork the repo and create a topic branch off `main`.
2. Make your change. Keep it small and focused.
3. Run `cargo test` and `cargo clippy --all-targets -- -D warnings`, and build each app your change touches (see [Build from source](./README.md#build-from-source)).
4. Open a PR with a short description of what changed and why. For visual changes, include a screenshot from each platform you changed.

**No CLA. No rights assignment.** Your contributions remain yours, licensed under [MIT](./LICENSE) to match the project. By opening a PR you confirm you wrote the change or have the right to contribute it under that license.

## Where things go

- **Behaviour lives in the core** (`core/`): composing, memory and echoes, taste, moods, the schedule, the budget, storage and every network request. If all three apps would need the same logic, it belongs here, with tests.
- **The apps are thin and native** (`apps/macos`, `apps/windows`, `apps/linux`): each uses its platform's own controls and follows its own guidelines (Apple's HIG, Fluent for Windows 11, the GNOME HIG).
- **[`docs/app-spec.md`](./docs/app-spec.md)** says what every app does. A change in behaviour goes there and into all three apps, or says in the spec which platforms don't have it yet and why. The website's help says the same thing to people.
- **The website** (`site/`) and the two documents it publishes, [PRIVACY.md](./PRIVACY.md) and [NETWORK.md](./NETWORK.md), describe the apps as they are. A change in what's sent, to whom or when updates both in the same PR.

## Dev setup

Everything needs [rustup](https://rustup.rs) with the stable toolchain. The README has each platform's [build steps](./README.md#build-from-source); the loop for the core:

```sh
git clone https://github.com/<your-fork>/AutoPaper
cd AutoPaper
scripts/fetch-model.sh                          # the embedding model, into models/ (git-ignored)
cargo test                                      # the core and the CLI, no network needed
cargo clippy --all-targets -- -D warnings       # as CI runs it
cargo run -p autopaper-cli -- generate          # a whole wallpaper from the terminal (Demo until `settings set`)
```

After a change to the core's exported API, rebuild the bindings: `scripts/build-xcframework.sh` for the Mac app, `scripts\build-core-windows.ps1` (in Windows) for the Windows app. The Linux app links the core directly.

### API keys for development

The apps keep keys in the platform's secure store (Settings → Accounts, or Keys). The `autopaper` developer CLI reads them from the environment or a `.env` file: copy `.env.example` to `.env` and fill in what you have. `.env` is git-ignored; never commit keys, fixtures that contain keys, or screenshots that show them. CI runs [gitleaks](https://github.com/gitleaks/gitleaks) on every push.

### UniFFI

The core is exported to Swift and C# with UniFFI, pinned at `=0.31.2` because `uniffi-bindgen-cs` v0.11.0 supports only that version's metadata. Two rules from experience:

- Exported traits can't have default method implementations, so adding a method to a trait the apps implement breaks every app. Add a new trait instead (see `ProgressDetailObserver`).
- Generated Swift compiles in Swift 5 language mode (`build-xcframework.sh` sets it on the bindings target); the app itself is Swift 6.

## Adding a provider

Each provider is one file in `core/src/providers/`, registered in `core/src/providers/registry.rs`.

- **All requests go through `core/src/net.rs`.** Its rules (hosted providers only at their API host, plain `http` only to local addresses, same-host redirects, time and size limits) apply to every provider; don't open your own client.
- **Keys go through the `SecretStore`**, in a header, and only to their own service.
- **Test against recorded responses.** Save a real response under `core/tests/fixtures/<provider>/` (checking that it holds no keys or personal data) and exercise it through the stub HTTP client. Live tests are `#[ignore]`d.
- **Say what it sends.** Add its hosts, requests, triggers and limits to [NETWORK.md](./NETWORK.md), and what it learns to [PRIVACY.md](./PRIVACY.md).

## Code style

- **Rust:** edition 2024; `cargo clippy --all-targets -- -D warnings` must pass (CI checks the core on all three systems and the Linux app). Errors are typed (`AutoPaperError`), never stringly.
- **Swift:** Swift 6 language mode with strict concurrency; UI and AppKit work on `@MainActor`.
- **C#:** .NET 10 and WinUI 3, with CommunityToolkit's MVVM and settings controls.
- **Everywhere:** small types, doc comments on public API and non-obvious decisions, no commented-out code, and match the surrounding code.
- **Dependencies** are few on purpose. Discuss a new one in an issue first. A new Rust crate must have a licence `scripts/third_party_notices.py` accepts; regenerate `THIRD-PARTY-NOTICES.md` with it after any change to `Cargo.lock` (it needs [cargo-about](https://github.com/EmbarkStudios/cargo-about)).

## Wording

AutoPaper's voice is plain, specific and short: no hype, no "AI magic", no exclamation marks. Mac menus and buttons use title case; Windows and Linux use sentence case; the words are otherwise the same, so the help and the website match every app. **A problem is said once, as a link to its fix**, not as directions ("Add your OpenAI key", linking to Settings → Accounts).

## Accessibility

AutoPaper follows WCAG 2.2 AA through [WCAG2ICT](https://www.w3.org/TR/wcag2ict-22/), plus each platform's accessibility guidelines. For any UI change:

- Every button, image and field has a spoken name that makes sense out of context; decorative images are hidden from screen readers.
- Nothing relies on colour alone; nothing moves when reduced motion is on; anything that changes on its own can be paused.
- Check it through the platform's accessibility interface: on macOS the accessibility API (`AXDescription`, `AXTitle`) or Accessibility Inspector; on Windows UI Automation (Accessibility Insights or AxeWindows) and Narrator; on Linux AT-SPI (Accerciser) and Orca.
- Website changes: run Lighthouse's accessibility audit in light and dark mode.

## Releasing (maintainer)

All three platforms release together, at one version (`version` in `Cargo.toml`, `MARKETING_VERSION` in `apps/macos/project.yml`, the Windows package manifest, and the AppStream metainfo's release notes):

1. **macOS:** `scripts/release.sh` archives a Release build, exports it with the Developer ID certificate, notarizes and staples the app and the `.dmg`, and signs the Sparkle update `.zip` with the EdDSA key in the maintainer's Keychain (`generate_keys --account AutoPaper`). `scripts/appcast.py` adds it to `site/static/appcast.xml`.
2. **Windows** (in Windows): `scripts/windows-release.ps1` builds one x64 + ARM64 `.msixbundle`, signs it with Azure Artifact Signing, and writes `site/static/AutoPaper.appinstaller` and the winget manifests (`packaging/winget/`).
3. **Linux:** `scripts/linux-release.sh` builds the `.flatpak` bundle and the files a Flathub submission needs (`packaging/flatpak/`), checked with Flathub's linter.
4. Publish the GitHub release with every file **before** pushing the feeds, since `appcast.xml` and `AutoPaper.appinstaller` point at the release's files. The website, feeds included, deploys itself on push to `main`.
5. Then the stores: a pull request to [winget-pkgs](https://github.com/microsoft/winget-pkgs) for `msitarzewski.AutoPaper`, and the update to the Flathub repository for `io.github.msitarzewski.AutoPaper`.

## Reporting bugs

Open an [issue](https://github.com/msitarzewski/AutoPaper/issues/new/choose) with:

- your system and version, and the AutoPaper version
- the providers and models you use for writing and painting, if it's about a wallpaper
- what you expected and what happened
- logs, if you can: on macOS, Console filtered by subsystem `com.autopaper`; on Windows, `logs\autopaper.log` in AutoPaper's data folder; on Linux, the output of `RUST_LOG=debug flatpak run io.github.msitarzewski.AutoPaper`. AutoPaper never logs keys, keywords or prompts.

For security issues, follow [SECURITY.md](./SECURITY.md) instead of opening a public issue.
