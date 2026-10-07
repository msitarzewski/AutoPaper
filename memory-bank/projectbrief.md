# Project Brief — AutoPaper

A native desktop app for macOS, Windows and Linux that keeps making new wallpapers you're unlikely to
ever see twice. You give it a few keywords ("rain • ruins • peaceful • night • blue") instead of an
image prompt; it composes a coherent scene, generates it with the AI provider you choose (your own API
key), sets it as your wallpaper and lock screen, and remembers everything it has made.

Origin: a Reddit post ("Desktop Wallpaper Agent"), handed to the user on 2026-10-05. The post's author
isn't building it.

## Goals (v1)
- **Keywords, not prompts.** Each keyword is marked **Must**, **Maybe** or **Avoid**.
- **Surprise** sets how much freedom the agent has, from faithful to wild.
- **Like / Dislike** each result; the agent learns what you enjoy.
- **Memory.** Every concept is remembered; the agent avoids making anything too similar for months or
  years (a "quiet period" you choose).
- **Echoes.** Once an idea's quiet period has passed, it may return as a new interpretation of the old
  concept (same kind of place, different weather/hour/season/age), never the old image, and linked to it.
- **History** of everything made, with echo lineage.
- **Wallpaper and lock screen**, on every display, where the OS allows it.
- **Bring your own keys**: OpenAI, Google (Gemini/Imagen), and local / OpenAI-compatible servers
  (Ollama, LM Studio, ComfyUI). Keys live in each platform's secure store.
- **Native on each platform** — macOS (SwiftUI/AppKit, Apple HIG), Windows 11 (WinUI 3, Fluent),
  Linux (GTK4 + libadwaita, GNOME HIG) — with one shared Rust core holding the agent's logic.
- **Accessible**: WCAG 2.2 AA as WCAG2ICT applies it to desktop software, verified with each platform's
  accessibility API and screen reader (VoiceOver, Narrator, Orca).
- **Private**: no accounts, no telemetry, no AutoPaper server. Only keywords and composed prompts go to
  the provider you pick.

## Later (v2: "Atmosphere")
A desktop atmosphere built around each wallpaper: accent colour, light/dark mode, and sounds, wherever
each OS permits it.

## Reference app
AudioPaper (`~/Software/AudioPaper`) is the UX and engineering template on macOS: menu bar menu (not a
popover), Settings panes sized to content, Keychain fields that auto-save, plugin protocols, untrusted-
response hardening, PRIVACY.md + NETWORK.md kept true, AX-API accessibility audits, Sparkle updates.
