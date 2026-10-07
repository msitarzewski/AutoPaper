# Security Policy

Thanks for taking the time to look. Reports, large or small, are welcome.

## Supported versions

AutoPaper is an early-stage project. Only the latest release receives security fixes, on macOS, Windows and Linux alike.

## Reporting a vulnerability

Report it privately, either way:

- through GitHub: **[Report a vulnerability](https://github.com/msitarzewski/AutoPaper/security/advisories/new)** on this repository's Security tab, or
- by email to **msitarzewski@gmail.com**.

Please include:

- a clear description of the issue and the impact you believe it has
- steps to reproduce, or a proof of concept if you have one
- the AutoPaper version (or commit) and the system you tested on
- your name or handle, if you'd like credit (optional)

Please do **not** open a public GitHub issue for security reports. Privacy concerns are welcome through the same channels; AutoPaper's data flows are documented in [PRIVACY.md](./PRIVACY.md) and [NETWORK.md](./NETWORK.md).

## Response time

This is a side project, so responses are best-effort:

- **Acknowledgement:** within 7 days
- **Initial assessment:** within 14 days
- **Fix or mitigation plan:** within 30 days for high or critical findings

## Scope

**In scope:**

- Credential handling: API keys in the Keychain, Windows Credential Manager or the Secret Service, and anything that could leak them to logs, disk, error messages or any host other than their own service
- The network rules in `core/src/net.rs`: hosted providers only at their API host over `https`, plain `http` only to your own computer or local network, same-host redirects that drop the key on a change of scheme or port, and the time and size limits
- Parsing untrusted input: providers' answers (JSON and images), a local server's answers, and ComfyUI workflows, including one you choose yourself
- Wallpaper handling that could overwrite or delete files outside AutoPaper's own data folder, including putting your own wallpaper back
- Each app's sandbox and permissions: the macOS App Sandbox and entitlements, the Windows package's capabilities, and the Flatpak's permissions, where any is broader than the app needs
- Updates: Sparkle's EdDSA signature check on macOS, the signed MSIX and its App Installer feed on Windows, and the Flatpak

**Out of scope:**

- What an image or language model writes or paints (report that to the provider, or to the model's makers)
- Rate limits, availability or billing of the providers you use
- A server you point AutoPaper at that is malicious but stays within the rules above
- Issues that require an already-compromised account or computer
