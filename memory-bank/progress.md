# Progress

## Done
- 2026-10-05: Plan approved (shared Rust core + native SwiftUI / WinUI 3 / GTK4 apps). Repo scaffolded on
  branch `build/v1`: licence, ignore rules, secret scanning config, memory bank, core design
  (`systemPatterns.md`).

- 2026-10-05: Toolchains proven on all three platforms (Swift/C# UniFFI round trips; WinUI MSIX sideload; GTK4 on
  Scratch). Website built (AudioPaper style, red, Lighthouse 100s), 7 ComfyUI examples, Brew Browser links.
- 2026-10-05: **Core built** (workflow: 6 module agents → integrator → 3 reviewers → fixer, then my own fixes):
  375 tests pass, clippy -D warnings clean, no stubs. Real-model calibration: threshold 0.79, echo band 0.72–0.93.
  `autopaper simulate --days 1100`: 107 echoes, none outside the band, no repeats in the quiet period. 23 review
  findings: 21 fixed with regression tests; the OpenAI-compatible key is now tied to its server's origin
  (`secret_account_for`); C# bindings not yet generated (needs the Windows VM / Docker). Composer rejects prompts that
  name the computer (Krea 2 lesson).

## Next (phases; each ends at an approval gate)
0. ~~Scaffold + toolchains~~ done
1. ~~Core + CLI + tests~~ done (approved 2026-10-05)
2. Phase 2 (apps built, audited, fixed) done 2026-10-06; extras since: installed macOS build (scripts/macos-install.sh),
   errors-as-links, the "working" effect, per-model ComfyUI workflows + workflow picker, macOS test VM (tart).
3. **Round 3 done** (2026-10-06): core 442 tests (moods, performance history + live progress, learned timeouts,
   next_start, typed PaintingFailed, Demo slow mode); macOS 22 findings → fixed (+ overlay mode, footer, moods dashboard,
   gallery, provenance, reload/stop, Pause keeps wallpaper); Windows 13/13 fixed; Linux 21/24 fixed (1 platform bug in
   libadwaita AT-SPI, 1 needs Flatpak packaging). Findings/outcomes in .scratch/round3/r3-*.json.
4. Next: true three-column Moods on macOS (in progress), phase 4 packaging (notarized DMG + Sparkle, MSIX, Flatpak),
   PRIVACY.md/NETWORK.md, Windows/Linux parity for the latest Mac UI (disclosure, dashboard, gallery, footer).
2. Native apps in parallel: macOS, Windows, Linux
3. Audits: accessibility per platform, security, code review
4. Packaging: notarized DMG + Sparkle, MSIX, Flatpak; README, PRIVACY.md, NETWORK.md, help
5. v2 "Atmosphere": accent, light/dark, sounds where each OS allows
