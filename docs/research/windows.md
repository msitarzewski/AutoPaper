# AutoPaper Windows: research and VM setup (verified 2026-10-05)

Everything marked **[verified in VM]** was executed in the Parallels VM "Windows 11" (ARM64), not just read about.

---

## 0. TL;DR pin list

| Item | Pin | Source |
|---|---|---|
| .NET SDK | **10.0.401** (runtime 10.0.12, LTS). .NET 11 is RC1 only | https://builds.dotnet.microsoft.com/dotnet/release-metadata/releases-index.json |
| TFM | **`net10.0-windows10.0.26100.0`** (the official template default). `TargetPlatformMinVersion` should be `10.0.22000.0` for a Windows 11-only app (the template sets 10.0.17763.0) | template nupkg (below) |
| Windows App SDK | **Microsoft.WindowsAppSDK 2.5.1** (Stable, 2026-09-16). 2.x uses SemVer, so pin `2.5.1`, not `*` | https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-channels |
| Windows SDK build tools | **Microsoft.Windows.SDK.BuildTools 10.0.28000.2705** | NuGet |
| `dotnet run` and packaging glue | **Microsoft.Windows.SDK.BuildTools.WinApp 0.7.1** plus the **winapp CLI 0.7.1** (winget id `Microsoft.WinAppCli`) | https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli |
| Templates | **Microsoft.WindowsAppSDK.WinUI.CSharp.Templates 0.0.7-alpha** (official, preview) | https://devblogs.microsoft.com/ifdef-windows/introducing-dotnet-new-templates-for-winui/ |
| Settings UI | **CommunityToolkit.WinUI.Controls.SettingsControls 8.2.251219** | NuGet, https://learn.microsoft.com/en-us/dotnet/communitytoolkit/windows/settingscontrols/settingscard |
| Segmented | **CommunityToolkit.WinUI.Controls.Segmented 8.2.251219** | NuGet |
| TokenizingTextBox | **CommunityToolkit.WinUI.Controls.TokenizingTextBox 8.2.251219** | NuGet |
| MVVM | **CommunityToolkit.Mvvm 8.4.2** | NuGet |
| Tray icon | **WinUIEx 2.9.3** (`TrayIcon`, Aug 2026, actively maintained) is the recommended choice. **H.NotifyIcon.WinUI 2.4.1** (Dec 2025; 2.5.0-beta.3 Sep 2026) is the alternative | GitHub/NuGet |
| Win32/COM interop | **Microsoft.Windows.CsWin32 0.3.346** (2026-10-02) | https://github.com/microsoft/CsWin32 |
| Rust | **rustc 1.99.0** stable. Targets `aarch64-pc-windows-msvc` and `x86_64-pc-windows-msvc` | VM |
| UniFFI (C#) | **uniffi-bindgen-cs v0.11.0+v0.31.0**. This **forces the core to use `uniffi = "0.31"`** (resolved to 0.31.2), even though crates.io latest is 0.32.2 | https://github.com/NordSecurity/uniffi-bindgen-cs |
| A11y scanner | **AxeWindowsCLI 2.4.2**. Use the self-contained zip; the MSI needs the x64 .NET 6 runtime | https://github.com/microsoft/axe-windows/releases |

**[verified in VM]** All of the NuGet packages above restore and build together in a single `net10.0-windows10.0.26100.0` project with WinAppSDK 2.5.1, with 0 warnings and 0 errors. `CsWin32` also generated `IDesktopWallpaper`, `DesktopWallpaper`, `CredWrite`, `CredRead`, `CredDelete` and `CredFree`.

---

## 1. Windows App SDK, .NET and CLI-only project setup

### Versions
- Windows App SDK stable releases: 2.0.1 (04/29), 2.1.3, 2.2.0, 2.3.1, 2.4.0, **2.5.1 (09/16/2026)**. 1.8 is in Maintenance and its servicing ended 09/24/2026. The package family name now follows the major version (`Microsoft.WindowsAppRuntime.2`). See https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-2-0?pivots=stable
- Notable 2.x additions: `SystemBackdropElement` (2.0), TitleBar drag-region APIs (2.1), `ApplicationData.GetForUnpackaged()` (2.2). 2.x adds no tray-icon API and no StartupTask changes.
- The official quickstart says ".NET SDK 10 or later": https://learn.microsoft.com/en-us/windows/apps/develop/ai-assisted/quickstart

### Official `dotnet new` templates (no Visual Studio)
```
dotnet new install Microsoft.WindowsAppSDK.WinUI.CSharp.Templates   # 0.0.7-alpha
dotnet new winui-navview -n AutoPaper     # also: winui, winui-mvvm, winui-tabview, winui-lib, winui-unittest, item templates winui-page/-dialog/...
```
VijayAnand.WinUITemplates (community, 6.0.0, 2026-04) still exists. The official package is preferred now.

Generated csproj (navview template, verbatim keys):
```xml
<OutputType>WinExe</OutputType>
<TargetFramework>net10.0-windows10.0.26100.0</TargetFramework>
<TargetPlatformMinVersion>10.0.17763.0</TargetPlatformMinVersion>
<ApplicationManifest>app.manifest</ApplicationManifest>
<Platforms>x86;x64;ARM64</Platforms>
<RuntimeIdentifier Condition="'$(RuntimeIdentifier)' == ''">win-$([System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant())</RuntimeIdentifier>
<PublishProfile Condition="Exists('Properties\PublishProfiles\win-$(Platform).pubxml')">win-$(Platform).pubxml</PublishProfile>
<UseWinUI>true</UseWinUI>
<WinUISDKReferences>false</WinUISDKReferences>
<EnableMsixTooling>true</EnableMsixTooling>
<!-- PackageReferences: Microsoft.Windows.SDK.BuildTools, Microsoft.WindowsAppSDK, Microsoft.Windows.SDK.BuildTools.WinApp (all Version="*" -> pin them) -->
<!-- Release: PublishReadyToRun=True, PublishTrimmed=True -->
```
- `WindowsPackageType` is **not** set, which means packaged (MSIX) by default. Setting `WindowsPackageType=None` makes the app unpackaged. AutoPaper needs identity for notifications, StartupTask and ApplicationData, so leave it packaged.
- The publish profiles `win-arm64` and `win-x64` set `SelfContained=true` (.NET is self-contained; the Windows App Runtime is a framework dependency unless `WindowsAppSDKSelfContained=true`).
- The template manifest declares `<rescap:Capability Name="runFullTrust"/>` and `<systemai:Capability Name="systemAIModels"/>`. Drop `systemAIModels` unless it is needed. It also uses `Publisher="CN=AppPublisher"`. Set a real CN, and keep it identical to the signing cert subject.
- **Trimming caveat:** Release sets `PublishTrimmed=True`. Microsoft's single-instance doc warns of crashes in trimmed WinAppSDK apps (https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/applifecycle/applifecycle-single-instance). Classic `[ComImport]` COM is also not trim-safe. Either set `PublishTrimmed=false`, or use CsWin32 with `CsWin32RunAsBuildTask=true` + `DisableRuntimeMarshalling=true` + `comInterop.useComSourceGenerators` (see §3).

### Build, run, package and sideload from the CLI **[verified in VM]**
```powershell
dotnet build -c Debug -p:Platform=ARM64                       # ~60 s cold
winapp run AutoPaper.csproj --arch arm64 --detach --json       # registers loose layout with identity + launches; prints AUMID/PID
#   (the template's Microsoft.Windows.SDK.BuildTools.WinApp also makes plain `dotnet run` do this)
winapp cert generate --manifest Package.appxmanifest --output devcert.pfx --export-cer --password <pw>   # publisher read from manifest
dotnet publish -c Release -r win-arm64 -p:Platform=ARM64 -o publish\arm64
dotnet publish -c Release -r win-x64   -p:Platform=x64   -o publish\x64     # cross-arch publish on ARM64 host works
winapp pack publish\arm64 --cert devcert.pfx --password <pw> --output AutoPaper_arm64.msix
winapp pack publish\x64 publish\arm64 --cert devcert.pfx --password <pw> --output AutoPaper.msixbundle   # multi-arch bundle works
# trust (ADMIN; done as SYSTEM via prlctl):  Import-Certificate devcert.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
Add-AppxPackage .\AutoPaper_arm64.msix                        # as the desktop user -> SignatureKind=Developer, depends on Microsoft.WindowsAppRuntime.2_2.5.1
```
- `winapp cert install` and `--install-cert` need elevation. The default cert password is `"password"`, which is public, so always pass `--password`. CLI reference: https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli/usage
- Gotcha (seen in the VM): all packages in a bundle must have identical `<Resource Language>` sets. `x-generate` derives these from the publish output, so publish every arch from the same commit or pin `<Resource Language="en-US"/>`.
- MSBuild-only alternative (no winapp): `dotnet publish -p:GenerateAppxPackageOnBuild=true -p:AppxPackageSigningEnabled=true -p:PackageCertificateKeyFile=... `. The winapp path is what Microsoft now documents.

---

## 2. Native Windows 11 patterns

### Tray (notification-area) icon
WinUI 3 and Windows App SDK 2.5 have **no built-in tray icon** (open since 2020: https://github.com/microsoft/microsoft-ui-xaml/issues/2020).

| Option | Status | Context menu |
|---|---|---|
| **WinUIEx 2.9.3** `TrayIcon` / `WindowManager.IsVisibleInTray` | Released 2026-08-13. Repo pushed 2026-09-27. 2.9.3 fixed re-adding the icon after an explorer restart (`TaskbarCreated`) and added SVG icons. Depends on Microsoft.WindowsAppSDK.WinUI ≥1.8 | XAML `MenuFlyout` set in the `ContextMenu` event; the icon works window-less. Docs: https://github.com/dotMorten/WinUIEx/blob/main/docs/concepts/TrayIcon.md |
| **H.NotifyIcon.WinUI 2.4.1** (stable, 2025-12-01). 2.5.0-beta.3 (2026-09-12) | Recent commits are mostly dependabot bumps. Has Efficiency Mode helpers and generated icons | Default `ContextMenuMode=PopupMenu` builds a **native Win32 popup menu** from your MenuFlyout, which matches built-in Windows 11 tray apps. `SecondWindow` mode is preview. Theme follows the system (`ContextMenuThemeMode`) in newer builds. https://github.com/HavenDV/H.NotifyIcon |
| Raw `Shell_NotifyIcon` via CsWin32 | Full control, ~200 lines (message-only window, `NIM_ADD`/`NIM_SETVERSION 4`, `TaskbarCreated`, `TrackPopupMenuEx`) | Native |

Recommendation: use **WinUIEx TrayIcon**. It is better maintained, small, and also gives window persistence and minimize-to-tray. If the native Win32 popup-menu look matters more than XAML flyouts, use H.NotifyIcon in `PopupMenu` mode. Both compiled against WinAppSDK 2.5.1 in the VM.

### Background or tray-resident behaviour and single instance
- Keep the process alive by cancelling `AppWindow.Closing` (`e.Cancel = true; sender.Hide();`) while a TrayIcon exists. The WinUIEx docs show this exact pattern.
- Single instance: define `DISABLE_XAML_GENERATED_MAIN` and write a custom `Program.Main`. In it, call `AppInstance.FindOrRegisterForKey("AutoPaper")`. A secondary instance calls `RedirectActivationToAsync` and waits with `CoWaitForMultipleObjects`. The main instance handles `Activated`. Source: https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/applifecycle/applifecycle-single-instance

### Settings-style UI, navigation and backdrop
- `NavigationView` (Left mode) plus **SettingsCard/SettingsExpander** (`CommunityToolkit.WinUI.Controls.SettingsControls`). This matches the Windows 11 Settings app.
- **Mica:** `SystemBackdrop = new MicaBackdrop()` on the Window. Extend into the title bar with a transparent custom title bar. It falls back to `SolidBackgroundFillColorBase` when transparency is off, on battery saver, or in high contrast. https://learn.microsoft.com/en-us/windows/apps/design/style/mica
- Segmented control: `CommunityToolkit.WinUI.Controls.Segmented` (8.2.251219). A native alternative is WinUI `SelectorBar` (WinAppSDK ≥1.5).
- `TokenizingTextBox`: `CommunityToolkit.WinUI.Controls.TokenizingTextBox` (8.2.251219).

### App notifications with buttons (packaged)
Source: https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-quickstart (updated 2026-09-10)
- Manifest: add `desktop:Extension Category="windows.toastNotificationActivation"` with `ToastActivatorCLSID=<GUID>`, plus `com:Extension Category="windows.comServer"` → `com:ExeServer Executable="AutoPaper.exe" Arguments="----AppNotificationActivated:"` → `com:Class Id=<same GUID>`.
- Startup order: create the window without activating it, subscribe `AppNotificationManager.Default.NotificationInvoked`, then call `Register()`, then call `AppInstance.GetCurrent().GetActivatedEventArgs()`. `Register` must come before `GetActivatedEventArgs`.
- Build: `new AppNotificationBuilder().AddArgument("action","x").AddText(...).AddButton(new AppNotificationButton("Keep").AddArgument("action","keep")).BuildNotification()` → `AppNotificationManager.Default.Show(n)`.
- `activationType="background"` is ignored for desktop apps, so the app must decide itself whether to show UI. Notifications do not work when the app runs elevated.

### StartupTask (launch at sign-in) and toggling it from Settings
Source: https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptask
```xml
<uap5:Extension Category="windows.startupTask" Executable="AutoPaper.exe" EntryPoint="Windows.FullTrustApplication">
  <uap5:StartupTask TaskId="AutoPaperStartup" Enabled="false" DisplayName="AutoPaper" />
</uap5:Extension>
```
- In the Settings toggle: `var t = await StartupTask.GetAsync("AutoPaperStartup");`. On: `await t.RequestEnableAsync()` (no consent dialog for packaged desktop apps). Off: `t.Disable()`.
- Reflect the states `DisabledByUser`, `DisabledByPolicy` and `EnabledByPolicy`. For `DisabledByUser`, the toggle must point the user to Settings > Apps > Startup, because the app cannot override it.
- Pass a launch argument or check `AppInstance` activation so a sign-in start goes straight to the tray without showing the window.

---

## 3. Wallpaper and lock screen

### Desktop wallpaper: `IDesktopWallpaper` (shell COM, desktop apps only, Win8+)
https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-idesktopwallpaper
- Methods: `GetMonitorDevicePathCount`/`GetMonitorDevicePathAt` (per-monitor IDs), `GetMonitorRECT`, `SetWallpaper(monitorId|null, path)`, `GetWallpaper`, `SetPosition(DWPOS_FILL|FIT|STRETCH|TILE|CENTER|SPAN)`, `SetBackgroundColor`, slideshow APIs, `Enable`.
- C#: use **CsWin32**. Put `IDesktopWallpaper` and `DesktopWallpaper` in `NativeMethods.txt`. Build-verified in the VM.
  - With trimming or AOT, set `<CsWin32RunAsBuildTask>true</CsWin32RunAsBuildTask><DisableRuntimeMarshalling>true</DisableRuntimeMarshalling>` and `"comInterop": {"useComSourceGenerators": true}` in `NativeMethods.json`. This emits `[GeneratedComInterface]`/`LibraryImport` code. See https://github.com/microsoft/CsWin32 (docfx/docs/getting-started.md, settings.schema.json).
  - Create and call it on an STA thread (the UI thread is fine).
- **Where to write image files:** packaged full-trust apps have their **AppData writes virtualized**. New files under `%LOCALAPPDATA%\…` are redirected to a private per-package location that explorer.exe may not see at that path (https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes). Write images to `ApplicationData.Current.LocalFolder.Path` (the real path `…\AppData\Local\Packages\<PFN>\LocalState`) or to the user's Pictures folder, and pass that real path to `SetWallpaper` and to the lock-screen API.
- The VM currently shows Windows Spotlight on the desktop. `SetWallpaper` replaces it with Picture mode.

### Lock screen
- APIs:
  - `UserProfilePersonalizationSettings.IsSupported()` + `Current.TrySetLockScreenImageAsync(StorageFile)` returns a bool (https://learn.microsoft.com/en-us/uwp/api/windows.system.userprofile.userprofilepersonalizationsettings).
  - `Windows.System.UserProfile.LockScreen.SetImageFileAsync(IStorageFile)` (Desktop extension SDK, https://learn.microsoft.com/en-us/uwp/api/windows.system.userprofile.lockscreen).
- Documented constraints:
  - **Each new image must have a different file name** than the previous one, or the call fails.
  - Microsoft's example loads the file from LocalFolder via `ms-appx:///Local/<name>`. With identity, use `StorageFile.GetFileFromPathAsync(LocalFolder.Path + name)`.
  - Mobile is limited to 2 MB, which does not apply here.
- Unpackaged desktop apps have had trouble with this API (Microsoft Q&A https://learn.microsoft.com/en-us/answers/questions/707448/change-windows-lock-screen-image-microsoft-windows). Packaged identity avoids the "no package identity" class of errors.
- Policy and edition limits:
  - The GPO "Prevent changing lock screen and logon image" (`HKLM\SOFTWARE\Policies\Microsoft\Windows\Personalization\NoChangingLockScreen`) blocks it.
  - The MDM Personalization CSP (`LockScreenImageUrl`) forces an image and prevents user changes. It is supported on Enterprise/Education, and on Pro only in SharedPC/BootToCloud modes (https://learn.microsoft.com/en-us/windows/client-management/mdm/personalization-csp).
  - Treat a `false` return as "managed or unsupported" and surface it in the UI.
  - If lock-screen Spotlight is on, a successful set switches the lock screen to Picture mode.
    **Correction (verified 2026-10-07 in the VM):** only true with `RegistryWriteVirtualization` disabled in the manifest (restricted capability
    `unvirtualizedResources`); otherwise Windows' in-process lock screen code records "Picture mode" in the app's private registry copy and Settings keeps
    showing Spotlight. Reading the current lock screen picture also needs the `picturesLibrary` capability.
- **[verified in VM, read-only]** `IsSupported()` is True. No `NoChangingLockScreen` policy and no CSP image are set. `RotatingLockScreenEnabled=1` (Spotlight). Calling the setter was not tested because it would change the VM's lock screen. Test it from the real packaged app.
- **Recommendation:** in the packaged app, write a uniquely named file to `LocalFolder`, try `TrySetLockScreenImageAsync`, fall back to `LockScreen.SetImageFileAsync`, and report "managed by your organization" when both fail or policy keys exist.

### Accent colour and dark mode (v2)
- `Windows.UI.ViewManagement.UISettings.GetColorValue(UIColorType.Accent / Foreground / Background)`. Dark mode means the Foreground colour is light. `ColorValuesChanged` fires on a background thread, so marshal it to the DispatcherQueue (https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-windows-themes).
- WinAppSDK also has `Microsoft.UI.System.ThemeSettings` (including `HighContrast`).

---

## 4. Secret storage (API keys)

| Option | Packaged WinUI 3 | Notes |
|---|---|---|
| `Windows.Security.Credentials.PasswordVault` (Credential Locker) | Works | ≤**20 credentials per app**. **Roams with the Microsoft account**. "Only use for passwords, not larger blobs". https://learn.microsoft.com/en-us/windows/apps/develop/security/credential-locker |
| **Credential Manager** `CredWriteW`/`CredReadW` (`CRED_TYPE_GENERIC`, `CRED_PERSIST_LOCAL_MACHINE`) via CsWin32, or the Rust `keyring` crate (4.2.0, Windows backend `windows-native-keyring-store`) | Works (full trust) | Per-user, DPAPI-protected, does not roam. Visible in Control Panel > Credential Manager. Any process running as that user can read it, which is the same trust model as Keychain and Secret Service. Blob ≤ 2560 bytes |
| DPAPI `ProtectedData` (`System.Security.Cryptography.ProtectedData`) | Works | You manage the file (put it in LocalFolder). Good for larger blobs |

**Recommendation:** use **Credential Manager**. The repo's `.gitignore` already names Credential Manager next to Keychain and Secret Service. If the shared Rust core owns secrets, use the `keyring` crate so a single code path covers macOS, Linux and Windows. Otherwise call `CredWrite`/`CredRead` from C# through CsWin32 (already build-verified). Use target names like `AutoPaper/<provider>`. Avoid PasswordVault: its MSA roaming silently copies API keys to other devices, and it has a 20-entry limit.

---

## 5. Accessibility

- Names and descriptions: set `AutomationProperties.Name` on icon-only or image controls, `AutomationProperties.HelpText` for extra guidance, and `AutomationProperties.LabeledBy` to link form labels. Set `AutomationProperties.AutomationId` everywhere for tests. Checklist: https://learn.microsoft.com/en-us/windows/apps/design/accessibility/accessibility-checklist
- Live updates (for example "Generating wallpaper…" → "Applied"): `AutomationProperties.LiveSetting="Polite|Assertive"` on the status element plus `FrameworkElementAutomationPeer.FromElement(x)?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged)`, or the newer `AutomationPeer.RaiseNotificationEvent(AutomationNotificationKind.ActionCompleted, AutomationNotificationProcessing.ImportantMostRecent, text, activityId)` (https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.automation.peers.automationpeer.raisenotificationevent).
- Keyboard: keep the default tab order, set `TabIndex` only where needed, use `AccessKey` on primary commands (Alt shows KeyTips), and `KeyboardAccelerator` for shortcuts.
- Contrast themes:
  - Use `ThemeDictionaries` with a `HighContrast` dictionary that maps to `SystemColor*Color` resources. Do not hard-code colours.
  - Consider `HighContrastAdjustment=None` once the brushes are correct.
  - Test all 4 themes (Aquatic, Desert, Dusk, Night sky). Left Alt+Left Shift+PrtScn toggles. https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes
- Text size: all WinUI text controls honour "Make text bigger". Do not clip at 225%.
- Narrator tips: Ctrl+Win+Enter toggles it. Caps Lock+arrows navigate scan mode. Verify that every SettingsCard reads header plus state. Verify that toasts are read and that their buttons are reachable with Win+N then Tab.
- Automation for audits **[verified in VM]**:
  - **AxeWindowsCLI 2.4.2**: `C:\Tools\AxeWindowsCLI-2.4.2\AxeWindowsCLI.exe --processid <pid> --outputdirectory <dir>` reported "0 errors" on the template app. Use the zip build (self-contained). The MSI build failed because it needs the x64 .NET 6 runtime. The `.a11ytest` output opens in Accessibility Insights for Windows (GUI v1.1.3203.01). Accessibility Insights itself has no CLI; Axe.Windows (NuGet `Axe.Windows` 2.4.2) is its automation engine.
  - **`winapp ui inspect -a <proc> -i`** dumps the UIA tree with names and AutomationIds. `winapp ui invoke|click|set-value|screenshot|search` support scripted UI tests (https://learn.microsoft.com/en-us/windows/apps/develop/ai-assisted/testing).
  - Both need the interactive session, so run them via `prlctl exec --current-user`.

---

## 6. Rust core → C# (ARM64 + x64)

### UniFFI C# bindings **[verified end-to-end in VM]**
- Install: `cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.11.0+v0.31.0` (~5 min on the VM).
  - It targets **uniffi 0.31.x**. The core crate must use `uniffi = "0.31"`, and so must any Swift or Kotlin bindgen used for the other platforms (one shared uniffi version).
  - .NET 8+ requires `<AllowUnsafeBlocks>true</AllowUnsafeBlocks>`. Generated code uses `[LibraryImport]` on NET8+.
  - Record properties are PascalCase.
  - Async and callback interfaces are supported, and v0.11 fixed several async races.
  - Strings and lists are limited to 2^31.
- Generate: `uniffi-bindgen-cs --library target\aarch64-pc-windows-msvc\release\autopaper_core.dll --out-dir <dir> [--config uniffi.toml]`.
  - The output is an `internal static class <Crate>Methods` in namespace `uniffi.<crate>`. Compile it into the app assembly, or configure it.
  - Optional: `dotnet tool install -g csharpier` for formatting.
- The test crate (edition 2024, `crate-type=["cdylib"]`, `uniffi::setup_scaffolding!()`, `#[uniffi::export]`) built for both targets. A .NET 10 console printed `Arm64: add(2,3)=5; Hello… on aarch64` and `X64: …on x86_64` (x64 runs under emulation). Kept at `C:\Users\michael\dev\uniffi-smoke`.
- Fallback C ABI: in edition 2024, `#[no_mangle]` must be written `#[unsafe(no_mangle)]`.

### Where the DLL goes
Put `autopaper_core.dll` (arch-matched) **in the package root next to `AutoPaper.exe`**. Use a csproj `<None|Content Include=… CopyToOutputDirectory=PreserveNewest Link="autopaper_core.dll" Condition="'$(RuntimeIdentifier)'=='win-arm64'">`, with one item per RID; the same pattern was used in the VM test. For packaged apps the loader searches "the package dependency graph of the process" (which includes the app's own package) and then the exe folder (https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order). Ship one DLL per arch-specific MSIX, and use a `.msixbundle` for both.

### Building the DLL
- **Natively in the VM (recommended, proven):**
  - The VM has Build Tools 2022 17.14 (MSVC 14.44, ARM64 and x64 toolsets) and the Windows 11 SDK 10.0.26100, which is enough for MSVC linking.
  - `cargo build --release --target aarch64-pc-windows-msvc` takes ~33 s and `--target x86_64-pc-windows-msvc` ~18 s for a small crate.
  - Set `CARGO_TARGET_DIR` to local NTFS (for example `C:\Users\michael\dev\target`) rather than the PrlSF share.
- **cargo-xwin from macOS (0.23.1, 2026-08):** `cargo install --locked cargo-xwin`, `rustup target add aarch64-pc-windows-msvc x86_64-pc-windows-msvc`, `rustup component add llvm-tools`, `brew install llvm`, then `cargo xwin build --release --target …`. It downloads the MSVC CRT and Windows SDK through xwin, which means accepting the Microsoft license. https://github.com/rust-cross/cargo-xwin
  - Generally works for pure-Rust cdylibs (both targets). Crates with C/asm (ring, aws-lc-rs, openssl) can need extra setup, so prefer rustls with the `ring` backend, which builds with clang-cl.
  - **Not tested on the Mac.** The Mac has Homebrew rust 1.98.1 plus a mixed rustup, no cargo-xwin, no Homebrew llvm, and no Windows targets. Installing those changes the Mac toolchain, so it was left alone pending approval.
  - Even with cross-compiling, the MSIX packaging and UI testing still need the VM, so building natively in the VM is simplest.

---

## 7. VM facts (Parallels "Windows 11")

- **OS:** Windows 11 Pro **Insider Preview (Dev/Canary) build 29680.1000** (rs_prerelease.260925-1447), **ARM64**, "Evaluation copy". It has 10 vCPU and 64 GB RAM, with ~155 GB free on C:. Insider builds expire, so watch for that.
- **Users:** `prlctl exec "Windows 11" <cmd>` runs as **NT AUTHORITY\SYSTEM** (elevated; use it for machine installs, certs and HKLM).
  - `prlctl exec "Windows 11" --current-user <cmd>` runs as the logged-in desktop user **windows-arm\michael** (non-elevated, console session 1). UI apps launched this way appear on the desktop. Use it for winget, rustup, dotnet, winapp, Add-AppxPackage and UIA tools.
  - michael is in Administrators, but UAC is on (`ConsentPromptBehaviorAdmin=5`), so his processes are not elevated.
- **Running scripts:** put the `.ps1` in `/Users/michael/Clean/autopaper/.scratch/windows/` (git-ignored), then run `prlctl exec "Windows 11" [--current-user] powershell -NoProfile -ExecutionPolicy Bypass -File '\\Mac\Home\Clean\autopaper\.scratch\windows\x.ps1'`. Avoid long `-EncodedCommand` (prlctl fails). Piping stdin to `-Command -` also works, but `$ErrorActionPreference='Stop'` combined with `2>&1` on native commands kills them in PS 5.1.
- **Shared folders:** `\\Mac\Home` = `/Users/michael` (desktop user sees it as **Z:**; SYSTEM can use the UNC path). The repo is at **`\\Mac\Home\Clean\autopaper`** = `Z:\Clean\autopaper`. Other mappings: W: = `\\Mac\agency-agents-app`, X: = `\\Mac\Mac Public`, Y: = `\\Mac\BeastDownloads`.
  - `dotnet build` directly on the share **works** (~50 s).
  - **Package registration from the share fails**: `winapp run`/`Add-AppxPackage -Register` gives "Windows cannot deploy to path AppX of file system type PrlSF (0x80073CFD)", and mapped drives give 0x80073D1F.
  - Workflow: `robocopy \\Mac\Home\Clean\autopaper C:\Users\michael\dev\autopaper /MIR /XD .git bin obj target AppX .scratch /XF .env .env.*`, then build, run and pack there. Copying a finished `.msix` from the share and running `Add-AppxPackage` on it is fine.
- **Screenshots:** `prlctl capture "Windows 11" --file /path/out.png` gives a 1465x839 PNG of the whole VM screen. It works while the VM is running and was used to confirm the WinUI app rendered.

### Installed now
| Tool | Version | How |
|---|---|---|
| .NET SDK (arm64) | 10.0.401 → `C:\Program Files\dotnet` | official installer, SHA-512 checked, SYSTEM, `/quiet` |
| Rust (rustup, user michael) | rustc/cargo 1.99.0, host aarch64-pc-windows-msvc, targets aarch64 + x86_64 msvc | rustup-init (SHA-256 checked), `--current-user` |
| uniffi-bindgen-cs | 0.11.0+v0.31.0 → `%USERPROFILE%\.cargo\bin` | cargo install |
| winapp CLI | 0.7.1 (MSIX, user) | `winget install --id Microsoft.WinAppCli --exact` (id is case-sensitive; `Microsoft.winappcli` is not found) |
| WinUI templates | Microsoft.WindowsAppSDK.WinUI.CSharp.Templates 0.0.7-alpha | dotnet new install |
| AxeWindowsCLI | 2.4.2 self-contained → `C:\Tools\AxeWindowsCLI-2.4.2` | zip (Microsoft-signed); the MSI was uninstalled |
| Developer Mode | ON (`AppModelUnlock\AllowDevelopmentWithoutDevLicense=1`, `AllowAllTrustedApps=1`) | registry as SYSTEM |
| Already present | VS **Build Tools 2022 17.14** at `C:\BuildTools` (VCTools workload, ARM64/ARM64EC/x64/x86 MSVC 14.44, Windows 11 SDK 10.0.26100), Git 2.55.0, winget 1.30.140 (user only), Windows App Runtime 2.5.1 (arm64/x64/x86) | — |

- Not installed and not needed: Visual Studio IDE, VS 2026 Build Tools, Accessibility Insights GUI, x64 .NET runtime (x64 test apps run self-contained).
- Leftovers kept for reference: `C:\Users\michael\dev\ApSmoke` (template app plus all candidate packages) and `C:\Users\michael\dev\uniffi-smoke`.
- The smoke package was unregistered and its dev cert (CN=AppPublisher, public password) was removed from LocalMachine\TrustedPeople.
- Logs: `/Users/michael/Clean/autopaper/.scratch/windows/*.out`.
