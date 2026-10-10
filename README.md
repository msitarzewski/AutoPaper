<p align="center">
  <img src="site/static/icon-256.png" alt="AutoPaper icon: a desktop display with AI sparkles on a red screen" width="160">
</p>

<h1 align="center">AutoPaper</h1>

<p align="center"><strong>Tell your computer what you'd like to see.</strong></p>

<p align="center">
  <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <a href="#macos"><img src="https://img.shields.io/badge/macOS-26%2B-lightgrey?logo=apple&logoColor=white" alt="macOS 26 or later"></a>
  <a href="#windows"><img src="https://img.shields.io/badge/Windows-11-lightgrey?logo=windows11&logoColor=white" alt="Windows 11"></a>
  <a href="#linux"><img src="https://img.shields.io/badge/Linux-Flatpak-lightgrey?logo=flatpak&logoColor=white" alt="Linux: Flatpak"></a>
  <a href="#three-native-apps-one-core"><img src="https://img.shields.io/badge/core-Rust-B7410E?logo=rust&logoColor=white" alt="Shared core: Rust"></a>
  <a href="https://github.com/msitarzewski/AutoPaper/actions/workflows/ci.yml"><img src="https://github.com/msitarzewski/AutoPaper/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="./CONTRIBUTING.md"><img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg" alt="Pull requests welcome"></a>
  <a href="https://github.com/sponsors/msitarzewski"><img src="https://img.shields.io/badge/♥-Sponsor-EC4899?logo=githubsponsors&logoColor=white" alt="Sponsor on GitHub"></a>
</p>

<p align="center">
  <a href="https://msitarzewski.github.io/AutoPaper/"><strong>Website</strong></a> ·
  <a href="https://msitarzewski.github.io/AutoPaper/help.html">Help</a> ·
  <a href="https://msitarzewski.github.io/AutoPaper/shortcuts.html">Shortcuts</a> ·
  <a href="https://msitarzewski.github.io/AutoPaper/reference.html">Reference</a> ·
  <a href="./PRIVACY.md">Privacy</a> ·
  <a href="#install">Install</a> ·
  <a href="https://github.com/sponsors/msitarzewski">Sponsor</a>
</p>

AutoPaper is a desktop app for **macOS, Windows and Linux** that keeps making new wallpapers you're unlikely to see twice. Give it a few keywords, such as *rain, ruins, peaceful, night, blue*, instead of an image prompt. It writes a scene, has it painted by the AI service you choose with your own key (OpenAI or Google Gemini) or by a model on your own computer (Ollama and ComfyUI, among others), and puts it on your desktop. It remembers everything it has made, so it doesn't repeat itself, and it learns what you like.

It's native on every platform, MIT-licensed and open source, with **no accounts, no telemetry and no AutoPaper server.**

