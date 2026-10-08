# Privacy

AutoPaper makes new wallpapers with AI: a language model writes each scene, and an image model paints it. Unless both run on your own computer or network, that means some of what you give AutoPaper leaves your computer. This page says exactly what, to whom, and what stays on your computer. For every request, host and limit, see [NETWORK.md](./NETWORK.md).

This page describes AutoPaper 0.1.0 on macOS, Windows and Linux, and changes with the apps.

## The short version

- **No telemetry, no analytics, no accounts, no AutoPaper server.** AutoPaper talks only to the providers you set up, and only to make wallpapers or to check that a provider works. The one exception is the Mac app's update check, if you allow it (see [Updates](#updates)); on Windows and Linux, updates are your system's job, from the author's own server, which keeps no access logs.
- **Out of the box it sends nothing.** Until you choose a provider, AutoPaper uses **Demo**, which writes simple ideas and paints gradients on your computer, without any network.
- **What leaves your computer**, when you use a hosted provider: to write a scene, the current mood's keywords and Surprise, short descriptions of recent wallpapers, a few things you've liked or disliked, and your language; to paint it, the finished prompt, the size and the quality. Plus AutoPaper's version, your system's name and version, and your IP address, as with any request online.
- **What never leaves your computer:** your images, history, ratings, mood names, spending, timings and settings. Memory (the check that a new idea isn't one you've had) runs on your computer with a small bundled model.
- **With local models** (Ollama, LM Studio, ComfyUI and the like, on your computer or your own network), nothing leaves your network at all.
- **Your keys stay in your system's secure store** (Keychain, Credential Manager, Secret Service) and are sent only to the service they belong to.

## Who learns what

| Who | Learns | When |
|---|---|---|
| **OpenAI** (`api.openai.com`), if you choose it | What's sent to write a scene and to paint it (below), and that your key is in use | For each new wallpaper; and when Settings lists its models or you choose **Test** |
| **Google** (`generativelanguage.googleapis.com`), if you choose Google Gemini | The same | The same |
| **Your own servers**: Ollama, LM Studio or another server that works like OpenAI's API, ComfyUI | The same, plus, for ComfyUI, the whole workflow AutoPaper runs (AutoPaper's own or yours) | The same, at the address you gave |
| **A hosted service you point AutoPaper at** (an OpenAI-compatible address on the internet) | The same | The same. AutoPaper doesn't know or vouch for such a service; its own terms apply |
| **GitHub** (this project's website, `msitarzewski.github.io`, and its releases) | That a copy of AutoPaper checked for updates, its version, and your IP address | On a Mac, about once a day if you allowed automatic checks, otherwise only when you choose **Check for Updates…**. The Mac's updates, and the Windows package winget installs, are downloaded from GitHub's release servers. See [Updates](#updates) |
| **msitarzewski.com**, the author's own server (Windows' App Installer file and packages, and the Linux Flatpak repository) | Your IP address and your system's User-Agent, while it answers; it keeps no access logs | On Windows, installed with App Installer: each time AutoPaper starts, and about every 8 hours. On Linux: whenever Flatpak checks for or installs updates. See [Updates](#updates) |
| **Microsoft** (winget) | That AutoPaper was installed or updated with winget, as with any app from it | Only when you install or update with winget |
| **Flathub's runtime repository** (Linux) | That the GNOME runtime AutoPaper runs on was installed or updated, as for any Flatpak app that uses it | When Flatpak installs or updates that runtime |
| **Websites you open from AutoPaper**: "Get a key" (OpenAI, Google AI Studio), Brew Browser, Ollama, ComfyUI | Whatever your browser tells any website | Only when you click the link. AutoPaper opens the page in your browser (or Brew Browser itself) and makes no request of its own |

AutoPaper has no server of its own that the apps talk to, and learns nothing. The update files for Windows and Linux are static files on the author's server, which keeps no access logs.

**Under their terms as of October 2026:**

