<#
.SYNOPSIS
Builds, checks, runs and packages the AutoPaper Windows app (apps\windows).

.DESCRIPTION
Run it INSIDE Windows, from a local copy of the repository (Windows can't register packages from the Parallels share;
memory-bank/techContext.md), after building the core:

    powershell -File scripts\build-core-windows.ps1            # autopaper_core.dll + C# bindings → apps\windows\Generated
    powershell -File scripts\windows-app.ps1 -Smoke -Test -Run  # bindings check, tests, then launch the app

Steps (any combination, in this order):

    -Smoke   apps\windows\CoreSmoke: opens an Engine on a temporary folder with the Demo providers, adds keywords, makes
             and renders one wallpaper, through both callback interfaces (SecretStore, ProgressObserver).
    -Test    apps\windows\AutoPaper.Tests (dotnet test): the Credential Manager store, the core reading keys
             through it, the core's moods and progress detail through the bindings, and the app's wording against
             its own strings. Uses its own credential prefix, never the app's.
    -Run     Builds Debug and registers + launches the app with package identity (winapp run): notifications,
             StartupTask and ApplicationData need it. Add -Clean to start from empty app data (first run again).
             Fails unless AutoPaper's window appears within -LaunchTimeout seconds (default 30), printing the
             app log's last lines, so a start-up failure can't pass as a successful run.
    -Pack    Publishes Release (self-contained, ReadyToRun, never trimmed) and packs a signed MSIX into
             apps\windows\AppPackages (git-ignored) with a development certificate, created there on first use with a
             random password kept beside it. Installing it needs the certificate trusted once, as an administrator:
                 Import-Certificate apps\windows\AppPackages\AutoPaper-dev.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
                 Add-AppxPackage apps\windows\AppPackages\AutoPaper_<version>_<arch>.msix

The embedding model (models\bge-small-en-v1.5, scripts/fetch-model.sh) is packaged when present; without it the
app's memory runs in reduced mode and Settings > Memory says so.

.PARAMETER Arch
arm64 or x64. Default: this machine's architecture. (x64 builds and packs on an ARM64 machine too.)

.PARAMETER LaunchTimeout
With -Run: seconds to wait for AutoPaper's window before failing (5-300, default 30).
#>
[CmdletBinding()]
param(
    [switch]$Smoke,
    [switch]$Test,
    [switch]$Run,
    [switch]$Clean,
    [switch]$Pack,
    [ValidateSet('arm64', 'x64', '')]
    [string]$Arch = '',
    [ValidateRange(5, 300)]
    [int]$LaunchTimeout = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).ProviderPath
$Windows = Join-Path $Repo 'apps\windows'
$App = Join-Path $Windows 'AutoPaper'
$Packages = Join-Path $Windows 'AppPackages'
if (-not $Arch) {
    $Arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
}
$Platform = if ($Arch -eq 'arm64') { 'ARM64' } else { 'x64' }
$Rid = "win-$Arch"

function Write-Step([string]$Message) {
    Write-Host ''
    Write-Host "==> $Message"
}

# Native commands run without 2>&1 (Windows PowerShell 5.1 turns redirected stderr into terminating errors under
# ErrorActionPreference Stop); a non-zero exit code is an error.
function Invoke-Native([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "$Exe $($Arguments -join ' ') failed (exit code $LASTEXITCODE)"
    }
}

if (-not ($Smoke -or $Test -or $Run -or $Pack)) {
    throw 'Nothing to do: pass -Smoke, -Test, -Run and/or -Pack (Get-Help scripts\windows-app.ps1 -Detailed).'
}
if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) { throw '.NET SDK 10 is needed (https://dot.net).' }
if (-not (Test-Path (Join-Path $Windows 'Generated\autopaper_core.cs'))) {
    throw 'The core bindings are missing: run scripts\build-core-windows.ps1 first.'
}
if (-not (Test-Path (Join-Path $Windows "Generated\$Rid\autopaper_core.dll"))) {
    throw "autopaper_core.dll for $Rid is missing: run scripts\build-core-windows.ps1 -Arch $Arch."
}
if ($Repo.StartsWith('\\') -and ($Run -or $Pack)) {
    throw "The repository is on a network share ($Repo); Windows can't register packages from there. Copy it to a local disk."
}

if ($Smoke) {
    Write-Step "Bindings check ($Rid)"
    $model = Join-Path $Repo 'models\bge-small-en-v1.5'
    $project = Join-Path $Windows 'CoreSmoke\CoreSmoke.csproj'
    Invoke-Native 'dotnet' @('build', $project, '-c', 'Release', '-r', $Rid, '-nologo', '-v', 'q')
    Invoke-Native 'dotnet' @('run', '--project', $project, '-c', 'Release', '-r', $Rid, '--no-build', '--', $model)
}

