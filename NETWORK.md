# Network

Every network request AutoPaper makes: host, trigger, what's sent, and the limits. For what this means for your privacy, see [PRIVACY.md](./PRIVACY.md).

This page describes AutoPaper 0.1.0, and changes with the code.

All requests for wallpapers are made by AutoPaper's shared core, the same code on macOS, Windows and Linux, through one HTTP client (`core/src/net.rs`: reqwest, with rustls and your system's own certificate checks). The apps themselves make no requests, apart from the Mac app's update check ([below](#checking-for-updates)): otherwise they only open links in your browser when you click them. The client keeps **no cookies and no cache, and sends no Referer**. Every request carries the User-Agent `AutoPaper/<version> (<system>/<version> AutoPaper/<version>; +https://github.com/msitarzewski/AutoPaper)`, for example `AutoPaper/0.1.0 (macOS/26.6 AutoPaper/0.1.0; +https://github.com/msitarzewski/AutoPaper)`. Outgoing connections only; AutoPaper opens no listening sockets.

The same client enforces:

- **Hosted providers only at their API host**: `https` to `api.openai.com` or `generativelanguage.googleapis.com`, by name, on the standard port. No other host, no IP addresses.
- **Your own servers** (Ollama, OpenAI-compatible servers such as LM Studio, ComfyUI): `https` to any address you give, but plain `http` only to your own computer or local network: `localhost` and loopback addresses, names ending in `.local`, the private ranges `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` and `fc00::/7`, and link-local addresses. Anything else over `http` is refused before it's sent.
- **Redirects** are followed by AutoPaper itself, at most 3, only to the same host, never from `https` to `http`. If the scheme or port changes, the key is removed from the request first.
- **Keys go in headers** (`Authorization`, or `x-goog-api-key` for Google), never in a URL, and only to the service they belong to.
- **Proxies:** your system's proxy settings apply to hosts on the internet; addresses on your own computer or network are always reached directly.
- **Limits:** 10 seconds to connect, then each request's own time limit (below) for everything, redirects and the answer included. An answer larger than its limit is abandoned as it arrives.
- **Images** must be PNG, JPEG or WebP, judged from the file itself, not its label, and at most 50 MB, 64 megapixels and 16,384 px on a side, checked before they're used.
- **Certificates** are checked by your system; nothing in AutoPaper relaxes that.

## When requests happen