- **OpenAI** says data sent to its API isn't used to train its models unless you opt in, and keeps abuse-monitoring logs, which may contain prompts and responses, for up to 30 days. AutoPaper sends its writing requests with `store: false`, so they aren't also kept as stored responses; image requests keep no application state.
- **Google, with a paid project** (billing turned on), doesn't use what you send to improve its products, and keeps prompts and responses for 55 days to detect abuse. The `generateContent` calls AutoPaper uses store no conversation, so there's nothing for a `store` setting to turn off.
- **Google, with a free Gemini key:** what you send may be used to improve Google's products, and may be read by human reviewers. In the EEA, Switzerland and the UK, the paid terms apply even to free keys. AutoPaper's Settings say this wherever you choose Google: *"A free Gemini key may let Google use what you send to improve its products; a paid project doesn't."* Painting with Gemini needs a paid project anyway, as Google's image models have no free tier.
- **Local servers** keep whatever they keep: ComfyUI, for example, saves each painting in its own `output/autopaper/` folder and lists the job in its history. AutoPaper doesn't delete those.

Each provider's own policy is the one that applies; you hold the key, so you're its customer, not AutoPaper.

## What's sent

Before composing a new wallpaper, AutoPaper checks both selected services with read-only model-list requests (ComfyUI uses `/object_info`). These checks send credentials to their own provider as usual, but no keywords, prompts or images. Demo needs no network; a budget block makes no checks. If a check fails, a saved usable wallpaper from the selected mood can be shown without generation. Both results stay in Console on this computer.