if ($Test) {
    Write-Step 'Tests'
    Invoke-Native 'dotnet' @('test', (Join-Path $Windows 'AutoPaper.Tests\AutoPaper.Tests.csproj'), '-c', 'Debug', '-r', $Rid, '-nologo')
}

if ($Run) {
    if (-not (Get-Command winapp -ErrorAction SilentlyContinue)) {
        throw 'The winapp CLI is needed: winget install --id Microsoft.WinAppCli --exact'
    }
    Write-Step "Building AutoPaper (Debug, $Platform)"
    Invoke-Native 'dotnet' @('build', (Join-Path $App 'AutoPaper.csproj'), '-c', 'Debug', "-p:Platform=$Platform", '-nologo')
    Write-Step 'Registering and launching'
    Get-Process AutoPaper -ErrorAction SilentlyContinue | Stop-Process -Force
    $arguments = @('run', (Join-Path $App 'AutoPaper.csproj'), '--arch', $Arch, '--no-build', '--detach')
    if ($Clean) { $arguments += '--clean' }
    Invoke-Native 'winapp' $arguments

    # Launch check: a start-up failure must not pass as "built and ran". AutoPaper shows its WinUI window on a normal
    # launch; a failure before that shows Windows' message box (a #32770 dialog, also titled AutoPaper) and exits on
    # OK, so only the WinUI window class counts.
    Write-Step 'Checking that AutoPaper opened its window'
    Add-Type -Namespace AutoPaperLaunch -Name Native -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, System.Text.StringBuilder name, int max);
'@
    function Get-WindowClass([IntPtr]$Handle) {
        $name = New-Object System.Text.StringBuilder 256
        [void][AutoPaperLaunch.Native]::GetClassName($Handle, $name, $name.Capacity)
        $name.ToString()
    }
    $deadline = (Get-Date).AddSeconds($LaunchTimeout)
    $window = $null
    $failed = $false
    while ((Get-Date) -lt $deadline -and -not $window -and -not $failed) {
        Start-Sleep -Milliseconds 500
        foreach ($process in @(Get-Process AutoPaper -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 })) {
            switch (Get-WindowClass $process.MainWindowHandle) {
                'WinUIDesktopWin32WindowClass' { $window = $process }
                '#32770' { $failed = $true }
            }
        }
    }
    if (-not $window) {
        $log = Join-Path $env:LOCALAPPDATA 'Packages'
        $log = Get-ChildItem $log -Directory -Filter 'AutoPaper_*' -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName 'LocalState\logs\autopaper.log' } | Where-Object { Test-Path $_ } | Select-Object -First 1
        if ($log) { Get-Content $log -Tail 5 | Out-Host }
        if ($failed) { throw "AutoPaper couldn't start: it's showing its start-up error (see the log above)." }
        throw "AutoPaper didn't open a window within $LaunchTimeout s (see the log above, if any)."
    }
    Write-Host "AutoPaper is running (process $($window.Id), window '$($window.MainWindowTitle)')."
}

if ($Pack) {
    if (-not (Get-Command winapp -ErrorAction SilentlyContinue)) {
        throw 'The winapp CLI is needed: winget install --id Microsoft.WinAppCli --exact'
    }
    New-Item -ItemType Directory -Force $Packages | Out-Null

    # A development certificate whose subject matches the manifest's Publisher (winapp reads it from there).
    $pfx = Join-Path $Packages 'AutoPaper-dev.pfx'
    $passwordFile = Join-Path $Packages 'AutoPaper-dev.password.txt'
    if (-not (Test-Path $pfx)) {
        Write-Step 'Creating a development signing certificate'
        $bytes = New-Object byte[] 24
        [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        [System.IO.File]::WriteAllText($passwordFile, [Convert]::ToBase64String($bytes))
        Invoke-Native 'winapp' @('cert', 'generate', '--manifest', (Join-Path $App 'Package.appxmanifest'),
            '--output', $pfx, '--export-cer', '--password', (Get-Content $passwordFile -Raw).Trim())
        $cer = [System.IO.Path]::ChangeExtension($pfx, '.cer')
        if (-not (Test-Path $cer)) { throw "winapp didn't export $cer" }
    }
    $password = (Get-Content $passwordFile -Raw).Trim()

    [xml]$manifest = Get-Content (Join-Path $App 'Package.appxmanifest')
    $version = $manifest.Package.Identity.Version
    $msix = Join-Path $Packages "AutoPaper_${version}_$Arch.msix"
    Write-Step "Publishing (Release, $Rid) and packing $msix"
    # Project mode: winapp publishes the project (self-contained, ReadyToRun; the project never trims) and signs.
    Invoke-Native 'winapp' @('pack', (Join-Path $App 'AutoPaper.csproj'), '-c', 'Release', '--arch', $Arch,
        '--cert', $pfx, '--cert-password', $password, '--output', $msix)
    Get-Item $msix | ForEach-Object { '{0:N0} bytes  {1}' -f $_.Length, $_.FullName }
}

Write-Step 'Done'