| Moment | Network? |
|---|---|
| Installing, first launch | **None.** Writing and painting both start on **Demo**, which works without any network |
| Using Demo | **None** |
| A new wallpaper is due on your schedule (checked again when your computer wakes or you unlock it) | One writing request, then one painting request, to the providers you chose (below). A scheduled wallpaper may start a little early, by its estimated time, so it's ready when due |
| **New Wallpaper Now**, **Make an Echo**, or **Dislike** with *Replace wallpapers I dislike* on | The same |
| The **New Wallpaper** action in Siri or Shortcuts (macOS) | The same |
| The month's budget is spent | **None** for paid providers: a wallpaper you liked comes back instead, or the current one stays |
| Showing a past wallpaper again, rating, switching moods, editing keywords, browsing History, the memory check | **None** |
| Settings → Providers opens, or you change a provider, address, workflow or key; **Refresh** and **Test** | The chosen providers' model lists (below) |
| Clicking **Get a key**, a Brew Browser button, or an install link for Ollama or ComfyUI | Opens the page in your browser, or Brew Browser itself (`brewbrowser://bundle/local-llm` or `brewbrowser://bundle/image-gen`); AutoPaper makes no request |
| Checking for updates | On a Mac, the update feed ([below](#checking-for-updates)): about once a day if you allowed automatic checks, or when you choose **Check for Updates…**. On Windows and Linux, AutoPaper makes none: the system's own installer does it |

**Per wallpaper, typically:** one writing request and one painting request. When the ideas are too close to ones you've had, AutoPaper asks again, at most twice; if a provider declines to write or paint an idea, it asks once more for a gentler one.

## OpenAI

`api.openai.com`, with your key as `Authorization: Bearer …`.

| Request | When | Sends | Time limit, size limit |
|---|---|---|---|
| `POST /v1/responses` | Writing a scene | The model (`gpt-6-luna` unless you choose another), `store: false`, AutoPaper's instructions and the message described in [PRIVACY.md](./PRIVACY.md#whats-sent), the answer's JSON schema, and the reasoning effort or temperature | 60 s, 4 MB |
| `POST /v1/images/generations` | Painting | The model (`gpt-image-2.5-flare` unless you choose another), the prompt, the size, the quality (`medium` for Standard, `high` for High), `output_format: jpeg`, `background: opaque`, `n: 1` | 240 s, 72 MB |
| `GET /v1/models` | Model list, **Test** (this is also how a key is checked) | — | 60 s, 4 MB |

The image comes back inside the answer; nothing is downloaded from anywhere else. No `user` identifier is sent.

## Google Gemini

`generativelanguage.googleapis.com`, with your key as `x-goog-api-key`.

| Request | When | Sends | Time limit, size limit |
|---|---|---|---|
| `POST /v1beta/models/{model}:generateContent` | Writing a scene (`gemini-3.5-flash-lite` unless you choose another) | AutoPaper's instructions and the message, the answer's schema, and the thinking level or temperature. If a model doesn't accept the thinking level, the request is sent once more without it | 60 s, 4 MB |
| `POST /v1beta/models/{model}:generateContent` | Painting (`gemini-3.1-flash-image`, Nano Banana 2, unless you choose another) | The prompt, `responseModalities: ["IMAGE"]`, the aspect ratio nearest your display, and the size: `2K` for Standard, `4K` for High | 240 s, 72 MB |
| `GET /v1beta/models?pageSize=1000` | Model list, **Test** | — (at most 10 pages) | 60 s, 4 MB |

The image comes back inside the answer. Google adds SynthID, its invisible watermark, to every image.

## Ollama (writing only)

The address you give; `http://127.0.0.1:11434` unless you change it. No key.

| Request | When | Sends | Time limit, size limit |
|---|---|---|---|
| `POST /api/chat` | Writing a scene | The model, the instructions and message, the answer's schema as `format`, `stream: false`, `think: false`, the temperature | 180 s, 8 MB |
| `GET /api/tags` | Model list, **Test**, and before writing when no model is chosen (to use the first one) | — | 20 s, 4 MB |

## OpenAI-compatible servers (LM Studio, LocalAI, stable-diffusion.cpp and others)

The address you give; there's no default, and `/v1` is added when the address has no path. A key is sent as `Authorization: Bearer …` only if you saved one, and only to that address.

| Request | When | Sends | Time limit, size limit |
|---|---|---|---|
| `POST {address}/chat/completions` | Writing a scene | The model (if chosen), the instructions and message, the answer's schema as `response_format`, the temperature, `stream: false` | 180 s, 8 MB |
| `POST {address}/images/generations` | Painting | The model (if chosen), the prompt, the size (at most 2048 px a side and never more pixels than your display), `n: 1`, `response_format: b64_json` | 300 s, 72 MB |
| `GET` the link the server returned | Only if the server answers with a link instead of the image | — The link is followed only if it's on the server's own address (the same scheme, host and port) | 120 s, 50 MB |
| `GET {address}/models` | Model list, **Test**, and before writing when no model is chosen | — | 20 s, 4 MB |

## ComfyUI (painting only)

The address you give; `http://127.0.0.1:8188` unless you change it. No key.

| Request | When | Sends | Time limit, size limit |
|---|---|---|---|
| WebSocket `/ws?clientId=…` | Opened just before each painting, to hear its progress step by step | A random ID for this job. Plain `ws://` to your own computer or network only, never through a proxy | 5 s to connect, 8 MB a message |
| `POST /prompt` | Painting | The workflow (AutoPaper's for the model you chose, or your own), filled in with the prompt, width, height and a random seed, and the same ID | 30 s, 16 MB |
| `GET /history/{id}` | After 1 s, then every 2 s until the job ends (at once when the progress socket says it has) | — | 30 s, 16 MB |
| `GET /view?filename=…&subfolder=…&type=…` | Once, when the painting is done, for the file ComfyUI named | — | 120 s, 50 MB |
| `POST /api/jobs/{id}/cancel`, or `POST /queue` with `{"delete": [id]}` on older ComfyUI | When you cancel, or when a job stops making progress or passes its time limit | That job's ID. Never `/interrupt`, which would stop whatever is running, someone else's job too | 30 s |
| `GET /object_info` (`/object_info/{node}` with your own workflow) | Model list, **Test**: which of AutoPaper's workflows your ComfyUI has the models and nodes for | — | 30 s, 16 MB |

A ComfyUI painting has no fixed time limit: it can take as long as it keeps making progress. It's given up and stopped when no progress arrives for 3 minutes after a step (or 4 times its slowest step, if that's longer), or 10 minutes while a step-less part of the job runs (loading a model, decoding the picture), or when it passes the longer of 30 minutes and 3 times AutoPaper's estimate for it. Waiting in ComfyUI's queue behind someone else's job doesn't count. The bundled workflows save each painting in ComfyUI's own `output/autopaper/` folder, as any ComfyUI job does; AutoPaper doesn't delete anything there.

## Checking for updates

### macOS: Sparkle

The Mac app uses [Sparkle](https://sparkle-project.org) 2.10.0, not the core's HTTP client. On its second launch AutoPaper asks whether to check automatically; nothing is checked until you choose. **Settings → About** changes the answer later.

| Host | Request | When | Sends |
|---|---|---|---|
| `msitarzewski.github.io` | `GET /AutoPaper/appcast.xml` (the update feed) | About once a day if you allowed automatic checks; whenever you choose **Check for Updates…** | Your IP address and Sparkle's User-Agent, `AutoPaper/<version> Sparkle/<version>`. **No system profile**: Sparkle's optional one is never offered (the app doesn't set `SUEnableSystemProfiling`) and never sent |
| `github.com` → GitHub's download servers | The update's `.zip` | Only when you choose to install an update | — |

Every update is verified against the EdDSA public key built into the app (`SUPublicEDKey`) before it's installed; a feed or file that doesn't match is refused. **Version History** in Sparkle's window opens the release's page on GitHub in your browser.

### Windows: App Installer or winget

AutoPaper makes no request: Windows does.

| Host | Request | When | Sends |
|---|---|---|---|
| `msitarzewski.com` | `GET /app-updates/autopaper/AutoPaper.appinstaller` (the App Installer file) | Installed with App Installer: each time AutoPaper starts, and in the background about every 8 hours (the file's `UpdateSettings`: `OnLaunch` with `HoursBetweenUpdateChecks="0"`, and `AutomaticBackgroundTask`) | Your IP address and App Installer's User-Agent |
| `msitarzewski.com` | The new version's `.msixbundle`, under `/app-updates/autopaper/windows/` | When the file names a newer version | — |
| Microsoft's winget repository | winget's index of packages | Installed with winget: only when you run `winget install` or `winget upgrade` | What winget sends |
| `github.com` → GitHub's download servers | The `.msixbundle` attached to the release | When winget installs or updates AutoPaper | — |

Windows installs a package only if its signature checks out.

### Linux: Flatpak

AutoPaper makes no request: Flatpak does, from AutoPaper's own Flatpak repository (the remote `autopaper`).

| Host | Request | When | Sends |
|---|---|---|---|
| `msitarzewski.com` | `GET /app-updates/autopaper/AutoPaper.flatpakref` | Once, when you install with it | — |
| `msitarzewski.com` | The repository under `/app-updates/autopaper/flatpak/`: its `config` and `summary`, then only the files that changed | Whenever GNOME Software or KDE Discover checks for updates, or you run `flatpak update`; and when installing | Your IP address, Flatpak's User-Agent (with its version), and the headers Flatpak sends to every repository: `Flatpak-Ref` (the app it's fetching) and, when updating, `Flatpak-Upgrade-From` (the version you have) |
| `dl.flathub.org` (Flathub's runtime repository) | The GNOME runtime (`org.gnome.Platform`) AutoPaper runs on, and its updates | When Flatpak installs or updates that runtime | — |

Flatpak installs only what the repository's GPG key signed. A `.flatpak` file from a GitHub release sets up the same repository, so it's updated the same way.

`msitarzewski.com` is the author's own server: HTTPS ends at a front proxy, which forwards each request over a private network to the file server, and neither keeps access logs for msitarzewski.com.

## When something fails

- **No automatic retries** of a request. A scheduled wallpaper that fails is tried again after 10 minutes, then 20, 40, and every 60 minutes after that, or later if the provider asks (its `Retry-After`, at most an hour; a minute if a "too many requests" answer doesn't say).
- A request that runs out of time is abandoned and its connection closed; so is one under way when you choose **Cancel** or **Stop**. A paid request abandoned mid-way may still be billed by the provider.
- Error details are cut to 200 characters, and anything that looks like a key is removed, before they're kept or shown.

## Not contacted

- No AutoPaper server, and no analytics, telemetry or crash-reporting service: none is built in.
- No AutoPaper service: the Mac app's update feed is a static file on this project's GitHub Pages site, and the Windows and Linux update files are static files on msitarzewski.com, fetched by Windows and Flatpak, never by AutoPaper.
- No fonts, content delivery networks or image hosts. OpenAI and Google return images inside their answers; an OpenAI-compatible server's link is followed only to that server itself.
- No model downloads while running. The model memory uses (BAAI's bge-small-en-v1.5, 384 dimensions) is part of the app and runs on your computer. Without it, AutoPaper falls back to a simpler local comparison and says so in Settings → Memory.

**Building from source** (developers only): `scripts/fetch-model.sh` downloads that model's three files from `huggingface.co`, at a pinned revision, over `https` only, and checks each one's SHA-256.

## Verifying it yourself

- **Watch it live:** macOS's Network privacy report, Little Snitch or LuLu; Resource Monitor's Network tab on Windows; OpenSnitch, or `ss -tp`, on Linux. You'll see only the hosts above, and only at the moments above.
- **Read the code:** the rules above are in `core/src/net.rs`; each provider is one file in `core/src/providers/` (`openai.rs`, `google.rs`, `ollama.rs`, `openai_compat.rs`, `comfyui.rs`), with its host, requests and limits at the top.
- **Run it from the terminal:** the developer CLI, `autopaper` (in `cli/`), runs the same core, with keys from a `.env` file: `autopaper compose` makes one writing request and prints the ideas and how they scored; `autopaper generate` makes a whole wallpaper.