**To write a scene** (one request; two or three when the first ideas are too close to ones you've had, and one more if a provider declines an idea):

- the current mood's **keywords** (Must, Maybe and Avoid) and its **Surprise**;
- **short descriptions of recent wallpapers**, up to 20, so the ideas don't repeat them, and, when it asks again, the descriptions of up to 8 it came too close to;
- **taste hints**: up to 6 things you've tended to like and 6 you've tended to dislike, such as "warm ivory" or "harbour";
- your **language and region**, such as `en-US`, so titles and descriptions come back in your language;
- AutoPaper's fixed instructions (how to compose a wallpaper), and the shape the answer must take.

For an **echo**, the original wallpaper's concept (title, description, setting, palette and so on), its prompt and how long ago it was made go too, instead of the keywords.

Not sent: the mood's name, dates or the time of day, your display's size, what you've spent, or anything that identifies your computer.

**To paint it:** the finished **prompt** (at most 2,500 characters), the **size** (from your display's shape), and the **quality** (Standard or High). ComfyUI also gets the workflow, with a random seed.

**With every request:** a User-Agent such as `AutoPaper/0.1.0 (macOS/26.6 AutoPaper/0.1.0; +https://github.com/msitarzewski/AutoPaper)`, which names AutoPaper's version and your system's name and version; the key, for the service it belongs to; and your IP address. No cookies, no device ID, no account.

**Your descriptions after clearing history:** **Clear History…** with **Keep memory** keeps each wallpaper's description so AutoPaper can still avoid repeats, and the most recent of those can still be sent as "recent" descriptions. **Clear History and Memory** (Windows: **Clear everything**; Linux: **Forget everything**) deletes them.

## What AutoPaper does to keep this small

- **Nothing until you choose.** Demo is the default, and a new install makes no requests at all: on a Mac, it asks before it ever checks for updates.
- **Only what a wallpaper needs.** Memory, taste, the budget and the time estimates are all worked out on your computer; only the writing and painting go to a provider.
- **Keys stay with their service.** Each key is sent only to its own provider. A key for an OpenAI-compatible server belongs to that server's address and is never sent anywhere else. Redirects are followed only within the same host, and a change of scheme or port drops the key first.
- **Your network stays yours.** Plain `http` is used only for addresses on your own computer or local network; anything further away needs `https`. Hosted providers are reached only at their own API host, over `https`.
- **No cookies, no cache, no Referer**, and responses are size-limited and checked before use (see [NETWORK.md](./NETWORK.md)).
- **Local request transparency.** Console keeps the prompts, provider/model choices, responses, retry decisions and outcomes of your wallpaper runs on this computer. It never records authentication headers or image bytes, and redacts credentials from retained details. Nothing in Console is uploaded automatically; Copy Details and Export JSON share only the run you choose. System logs still contain no prompts or request bodies.

## What's stored on your computer

Everything AutoPaper keeps is in its own data folder, plus a few preferences. None of it is uploaded anywhere.

| What | Where | How long |
|---|---|---|
| Your moods and keywords, settings, history (each wallpaper's concept, prompt, description, the providers and models that made it, its size, estimated cost and rating), memory, taste, monthly spending | `autopaper.sqlite3` in the data folder | Until you delete them. Failed attempts are deleted after 30 days |
| Wallpapers as delivered | `images/` in the data folder | Up to the storage limit you set (2 GB unless you change it); the oldest go first, never ones you liked or the one on your desktop |
| Thumbnails (640 px wide) | `thumbs/` | Kept when the original is cleared, so History still shows it |
| Wallpapers sized for each display | `renders/` | A cache, cleared first |
| How long each provider took (job, provider, server address, model, size, steps, seconds) | `autopaper.sqlite3` | The newest 50 for each provider and model. Never sent |
| Your own wallpaper's location (macOS, with **As my wallpaper**), or a copy of it (Windows; Linux on GNOME), so it can be put back | macOS preferences; Windows `own-wallpaper\` in the data folder; Linux `own-wallpaper/` in the data folder | Replaced whenever AutoPaper records it again |
| A ComfyUI workflow of your own, if you chose one | macOS preferences; Windows `workflows\` in the data folder; Linux GSettings | Until you choose another |
| App preferences (for example notifications, the last Settings pane) | macOS preferences; Windows app settings; Linux GSettings (`io.github.msitarzewski.AutoPaper`) | Until you change them |
| API keys | Keychain; Credential Manager; Secret Service | Until you remove them in Settings |
| Console runs (original mood/keywords/Surprise, prompts, provider/model choices, sanitized requests and responses, retry checks, timing, estimated cost and outcome) | `runs` in `autopaper.sqlite3`; visible in Console | At most 200 runs for 30 days; Clear Console deletes them independently of wallpapers and memory. Each trace is size-limited, with omitted details labeled. Requests made before this version were not recorded |
| Diagnostics | macOS: the system log (errors, and a line naming the providers and models that made each wallpaper); Windows: `logs\autopaper.log` in the data folder (unexpected errors only, at most 256 KB, plus one older file); Linux: standard error only, no log file | No keys, keywords or prompts in any of them |

The data folder is:

- **macOS:** `~/Library/Containers/com.autopaper/Data/Library/Application Support/AutoPaper` (AutoPaper is sandboxed).
- **Windows:** the app's own `LocalState` folder, under `%LOCALAPPDATA%\Packages\`.
- **Linux:** `~/.local/share/autopaper` (`$XDG_DATA_HOME/autopaper`).

Keys are stored under these names: on macOS, Keychain items for the service `com.autopaper.credentials`; on Windows, Credential Manager entries named `AutoPaper:openai.api_key` and so on, kept on this PC only (they don't roam with your account); on Linux, Secret Service items labelled "AutoPaper: OpenAI API key" and so on.

## Permissions

- **macOS:** AutoPaper runs in the App Sandbox and asks for only outgoing network connections and read access to a file you choose (a ComfyUI workflow of your own). No location, contacts, camera, microphone or photos. Notifications are off unless you turn them on, and macOS asks first.
- **Windows:** a packaged desktop app. It reads and sets the desktop picture, sets the lock screen picture unless you turn that off, and shows notifications unless you turn them off. It has access to your Pictures library (the `picturesLibrary` capability) only because Windows hands an app the lock screen's current picture only with it: AutoPaper uses it to read your own lock screen picture, so it can put it back, and for nothing else. When you install it, App Installer lists three things it can do:
  - **Uses all system resources** (`runFullTrust`): it's an ordinary desktop app, as every WinUI 3 desktop app is, rather than one confined like a Microsoft Store app.
  - **Use your pictures library** (`picturesLibrary`): only to read the lock screen picture, as above.
  - **Write registry entries and files that are not cleaned up on uninstall** (`unvirtualizedResources`): when AutoPaper sets the lock screen picture, Windows records that in your own settings rather than in a private copy only AutoPaper sees. Without it, Windows Settings kept showing your old lock screen and the change didn't take. AutoPaper uses it for nothing else; its history, images and settings stay in its own app folder.
- **Linux:** it sets the wallpaper through the desktop's Wallpaper portal (which may ask you once), runs in the background through the Background portal, and keeps keys in the Secret Service.
- **Siri and Shortcuts (macOS):** AutoPaper's actions can make a new wallpaper, like or dislike the current one, switch moods, and say what's on your desktop. The answers stay on your Mac, apart from whatever Siri itself does with a spoken request.
- **Network:** outgoing only. AutoPaper accepts no incoming connections.

## Turning things off

- **Choose Demo, or local providers**, in Settings → Providers, and nothing leaves your computer or your network.
- **Remove a key** in Settings → Accounts (Windows and Linux: Keys), and that service is never contacted.
- **Delete a wallpaper** from History, and its image and memory go; **Clear History…** clears them all, keeping memory unless you choose otherwise; **Reset What It Learned…** forgets your taste.
- **Choose Only when I ask** in Settings → General, and AutoPaper contacts a provider only when you ask: for a new wallpaper, an echo, or a replacement for one you dislike (that last one can be turned off too), or when Settings → Providers lists a provider's models.

## Updates

Each app is updated the usual way for its system. None of these sends your keys, keywords, prompts, wallpapers or settings, and none carries an account or a device ID: AutoPaper has neither. What's sent is what any browser sends for a file: your IP address, and a User-Agent naming the program asking. Flatpak also says, as it does to every repository, which app it's fetching and, when updating, which version you have.

- **macOS (Sparkle):** on its second launch AutoPaper asks whether to check for updates automatically; nothing is checked before you answer. If you allow it, it checks about once a day, and **Check for Updates…** checks whenever you choose. A check downloads the update feed, `https://msitarzewski.github.io/AutoPaper/appcast.xml`, from this project's website on GitHub Pages, with the User-Agent `AutoPaper/<version> Sparkle/<version>`. Sparkle can also send a system profile (your Mac's model, processor, macOS version and language); AutoPaper leaves that off, so it is never sent. An update you choose to install is downloaded from GitHub's release servers, and it's installed only if its signature matches the public key built into AutoPaper.
- **Windows, installed with App Installer:** Windows' App Installer, not AutoPaper, checks AutoPaper's App Installer file, `https://msitarzewski.com/app-updates/autopaper/AutoPaper.appinstaller`, each time AutoPaper starts and in the background about every 8 hours (the file's own update settings), and downloads a new version's package from the same server. Windows installs it only if its signature checks out.
- **Windows, installed with winget:** `winget upgrade` asks Microsoft's winget repository for the newest version, and downloads the package attached to that release on GitHub, when you run it.
- **Linux (Flatpak):** Flatpak, not AutoPaper, checks AutoPaper's Flatpak repository, `https://msitarzewski.com/app-updates/autopaper/flatpak` (the remote `autopaper`), whenever GNOME Software or KDE Discover checks for updates or you run `flatpak update`. A check downloads the repository's summary; an update downloads only the files that changed. Flatpak installs only what the repository's key signed. AutoPaper never checks for updates itself.

**msitarzewski.com** is the author's own server. HTTPS ends at a front proxy, which forwards each request over a private network to the file server; neither keeps access logs for msitarzewski.com. Like any web server, it sees your IP address and the User-Agent of the program asking (Windows' App Installer, or Flatpak) while it answers.

## Questions

Report privacy concerns the same way as security issues; see [SECURITY.md](./SECURITY.md).
