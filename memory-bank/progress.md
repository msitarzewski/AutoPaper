# Progress

## Done
- 2026-10-05: Plan approved (shared Rust core + native SwiftUI / WinUI 3 / GTK4 apps). Repo scaffolded: licence, ignore
  rules, secret scanning config, memory bank, core design (`systemPatterns.md`). Toolchains proven on all three platforms
  (Swift/C# UniFFI round trips; WinUI MSIX sideload; GTK4 on Scratch). Website built (AudioPaper style, red, Lighthouse 100s),
  7 ComfyUI examples, Brew Browser links.
- 2026-10-05: **Core built** (6 module agents → integrator → 3 reviewers → fixer): 375 tests, clippy -D warnings clean, no stubs.
  Real-model calibration: threshold 0.79, echo band 0.72–0.93. `autopaper simulate --days 1100`: 107 echoes, none outside the band.
- 2026-10-06: **Phase 2** apps built, audited and fixed. **Round 3**: moods, performance history + live progress, learned timeouts,
  typed `PaintingFailed`, Demo slow mode (core 442 tests); macOS 22 findings fixed (+ overlay mode, footer, moods dashboard,
  gallery, provenance, reload/stop, Pause keeps wallpaper); Windows 13/13; Linux 21/24. Errors as links; the "working" effect;
  per-model ComfyUI workflows + picker; macOS test VM (tart); Gemini model-list and thinking-level fixes.
- 2026-10-06: macOS **three-column Moods** (sidebar | list | detail, reload/stop in the detail toolbar); website with all pages
  (Help, Shortcuts, Reference, Privacy, Network); PRIVACY.md/NETWORK.md written from the code.
- 2026-10-06: **Gemini fixes** from the user's "failed every recent session" report: painting moved from `/v1` to `/v1beta`
  (Nano Banana 2.1 and preview models only exist there); error redaction no longer hides model names; writer retries now say what
  was wrong and `KeywordNotFollowed` names the keyword + mood (core 445+ tests).
- 2026-10-06: **Windows/Linux parity** with the Mac UI (agents): Moods group, summary + 30-day chart, Grid|Gallery, provenance,
  reload/stop (Ctrl+R/F5), Pause keeps, Windows lock-screen record/restore; Linux sidebar layout. Mac: Sparkle 2.10.0,
  `scripts/release.sh`, KeywordNotFollowed link.
- 2026-10-06: **Published** the repo (first commit, then CI fixes: gitleaks pin, colour codes in rustc's link list, Windows path
  separators, a Swift 6.3 IRGen crash on method-reference Binding setters, a slow-to-type-check expression). CI green on 7 jobs.
- 2026-10-06/07: **Self-hosted updates** on pipx (no Flathub): own signed Flatpak repo, App Installer feed + bundles; scripts
  `pipx-app-updates.sh`, `publish-flatpak.sh`, `publish-windows.sh`, `flatpak-repo-key.sh`; CI `flatpak.yml`.
- 2026-10-07: **Azure Artifact Signing** set up (account, identity validation, PublicTrust profile); Windows bundle signed and
  verified; **v0.1.0 released** on all three platforms; real-key Sparkle update test passed; site shows the real Download buttons.

## Next
- User's UI tweaks (pending). Optional: winget submission; Linux Flatpak "pick my picture" restore; full VoiceOver/Narrator/Orca
  passes; CI `cargo fmt` check after a one-off format.
- Backlog (user go-ahead needed): "On this Mac" writer (Foundation Models), ChatGPT-plan writer, Foundry Local preset,
  Image Playground sheet; verify Nano Banana 2.1 pricing / Gemini Omni image output.
- v2 "Atmosphere": accent, light/dark, sounds where each OS allows.