> [!NOTE]
> **v0.1.2 is out for macOS, Windows and Linux.** [Download it](https://github.com/msitarzewski/AutoPaper/releases/latest) or see [Install](#install). It makes wallpapers that suit your light or dark mode, brings AutoPaper's windows forward from the Dock and menu, and tidies the Mac's Moods list. (0.1.1 added a Console for every wallpaper run, clearer budget messages, and longer waits for local models.) Bug reports and ideas are very welcome as [issues](https://github.com/msitarzewski/AutoPaper/issues/new/choose).

<p align="center">
  <img src="site/static/examples/rain-ruins-960.jpg" width="820" alt="Soft rain falls on a ruined stone colonnade standing in still water at night, lit in deep blues by a moon behind thin cloud.">
</p>
<p align="center"><sub><b>“Rain over the drowned colonnade”</b>, from five Must keywords: rain, ruins, peaceful, night, blue. Surprise 0.2.</sub></p>

## How it works

1. **You give it keywords.** Each one is a **Must** (in every wallpaper), a **Maybe** (in some) or an **Avoid** (never, and never even mentioned to the painter). **Surprise** sets how much freedom it has, from faithful to wild.
2. **It writes a scene.** A language model turns the keywords into four concepts (a place, weather, light, a palette, a title), and AutoPaper keeps the one least like anything you've had and most like what you enjoy.
3. **It's painted by the service you choose**, at the largest size it supports, with a progress ring and the time left, which it learns from your own computer's history.
4. **It goes on your desktop**, sized for each display, and into your history and memory.

A new wallpaper comes every hour, every 3, 6 or 12 hours, every day or every week, or only when you ask. **New Wallpaper Now** (⌘R / Ctrl+R) makes one straight away.

## Same keywords, different worlds

Every one of these came from a handful of keywords, composed the way AutoPaper composes and painted locally with ComfyUI.

<table>
  <tr>
    <td width="50%"><img src="site/static/examples/rain-ruins-wild-960.jpg" alt="Rain falls on the ruins of a great domed observatory, overgrown and glowing with blue moss and flowers under drifting night clouds, with a few luminous moths."></td>
    <td width="50%"><img src="site/static/examples/snow-lanterns-960.jpg" alt="A path of small paper lanterns winds through a snowy pine forest at blue hour, warm light on fresh snow."></td>
  </tr>
  <tr>
    <td><sub><b>The observatory that grew moss.</b> The same five keywords as above, with Surprise turned up to 0.9: AutoPaper added the observatory, the glowing moss and the moths.</sub></td>
    <td><sub><b>Lanterns through the snowy pines.</b> Must: forest, snow. Maybe: lanterns, warm. Avoid: neon.</sub></td>
  </tr>
  <tr>
    <td><img src="site/static/examples/echo-then-960.jpg" alt="A quiet black ocean at night, with distant silver structures standing far out at sea in thin fog."></td>
    <td><img src="site/static/examples/echo-now-960.jpg" alt="The same ocean after a storm: lingering swells, broken clouds, and the distant structures, now weathered, catching an early sunrise."></td>
  </tr>
  <tr>
    <td><sub><b>Black ocean, silver structures.</b> Must: ocean, night, quiet. Maybe: fog, silver.</sub></td>
    <td><sub><b>…and its echo, months later.</b> The same idea seen again: after a storm, at sunrise, the structures weathered. Never the old image itself.</sub></td>
  </tr>
  <tr>
    <td><img src="site/static/examples/desert-glass-960.jpg" alt="A small glass pavilion rests on pale dunes at dawn, scattering faint rainbows across the sand."></td>
    <td><img src="site/static/examples/rooftops-spring-960.jpg" alt="A watercolour of old hillside rooftops in spring, cherry blossom between the houses and kites drifting in a pale sky."></td>
  </tr>
  <tr>
    <td><sub><b>A glass pavilion at first light.</b> Must: desert, dawn, calm. Maybe: glass. Avoid: people.</sub></td>
    <td><sub><b>Kites over the spring rooftops.</b> Must: rooftops, spring. Maybe: mist, city. Surprise 0.6.</sub></td>
  </tr>
</table>

<sub>These examples were made before the apps could make their own, and are shared under CC BY 4.0. The models and seeds are on the <a href="https://msitarzewski.github.io/AutoPaper/credits.html">Credits page</a>.</sub>

## What makes it different

- **Moods.** Named sets of keywords, each with its own Surprise: *Rainy beach*, *Night city*, *Winter forest*. One is current, and switching never makes a wallpaper by itself. A **Your Moods** summary shows what each one has made. On a Mac, each mood has a **Use** button (**In Use** for the current one) in its header.
- **Light or dark, to match.** *Match my appearance* (on by default, in Settings → General) asks for wallpapers that suit your computer's light or dark mode: bright and airy on a light desktop, deep and moody on a dark one. Your keywords still come first.
- **A writer that needs no key.** On a Mac with Apple Intelligence, *On this Mac* writes ideas with Apple's on-device model: free, private, offline. AutoPaper asks it for fewer ideas with shorter instructions and fixes what it gets wrong, such as a keyword left out. Painting still needs a painter (or Demo's gradients). [Foundry Local](https://msitarzewski.github.io/AutoPaper/help.html#foundry) works the same way on Windows through *OpenAI-compatible*.
- **Memory.** Each new idea is compared, on your computer, with everything from your quiet period (a month to two years). Ideas too close to one you've had are asked for again, so the same scene doesn't come back next week.
- **Echoes.** Once the quiet period has passed, an old idea can return seen differently (other weather, another hour or season, years of decay), linked to the original.
- **Taste.** Like or Dislike any wallpaper and AutoPaper leans towards what you like. Taste is a hint; Avoid is the rule.
- **History.** Everything it has made, as a grid or a gallery, with what wrote it, what painted it, at what size and what it cost. A monthly **budget** keeps paid providers in check.
- **Your own wallpaper, kept safe.** On a Mac, AutoPaper's wallpapers sit over yours, which is never touched: quit, and yours is there exactly as it was. On Windows and Linux it keeps a copy of yours (and on Windows your lock screen picture) and puts it back. **Pause** keeps the current one up.
- **Problems point to the fix.** A missing key or a keyword the model keeps ignoring is said once, as a link to the setting that fixes it, never as a paragraph of directions.
- **Local all the way, if you like.** With Ollama writing and ComfyUI painting, nothing leaves your network. Bundled ComfyUI workflows cover Z-Image Turbo, Krea 2 Turbo and Qwen-Image 2.1, or bring your own.

The website explains each of these in full: [msitarzewski.github.io/AutoPaper](https://msitarzewski.github.io/AutoPaper/).

## Three native apps, one core

No web views and no cross-platform UI kit. Each app follows its own platform's guidelines and uses the system's own controls, while the thinking (composing, memory, echoes, taste, moods, the schedule and the budget) lives in one shared Rust core, so it behaves the same everywhere.

| | macOS | Windows | Linux |
|---|---|---|---|
| Built with | SwiftUI and AppKit, Liquid Glass | WinUI 3 (Windows App SDK, .NET 10) | GTK 4 and libadwaita (Rust) |
| Lives in | A menu in the menu bar | The notification area | The background, with notifications (a tray icon on KDE and other desktops) |
| Keys kept in | Keychain | Credential Manager | Secret Service |
| Updates | Sparkle | App Installer or winget | Flatpak, from AutoPaper's own repository |
| Needs | macOS 26 or later, Apple silicon or Intel | Windows 11, x64 or Arm | Flatpak (its GNOME runtime brings GTK and libadwaita); built from source, GTK 4.22 and libadwaita 1.9 or later |

```
             ┌──────────────── autopaper-core (Rust) ────────────────┐
             │  composer · memory & echoes · taste · moods · budget  │
             │ schedule · SQLite store · providers · one HTTP client │
             └───────────────────────────────────────────────────────┘
                  │                  │                  │
            UniFFI (Swift)      UniFFI (C#)        Rust crate
                  │                  │                  │
           SwiftUI + AppKit       WinUI 3       GTK 4 + libadwaita
                macOS             Windows             Linux
```

## Install

AutoPaper v0.1.0 is available for macOS, Windows and Linux.

### macOS

1. Download the `.dmg` from the [latest release](https://github.com/msitarzewski/AutoPaper/releases/latest). It's signed with a Developer ID and notarized by Apple, so it opens without warnings.
2. Open it and drag **AutoPaper** into **Applications**, then open it from there. AutoPaper lives in the menu bar.

It updates itself with [Sparkle](https://sparkle-project.org): on its second launch it asks whether to check automatically (about once a day), **Settings → About** changes that answer, and **Check for Updates…** checks now. Every update is verified with an EdDSA signature before it's installed.

### Windows

Open [`AutoPaper.appinstaller`](https://msitarzewski.com/app-updates/autopaper/AutoPaper.appinstaller): Windows' App Installer shows who signed AutoPaper (Michael Sitarzewski, verified by Microsoft), installs it from msitarzewski.com, the author's own server, and keeps it up to date from there. The signed MSIX bundle (x64 and Arm) is also attached to the [release](https://github.com/msitarzewski/AutoPaper/releases/latest).

`winget install msitarzewski.AutoPaper` will work once Microsoft has reviewed AutoPaper's winget listing, which can take a few days.

### Linux

Open [`AutoPaper.flatpakref`](https://msitarzewski.com/app-updates/autopaper/AutoPaper.flatpakref): GNOME Software or KDE Discover adds AutoPaper's repository (the remote `autopaper`, on msitarzewski.com, the author's own server) and installs it. From a terminal:

```sh
flatpak install --from https://msitarzewski.com/app-updates/autopaper/AutoPaper.flatpakref
```

Ubuntu ships neither Flatpak nor a store that opens that file: run `sudo apt install flatpak` first, then the command above.

Each [release](https://github.com/msitarzewski/AutoPaper/releases/latest) also has a `.flatpak` file; `flatpak install --user <file>.flatpak` sets up the same repository. The GNOME runtime AutoPaper needs comes from Flathub's runtime repository, which Flatpak sets up if you don't have it. Either way, Flatpak keeps AutoPaper up to date (`flatpak update`, GNOME Software or Discover); AutoPaper never checks for updates itself.

## Providers and keys

| Provider | Writes | Paints | Key |
|---|---|---|---|
| OpenAI | Yes | Yes | Your own API key |
| Google Gemini | Yes | Yes (needs a paid project) | Your own API key |
| Ollama | Yes | No | None: it runs on your computer or network |
| LM Studio and other OpenAI-compatible servers | Yes | Yes, if the server can | Optional |
| ComfyUI | No | Yes, with bundled workflows for Z-Image Turbo, Krea 2 Turbo and Qwen-Image 2.1, or your own | None |
| Demo | Simple ideas | Soft gradients | None, and no network: try the app without AI |

Keys are kept in your system's secure store, sent only to the service they belong to, and never logged or shown in an error. New to local models? [Brew Browser](https://brew-browser.zerologic.com) has bundles that install Ollama and ComfyUI on macOS and Linux.

## Privacy

The full account is in **[PRIVACY.md](./PRIVACY.md)** (who learns what, what's stored) and **[NETWORK.md](./NETWORK.md)** (every host, request and limit). In brief:

- No telemetry, analytics, accounts or AutoPaper server. Out of the box AutoPaper uses Demo and sends nothing.
- To write a scene, the provider you chose gets the current mood's keywords and Surprise, short descriptions of recent wallpapers, a few things you've liked or disliked, your language, and whether your computer is in light or dark mode (while *Match my appearance* is on). To paint it, the finished prompt, the size and the quality.
- Your images, history, ratings, spending and settings stay on your computer. Memory runs on your computer with a small bundled model. With local models, nothing leaves your network.
- Updates are checked by Sparkle on a Mac (the feed on this project's GitHub Pages, no system profile), by Windows' App Installer (from msitarzewski.com, the author's own server, which keeps no access logs) or winget, and by Flatpak on Linux (from the same server).

## Accessibility

AutoPaper aims for WCAG 2.2 AA as W3C's [WCAG2ICT](https://www.w3.org/TR/wcag2ict-22/) applies it to desktop software, and for each platform's own accessibility guidelines. Every control and image has a spoken name (each keyword says its weight, "rain, Must"), every wallpaper is described in words, nothing relies on colour alone, and the effect while painting holds still with reduced motion. Each app has been checked through its system's accessibility interface (the macOS accessibility API, UI Automation, AT-SPI); full passes with VoiceOver, Narrator and Orca are still to come, and reports are welcome as [issues](https://github.com/msitarzewski/AutoPaper/issues/new/choose).

## Build from source

Everything needs [rustup](https://rustup.rs) with the stable toolchain. The embedding model memory uses is downloaded separately and never committed; without it, memory falls back to a simpler comparison and Settings → Memory says so:

```sh
git clone https://github.com/msitarzewski/AutoPaper
cd AutoPaper
scripts/fetch-model.sh           # BAAI/bge-small-en-v1.5 into models/ (git-ignored), checked by SHA-256
```

### The core and the developer CLI

```sh
cargo test                                           # the core and the CLI, no network needed
cargo run -p autopaper-cli -- --help                 # `autopaper`: the same core from the terminal
cargo run -p autopaper-cli -- simulate --days 30     # the agent over 30 simulated days, with Demo providers
```

The CLI reads keys from a `.env` file: copy `.env.example` to `.env` and fill in what you have. `.env` is git-ignored; never commit keys.

### macOS

Needs macOS 26 and Xcode 26 or later, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`), and the `aarch64-apple-darwin` and `x86_64-apple-darwin` Rust targets.

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/build-xcframework.sh     # the core as a Swift package in apps/macos/Generated/ (rerun after core changes)
cd apps/macos && xcodegen generate && open AutoPaper.xcodeproj    # run the AutoPaper scheme
```

`scripts/macos-install.sh [--open]` builds Release and installs it to `/Applications`. Without an Apple Developer team, build unsigned (keys then can't be saved in the Keychain):

```sh
xcodebuild -project apps/macos/AutoPaper.xcodeproj -scheme AutoPaper -derivedDataPath build CODE_SIGNING_ALLOWED=NO build
```

### Windows

In Windows 11, from a copy of the repository on a local disk. Needs the .NET 10 SDK; Visual Studio 2022 Build Tools with the C++ workload and a Windows 11 SDK (plus the "C++ Clang Compiler for Windows" component for ARM64); the `x86_64-pc-windows-msvc` and/or `aarch64-pc-windows-msvc` Rust targets; and the C# binding generator that matches the core's UniFFI:

```powershell
cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.11.0+v0.31.0
scripts\build-core-windows.ps1 -Arch x64           # autopaper_core.dll + C# bindings into apps\windows\Generated
scripts\windows-app.ps1 -Smoke -Test -Run -Arch x64  # bindings check, tests, then run the app with package identity
```

`-Run` needs Microsoft's winapp CLI (`winget install Microsoft.WinAppCli`); `-Pack` makes a test-signed MSIX. `scripts\windows-app.ps1 -?` lists the options.

### Linux

Needs GTK 4.22 and libadwaita 1.9 or later, with their development files: Ubuntu 26.04 `libgtk-4-dev libadwaita-1-dev libglib2.0-bin`, Fedora 44 `gtk4-devel libadwaita-devel`.

```sh
cargo run -p autopaper-linux                 # build and run from target/
scripts/linux-install.sh                     # or install to ~/.local, so the desktop knows it (desktop entry, icons, schema)
```

The Flatpak builds from `packaging/flatpak/` with `flatpak run org.flatpak.Builder --force-clean --user --install builddir packaging/flatpak/io.github.msitarzewski.AutoPaper.yml` (needs `org.flatpak.Builder` and the GNOME SDK, from Flathub's runtime repository).

### The website

`site/` is built by `scripts/build_site.py` (with `PRIVACY.md` and `NETWORK.md`, rendered by pandoc) and published by CI on every push to `main`. Preview it with `python3 scripts/build_site.py && python3 -m http.server -d _site 8090`.

## Architecture

```
core/                 The shared Rust core (autopaper-core): composing, memory and echoes, taste, moods, the schedule,
                      the budget, storage (SQLite), the providers (core/src/providers/) and the one HTTP client
                      (core/src/net.rs). UniFFI exports it to Swift and C#.
cli/                  autopaper, a developer CLI over the same core (keys from .env)
uniffi-bindgen/       UniFFI's binding generators, at the core's UniFFI version (Swift)
apps/macos/           The Mac app: SwiftUI and AppKit, an XcodeGen project.yml (the .xcodeproj is generated)
apps/windows/         The Windows app: WinUI 3 and C#, its tests, and a bindings smoke test
apps/linux/           The Linux app: GTK 4 and libadwaita in Rust, with its desktop entry, AppStream metainfo,
                      GSettings schema and icons
packaging/            The Flatpak manifest (the release script adds the winget manifests)
docs/app-spec.md      What every app does, so the three match in behaviour while each fits its platform
docs/research/        Research notes behind the technical choices
docs/icon/            The app icon's sources and palette build
site/                 The website (GitHub Pages): page fragments, layout, static assets, the Mac's update feed
scripts/              Builds, installs, the website, third-party notices, and the release scripts
memory-bank/          The project's working notes (AGENT-ZERO's Memory Bank)
```

## Contributing: PRs welcome

AutoPaper is MIT-licensed open source, and pull requests are welcome, from a typo fix to a new provider. Start with [CONTRIBUTING.md](./CONTRIBUTING.md): how the pieces fit, how to run the tests, and how a change to the shared core reaches all three apps. [`docs/app-spec.md`](./docs/app-spec.md) says what every app does, so a change on one platform can find its counterpart on the others.

Good places to start:

- **Providers.** A new writer or painter is one file in `core/src/providers/` plus a registry entry, and every app gets it.
- **Accessibility.** Hands-on passes with VoiceOver, Narrator and Orca, and reports of anything that gets in the way.
- **Desktops.** KDE Plasma, Cinnamon, Xfce and multi-monitor setups we haven't tried.
- **Translations.** The Windows app keeps its words in resource files already; the Mac app (String Catalogs) and the Linux app (gettext) need the same before AutoPaper can speak more languages.
- **ComfyUI workflows.** Tested workflows for more local models, with their largest safe sizes.

Found a bug or have an idea? [Open an issue](https://github.com/msitarzewski/AutoPaper/issues/new/choose). Security or privacy problems go by email, as described in [SECURITY.md](./SECURITY.md), not in public issues.

## Roadmap

- **Atmosphere:** let the wallpaper set the mood beyond the picture: your accent colour, switching your computer between light and dark to suit it, and gentle sounds.
- **More built-in writers:** Foundry Local and Windows' own models as one-click choices on Windows (today Foundry Local works as an OpenAI-compatible server).
- **Sign in with ChatGPT**, so a ChatGPT plan can do the writing without an API key.
- **Image Playground** on a Mac, to paint a wallpaper by hand when you want to.

## Built with Agency Agents

AutoPaper was designed and built by an agent team from **[Agency Agents](https://github.com/msitarzewski/agency-agents)**, by the creator of Agency Agents: the open library of specialist AI agent personas for coding assistants. Software Architect and Backend Architect shaped the shared core; AI Engineer and Prompt Engineer tuned how it composes and remembers; a Senior Developer took each platform; Frontend Developer built the website; Security Engineer, Accessibility Auditor and Code Reviewer audited every round; and DevOps Automator got it ready to ship. They worked in Claude Code in the terminal, running Claude Opus 5.5, following the [AGENT-ZERO](https://github.com/msitarzewski/AGENT-ZERO) workflow, with a human deciding what got built.

The [Agency Agents app](https://agencyagents.app) installs the same agents into the AI coding tools you use.

## Other projects

Also by [Michael Sitarzewski](https://github.com/msitarzewski), all open source:

- [**AudioPaper**](https://msitarzewski.github.io/AudioPaper/): your Mac's desktop, set to the music you're playing. AutoPaper's older sibling: the desktop overlay that keeps your own wallpaper safe, the website and the release pipeline all started there.
- [**Agency Agents**](https://github.com/msitarzewski/agency-agents): a library of specialized AI agent personas for coding assistants, and the team that built AutoPaper.
- [**Agency Agents app**](https://agencyagents.app): a small native app for browsing and installing those agents across the AI tools you use.
- [**Brew Browser**](https://brew-browser.zerologic.com): a native GUI for Homebrew, with bundles for local AI.
- [**Anomalous**](https://anomalous.bot): system anomaly detection for macOS, with on-device judgement about what's worth your attention.
- [**AGENT-ZERO**](https://github.com/msitarzewski/AGENT-ZERO): the AGENTS.md workflow this project is built with.
- [**BEDROCK**](https://github.com/msitarzewski/bedrock): a free library about how a person becomes who they are.

## Support

<p align="center">
  <a href="https://github.com/sponsors/msitarzewski"><img src="https://img.shields.io/badge/♥_Sponsor_AutoPaper-on_GitHub-EC4899?style=for-the-badge&logo=githubsponsors&logoColor=white" alt="Sponsor AutoPaper on GitHub"></a>
</p>

AutoPaper is free and MIT-licensed, with no paid tier. If it makes your desk nicer, you can [sponsor its development on GitHub](https://github.com/sponsors/msitarzewski). Sponsorship is purely a thank-you, and it helps pay for the AI that builds and tests it.

## Credits and licence

Made by Michael Sitarzewski, working with an AI coding assistant, Claude. The idea comes from a Reddit post titled "Desktop Wallpaper Agent", whose author described it and said they wouldn't build it themselves; the line "Tell your computer what you'd like to see" is the post's own.

[MIT](./LICENSE) © 2026 Michael Sitarzewski. The icon's sparkles are from Heroicons (MIT); everything AutoPaper includes or is built with is listed in [THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md). The example wallpapers are CC BY 4.0.
