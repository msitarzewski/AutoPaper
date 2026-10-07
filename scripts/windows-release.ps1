<#
.SYNOPSIS
Builds, packs and signs AutoPaper for Windows: one x64 + arm64 .msixbundle, its App Installer file (the update feed,
served from msitarzewski.com) and its winget manifests.

.DESCRIPTION
Run it INSIDE Windows (the "Windows 11" VM), from a local copy of the repository (Windows can't build packages
reliably from the Parallels share; memory-bank/techContext.md), with Windows PowerShell 5.1:

    robocopy \\Mac\Home\Clean\autopaper C:\Users\michael\dev\autopaper-release /MIR /XD .git bin obj target AppX AppPackages .scratch _site build /XF .env .env.*
    $env:AUTOPAPER_SIGN_ENDPOINT = 'https://eus.codesigning.azure.net/'
    $env:AUTOPAPER_SIGN_ACCOUNT  = '<Artifact Signing account>'
    $env:AUTOPAPER_SIGN_PROFILE  = '<certificate profile>'
    az login                                  # or AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET
    powershell -File scripts\windows-release.ps1 -Version 0.1.0 -CopyTo \\Mac\Home\Clean\autopaper

then, on the Mac, scripts/publish-windows.sh 0.1.0 puts the bundle and the App Installer file on the update server.

Steps, in order:

    1. The core for arm64 and x64 (scripts\build-core-windows.ps1 -Arch all; both are required here), unless -SkipCore.
    2. AutoPaper published for win-x64 and win-arm64 (Release, self-contained .NET and Windows App SDK, ReadyToRun,
       never trimmed) and packed by MSBuild into one unsigned .msix per architecture.
    3. Each package's AppxManifest.xml is given the release's Identity: Version <Version>.0 and Publisher = the signing
       certificate's exact subject (an MSIX installs only when they're equal). The repository's
       Package.appxmanifest is never edited: the change is made in an unpacked copy, which MakeAppx packs again.
    4. Each package signed, then bundled (MakeAppx) into build\windows\<Version>\AutoPaper_<Version>_x64_arm64.msixbundle,
       which is signed and verified (SignTool; the signer must be the packages' Publisher).
    5. build\windows\<Version>\AutoPaper.appinstaller, the update feed, next to the bundle (see "Updates" below), and
       packaging\winget\manifests\m\msitarzewski\AutoPaper\<Version>\ (three files, schema 1.12.0, checked with winget
       validate when winget is installed), whose installer is the copy of the bundle attached to the GitHub release
       v<Version>. Nothing is published or submitted anywhere: scripts/publish-windows.sh (on the Mac) uploads the feed
       and the bundle, and the GitHub release and winget-pkgs are done by hand.

Signing (Azure Artifact Signing, Microsoft's documented MSIX route: SignTool with the Artifact Signing dlib,
https://learn.microsoft.com/windows/msix/package/sign-msix-package-guide):

    AUTOPAPER_SIGN_ENDPOINT   The account's regional endpoint, e.g. https://eus.codesigning.azure.net/ (East US). It must
                              be the region the account and its certificate profile were created in (else 403).
    AUTOPAPER_SIGN_ACCOUNT    The Artifact Signing account's name.
    AUTOPAPER_SIGN_PROFILE    The certificate profile's name (Public Trust).
    AUTOPAPER_SIGN_PUBLISHER  Optional: the certificate's exact subject, when the Azure CLI can't read it (see below).

    Authentication is the dlib's DefaultAzureCredential, limited here to two sources: environment variables for a
    service principal (AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET) or the Azure CLI's login (az login). The
    identity needs the "Artifact Signing Certificate Profile Signer" role on the profile. Secrets are never printed.

    The MSIX Publisher must equal the certificate's subject exactly ("CN=..., O=..., L=..., S=..., C=..."). It's read from the
    certificate profile (properties.certificates[].subjectName, through az rest), or taken from -Publisher /
    AUTOPAPER_SIGN_PUBLISHER (copy it from the profile's page in the Azure portal). After signing, the bundle's signer
    is compared with it, so a mismatch fails here, not on someone's PC.

    The dlib exists only for x64 (and x86): x64 SignTool runs it, on ARM64 Windows under emulation, and it needs the
    x64 .NET runtime 8 or later (on ARM64 Windows it lives in C:\Program Files\dotnet\x64). The tools come from NuGet,
    pinned and checked by SHA-256, into build\windows\tools: Microsoft.Windows.SDK.BuildTools (SignTool, MakeAppx) and
    Microsoft.ArtifactSigning.Client (the dlib). Signatures are timestamped (http://timestamp.acs.microsoft.com):
    Artifact Signing's certificates are valid for 72 hours, the timestamp keeps the signature valid after that.

-DevCert signs with AutoPaper's self-signed development certificate instead (apps\windows\AppPackages\AutoPaper-dev.pfx
and its password file, made by scripts\windows-app.ps1 -Pack, or here on first use), for testing installs and
upgrades before the Artifact Signing account exists. Its App Installer file and winget manifests say they're a
development build; a release run with Artifact Signing replaces them.

Updates work like Sparkle on the Mac, from AutoPaper's own server (not GitHub, whose downloads redirect and come back
as application/octet-stream): https://msitarzewski.com/app-updates/autopaper/ (Caddy on pipx; scripts/pipx-app-updates.sh)

    AutoPaper.appinstaller                              the feed (application/appinstaller, Cache-Control: no-cache);
                                                        its own Uri is this address, so a downloaded copy always
                                                        defers to the live one
    windows/AutoPaper_<Version>_x64_arm64.msixbundle    the bundle it points to (application/msixbundle). The name is
                                                        versioned, so a new release never replaces a file someone is
                                                        still downloading; older ones are kept for a while

A person who installs from AutoPaper.appinstaller (downloaded and opened; the ms-appinstaller: link is off by default
since December 2023) gets the package with the feed's address kept, and Windows checks it on every launch of AutoPaper
(HoursBetweenUpdateChecks 0) and every 8 hours in the background, updating silently (desktop apps get no prompt; the
new version is there by the next launch). winget installs update through winget, from the GitHub release.
-UpdatesUrl, -FeedName and -BundleFolder change where the feed says things are (a test feed in a test folder).

The Windows App SDK is packaged with the app (WindowsAppSDKSelfContained), so the bundle installs on its own: a
framework-dependent package needs Microsoft.WindowsAppRuntime.2 installed first, which only the Store provides
automatically ("For packaged apps that are not distributed through the Store, you as the developer are responsible
for distributing the Framework package"); AutoPaper's app notifications don't need the runtime's Singleton package.
-FrameworkDependent builds the smaller framework-dependent packages instead (for testing only).

.PARAMETER Version
The release version, three numbers (0.1.0). The packages' version is <Version>.0; the GitHub release's tag is
v<Version>.

.PARAMETER DevCert
Sign with the development certificate (testing), not Artifact Signing.

.PARAMETER SkipCore
Use the core already in apps\windows\Generated (both architectures must be there).

.PARAMETER Publisher
The signing certificate's exact subject, overriding what's read from the certificate profile (Artifact Signing).

.PARAMETER CopyTo
A repository folder to copy the release into after a successful run, e.g. \\Mac\Home\Clean\autopaper: the bundle and
the App Installer file to build\windows\<Version>\ there (git-ignored; scripts/publish-windows.sh uploads them from
there), and the winget manifests to packaging\winget\.

.PARAMETER UpdatesUrl
The update server's address for AutoPaper, ending in /. Default: https://msitarzewski.com/app-updates/autopaper/

.PARAMETER FeedName
The App Installer file's name there. Default: AutoPaper.appinstaller (test builds use their own, e.g. test.appinstaller).

.PARAMETER BundleFolder
The folder there that holds the bundles. Default: windows (test builds use their own, e.g. windows-test).

.PARAMETER TargetDir
Cargo's target directory for the core (passed to build-core-windows.ps1); must be on a local disk.

.PARAMETER FrameworkDependent
Leave the Windows App SDK out of the packages (they then need Microsoft.WindowsAppRuntime.2 installed). Testing only.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d{1,5}\.\d{1,5}\.\d{1,5}$')]
    [string]$Version,
    [switch]$DevCert,
    [switch]$SkipCore,
    [string]$Publisher = '',
    [string]$CopyTo = '',
    [string]$TargetDir = '',
    [switch]$FrameworkDependent,
    [ValidatePattern('^https://[^\s?#]+/$')]
    [string]$UpdatesUrl = 'https://msitarzewski.com/app-updates/autopaper/',
    [ValidatePattern('^[A-Za-z0-9._-]+\.appinstaller$')]
    [string]$FeedName = 'AutoPaper.appinstaller',
    [ValidatePattern('^[A-Za-z0-9._-]+$')]
    [string]$BundleFolder = 'windows'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.IO.Compression.FileSystem

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).ProviderPath
$Windows = Join-Path $Repo 'apps\windows'
$Project = Join-Path $Windows 'AutoPaper\AutoPaper.csproj'
$Generated = Join-Path $Windows 'Generated'
$PackageVersion = "$Version.0"
$BundleName = "AutoPaper_${Version}_x64_arm64.msixbundle"
$Output = Join-Path $Repo "build\windows\$Version"
$Stage = Join-Path $Output 'stage'
$Tools = Join-Path $Repo 'build\windows\tools'
$Architectures = @('x64', 'arm64')
$PackageName = 'AutoPaper'

# Where the release lives: the website and the GitHub release (winget's installer), and the update server (the App
# Installer feed and the bundle it points to; scripts/publish-windows.sh uploads them, scripts/pipx-app-updates.sh
# set the server up).
$Owner = 'msitarzewski'
$Site = 'https://msitarzewski.github.io/AutoPaper'
$ReleaseBundleUri = "https://github.com/$Owner/AutoPaper/releases/download/v$Version/$BundleName"
$AppInstallerUri = "$UpdatesUrl$FeedName"
$BundleUri = "$UpdatesUrl$BundleFolder/$BundleName"
$TimestampUrl = 'http://timestamp.acs.microsoft.com'

# Pinned tools (NuGet), checked by the SHA-256 of their .nupkg.
$SdkBuildTools = @{ Id = 'Microsoft.Windows.SDK.BuildTools'; Version = '10.0.28000.2705'; Sha256 = '8BFDFB6CA2633F531CF80B5FA22512BA61A394D7988F0970DB83BAADC67929ED' }
$SigningClient = @{ Id = 'Microsoft.ArtifactSigning.Client'; Version = '1.0.128'; Sha256 = '74BD7D27E6CE1051409C38D9B46BC8DF0400ECD643D51FFBF2AC00869061E40B' }

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

# A native command's output (stdout and stderr) as text, with its exit code in $LASTEXITCODE: for commands whose
# failure is reported here rather than thrown (stderr isn't an error in itself).
function Invoke-Captured([string]$Exe, [string[]]$Arguments) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        return (& $Exe @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
    } finally {
        $ErrorActionPreference = $previous
    }
}

# MakeAppx lists every file it reads or writes: only its last line is shown, unless it fails.
function Invoke-MakeAppx([string[]]$Arguments) {
    $output = Invoke-Captured $MakeAppx $Arguments
    if ($LASTEXITCODE -ne 0) {
        Write-Host $output
        throw "makeappx $($Arguments -join ' ') failed (exit code $LASTEXITCODE)"
    }
    Write-Host (($output -split "`n" | Where-Object { $_.Trim() }) | Select-Object -Last 1)
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToUpperInvariant()
}

function Write-Utf8([string]$Path, [string]$Text) {
    New-Item -ItemType Directory -Force (Split-Path $Path) | Out-Null
    $Text = $Text.Replace("`r`n", "`n")
    if (-not $Text.EndsWith("`n")) { $Text += "`n" }
    [System.IO.File]::WriteAllText($Path, $Text, (New-Object System.Text.UTF8Encoding $false))
}

# A NuGet package's contents, downloaded once into build\windows\tools and checked against its pinned SHA-256.
function Get-NuGetTool([hashtable]$Package) {
    $folder = Join-Path $Tools "$($Package.Id).$($Package.Version)"
    $marker = Join-Path $folder '.verified'
    if (Test-Path $marker) { return $folder }
    New-Item -ItemType Directory -Force $Tools | Out-Null
    $id = $Package.Id.ToLowerInvariant()
    $nupkg = Join-Path $Tools "$id.$($Package.Version).zip"
    $url = "https://api.nuget.org/v3-flatcontainer/$id/$($Package.Version)/$id.$($Package.Version).nupkg"
    Write-Host "Downloading $($Package.Id) $($Package.Version)"
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $nupkg
    $hash = Get-Sha256 $nupkg
    if ($hash -ne $Package.Sha256) {
        Remove-Item $nupkg -Force
        throw "$($Package.Id) $($Package.Version) has SHA-256 $hash, expected $($Package.Sha256): not using it."
    }
    if (Test-Path $folder) { Remove-Item $folder -Recurse -Force }
    [System.IO.Compression.ZipFile]::ExtractToDirectory($nupkg, $folder)
    Remove-Item $nupkg -Force
    Set-Content -LiteralPath $marker -Value $hash
    return $folder
}

# The 13-character publisher ID in a package family name: the first 8 bytes of the SHA-256 of the publisher (UTF-16LE),
# a zero bit appended, in Crockford-style base32 (checked: CN=Michael Sitarzewski -> nef1hbbyd9msy, Microsoft's
# publisher -> 8wekyb3d8bbwe).
function Get-PublisherId([string]$Subject) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $bytes = $sha.ComputeHash([System.Text.Encoding]::Unicode.GetBytes($Subject))
    $bits = (($bytes[0..7] | ForEach-Object { [Convert]::ToString($_, 2).PadLeft(8, '0') }) -join '') + '0'
    $alphabet = '0123456789abcdefghjkmnpqrstvwxyz'
    $id = ''
    for ($i = 0; $i -lt 65; $i += 5) { $id += $alphabet[[Convert]::ToInt32($bits.Substring($i, 5), 2)] }
    return $id
}

function Read-ZipEntry([string]$Zip, [string]$Name) {
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        $entry = $archive.Entries | Where-Object { $_.FullName -eq $Name } | Select-Object -First 1
        if (-not $entry) { throw "$Name isn't in $Zip" }
        $stream = $entry.Open()
        try {
            $memory = New-Object System.IO.MemoryStream
            $stream.CopyTo($memory)
            return , $memory.ToArray()
        } finally { $stream.Dispose() }
    } finally { $archive.Dispose() }
}

function Get-BytesSha256([byte[]]$Bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    return ([BitConverter]::ToString($sha.ComputeHash($Bytes)) -replace '-', '').ToUpperInvariant()
}

function ConvertTo-XmlAttribute([string]$Text) { [System.Security.SecurityElement]::Escape($Text) }

# The x64 .NET runtime (8 or later) the Artifact Signing dlib runs on (x64 SignTool hosts it).
function Test-X64DotNet {
    $roots = @()
    if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
        $roots += Join-Path $env:ProgramFiles 'dotnet\x64'
        $registered = Get-ItemProperty 'HKLM:\SOFTWARE\dotnet\Setup\InstalledVersions\x64' -Name InstallLocation -ErrorAction SilentlyContinue
        if ($registered) { $roots += $registered.InstallLocation }
    } else {
        $roots += Join-Path $env:ProgramFiles 'dotnet'
    }
    foreach ($root in $roots) {
        $shared = Join-Path $root 'shared\Microsoft.NETCore.App'
        if (Test-Path $shared) {
            foreach ($version in Get-ChildItem $shared -Directory) {
                if ([int]($version.Name.Split('.')[0]) -ge 8) { return $true }
            }
        }
    }
    return $false
}

# ── Checks and signing identity ───────────────────────────────────────────────────────────────

if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) { throw '.NET SDK 10 is needed (https://dot.net).' }
if ($Repo.StartsWith('\\')) {
    throw "The repository is on a network share ($Repo). Copy it to a local disk first (see Get-Help .\scripts\windows-release.ps1)."
}

$SignToolArch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
$Signing = @{}
if ($DevCert) {
    $pfx = Join-Path $Windows 'AppPackages\AutoPaper-dev.pfx'
    $passwordFile = Join-Path $Windows 'AppPackages\AutoPaper-dev.password.txt'
    if (-not (Test-Path $pfx)) {
        if (-not (Get-Command winapp -ErrorAction SilentlyContinue)) {
            throw 'The development certificate is missing and the winapp CLI to make it isn''t installed: winget install --id Microsoft.WinAppCli --exact'
        }
        Write-Step 'Creating the development signing certificate'
        New-Item -ItemType Directory -Force (Split-Path $pfx) | Out-Null
        $bytes = New-Object byte[] 24
        [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        [System.IO.File]::WriteAllText($passwordFile, [Convert]::ToBase64String($bytes))
        Invoke-Native 'winapp' @('cert', 'generate', '--manifest', (Join-Path $Windows 'AutoPaper\Package.appxmanifest'),
            '--output', $pfx, '--export-cer', '--password', (Get-Content $passwordFile -Raw).Trim())
    }
    $password = (Get-Content $passwordFile -Raw).Trim()
    $certificate = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($pfx, $password)
    $Signing.Subject = $certificate.Subject
    $Signing.Thumbprint = $certificate.Thumbprint
    $Signing.Pfx = $pfx
    $Signing.Password = $password
    $Signing.Kind = 'development certificate'
} else {
    foreach ($name in 'AUTOPAPER_SIGN_ENDPOINT', 'AUTOPAPER_SIGN_ACCOUNT', 'AUTOPAPER_SIGN_PROFILE') {
        if (-not [Environment]::GetEnvironmentVariable($name)) {
            throw "$name isn't set. Artifact Signing needs AUTOPAPER_SIGN_ENDPOINT, AUTOPAPER_SIGN_ACCOUNT and AUTOPAPER_SIGN_PROFILE (Get-Help .\scripts\windows-release.ps1 -Detailed), or pass -DevCert to test with the development certificate."
        }
    }
    $endpoint = $env:AUTOPAPER_SIGN_ENDPOINT
    if (-not $endpoint.StartsWith('https://')) { throw "AUTOPAPER_SIGN_ENDPOINT must be an https:// address, like https://eus.codesigning.azure.net/" }
    if (-not (Test-X64DotNet)) {
        throw ('The Artifact Signing dlib needs the x64 .NET runtime 8 or later (x64 SignTool loads it' +
            $(if ($SignToolArch -eq 'arm64') { ', under emulation on this ARM64 PC' } else { '' }) +
            '). Install it, then run this again: winget install --id Microsoft.DotNet.Runtime.8 --architecture x64')
    }
    $subject = if ($Publisher) { $Publisher } elseif ($env:AUTOPAPER_SIGN_PUBLISHER) { $env:AUTOPAPER_SIGN_PUBLISHER } else { '' }
    if (-not $subject) {
        if (-not (Get-Command az -ErrorAction SilentlyContinue)) {
            throw ('The certificate''s subject is needed for the MSIX Publisher, and the Azure CLI (to read it from the ' +
                'certificate profile) isn''t installed. Install it (winget install --id Microsoft.AzureCLI --exact) and ' +
                'az login, or set AUTOPAPER_SIGN_PUBLISHER to the subject shown on the certificate profile in the Azure portal.')
        }
        Write-Step "Reading the certificate's subject from certificate profile $($env:AUTOPAPER_SIGN_PROFILE)"
        $accountId = Invoke-Captured 'az' @('resource', 'list', '--resource-type', 'Microsoft.CodeSigning/codeSigningAccounts',
            '--name', $env:AUTOPAPER_SIGN_ACCOUNT, '--query', '[0].id', '-o', 'tsv')
        if ($LASTEXITCODE -ne 0 -or $accountId -notmatch '^/subscriptions/') {
            throw "The Azure CLI didn't find Artifact Signing account $($env:AUTOPAPER_SIGN_ACCOUNT) (az login, and az account set --subscription <its subscription>), or set AUTOPAPER_SIGN_PUBLISHER."
        }
        $subjects = @()
        foreach ($apiVersion in '2025-10-13', '2024-09-30-preview') {
            $answer = Invoke-Captured 'az' @('rest', '--method', 'get', '--url',
                "https://management.azure.com$($accountId.Trim())/certificateProfiles/$($env:AUTOPAPER_SIGN_PROFILE)?api-version=$apiVersion",
                '--query', 'properties.certificates[].subjectName', '-o', 'tsv')
            if ($LASTEXITCODE -eq 0) {
                $subjects = @($answer -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -match '^CN=' } | Select-Object -Unique)
                if ($subjects.Count -gt 0) { break }
            }
        }
        if ($subjects.Count -ne 1) {
            throw "Certificate profile $($env:AUTOPAPER_SIGN_PROFILE) gave $($subjects.Count) subjects ($($subjects -join ' | ')); set AUTOPAPER_SIGN_PUBLISHER to the one to use."
        }
        $subject = $subjects[0]
    }
    $Signing.Subject = $subject.Trim()
    $Signing.Kind = 'Azure Artifact Signing'
}
if ($Signing.Subject -notmatch '^CN=') { throw "The publisher '$($Signing.Subject)' isn't a certificate subject (CN=...)." }
if ($Signing.Subject -match '[^\x20-\x7E]') {
    throw "The publisher '$($Signing.Subject)' has characters outside ASCII, which App Installer files can't carry."
}
$PublisherId = Get-PublisherId $Signing.Subject
$FamilyName = "${PackageName}_$PublisherId"
Write-Host "AutoPaper $Version (package $PackageVersion), signed with the $($Signing.Kind)"
Write-Host "Publisher: $($Signing.Subject)"
Write-Host "Package family: $FamilyName"

# ── Tools ─────────────────────────────────────────────────────────────────────────────────────

Write-Step 'Tools'
$buildTools = Get-NuGetTool $SdkBuildTools
$sdkBin = Get-ChildItem (Join-Path $buildTools 'bin') -Directory | Sort-Object Name -Descending | Select-Object -First 1
$MakeAppx = Join-Path $sdkBin.FullName "$SignToolArch\makeappx.exe"
$SignTool = Join-Path $sdkBin.FullName "$SignToolArch\signtool.exe"
if (-not $DevCert) {
    # The dlib is x64 only: x64 SignTool, under emulation on ARM64.
    $SignTool = Join-Path $sdkBin.FullName 'x64\signtool.exe'
    $Dlib = Join-Path (Get-NuGetTool $SigningClient) 'bin\x64\Azure.CodeSigning.Dlib.dll'
    if (-not (Test-Path $Dlib)) { throw "$Dlib is missing from $($SigningClient.Id) $($SigningClient.Version)" }
}
foreach ($tool in $MakeAppx, $SignTool) { if (-not (Test-Path $tool)) { throw "$tool is missing" } }
Write-Host "MakeAppx: $MakeAppx"
Write-Host "SignTool: $SignTool"

# Signs one package or the bundle (the packages first, then the bundle, as Visual Studio does), timestamped when it's
# Artifact Signing.
function Invoke-Sign([string]$Path) {
    if ($DevCert) {
        Invoke-Native $SignTool @('sign', '/fd', 'SHA256', '/f', $Signing.Pfx, '/p', $Signing.Password, $Path)
        return
    }
    $metadataFile = Join-Path $Stage 'artifact-signing.json'
    if (-not (Test-Path $metadataFile)) {
        # DefaultAzureCredential, limited to a service principal's environment variables and the Azure CLI's login.
        $metadataJson = [ordered]@{
            Endpoint = $env:AUTOPAPER_SIGN_ENDPOINT
            CodeSigningAccountName = $env:AUTOPAPER_SIGN_ACCOUNT
            CertificateProfileName = $env:AUTOPAPER_SIGN_PROFILE
            ExcludeCredentials = @('ManagedIdentityCredential', 'WorkloadIdentityCredential', 'SharedTokenCacheCredential',
                'VisualStudioCredential', 'VisualStudioCodeCredential', 'AzurePowerShellCredential',
                'AzureDeveloperCliCredential', 'InteractiveBrowserCredential')
        } | ConvertTo-Json
        Write-Utf8 $metadataFile $metadataJson
    }
    Invoke-Native $SignTool @('sign', '/v', '/fd', 'SHA256', '/tr', $TimestampUrl, '/td', 'SHA256',
        '/dlib', $Dlib, '/dmdf', $metadataFile, $Path)
}

# ── Core ──────────────────────────────────────────────────────────────────────────────────────

if (-not $SkipCore) {
    Write-Step 'Building the core (arm64 and x64)'
    $coreArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $Repo 'scripts\build-core-windows.ps1'), '-Arch', 'all')
    if ($TargetDir) { $coreArgs += @('-TargetDir', $TargetDir) }
    Invoke-Native 'powershell' $coreArgs
}
foreach ($arch in $Architectures) {
    if (-not (Test-Path (Join-Path $Generated "win-$arch\autopaper_core.dll"))) {
        throw "apps\windows\Generated\win-$arch\autopaper_core.dll is missing: a release needs the core for both architectures (scripts\build-core-windows.ps1 -Arch all)."
    }
}
if (-not (Test-Path (Join-Path $Generated 'autopaper_core.cs'))) { throw 'apps\windows\Generated\autopaper_core.cs is missing (scripts\build-core-windows.ps1).' }
if (-not (Test-Path (Join-Path $Repo 'models\bge-small-en-v1.5\model.safetensors'))) {
    Write-Warning 'The embedding model (models\bge-small-en-v1.5, scripts/fetch-model.sh) is missing: this build''s memory would run in reduced mode.'
    if (-not $DevCert) { throw 'A release must ship the embedding model: fetch it first (scripts/fetch-model.sh).' }
}

# ── Packages ──────────────────────────────────────────────────────────────────────────────────

if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
$bundleInput = Join-Path $Stage 'bundle'
New-Item -ItemType Directory -Force $bundleInput | Out-Null
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'

foreach ($arch in $Architectures) {
    $platform = if ($arch -eq 'arm64') { 'ARM64' } else { 'x64' }
    $built = Join-Path $Stage "msbuild-$arch\"
    Write-Step "Publishing and packing AutoPaper for win-$arch"
    $publish = @('publish', $Project, '-c', 'Release', '-r', "win-$arch", "-p:Platform=$platform",
        '-p:PublishReadyToRun=true', '-p:PublishTrimmed=false', '-p:SelfContained=true',
        '-p:GenerateAppxPackageOnBuild=true', '-p:AppxPackageSigningEnabled=false', '-p:AppxBundle=Never',
        '-p:UapAppxPackageBuildMode=SideloadOnly', '-p:AppxSymbolPackageEnabled=false', '-p:DebugType=None', '-p:DebugSymbols=false', "-p:AppxPackageDir=$built", '-nologo')
    if (-not $FrameworkDependent) { $publish += '-p:WindowsAppSDKSelfContained=true' }
    Invoke-Native 'dotnet' $publish
    $msix = Get-ChildItem $built -Recurse -Filter '*.msix' | Where-Object { $_.Name -like "*_$arch.msix" } | Select-Object -First 1
    if (-not $msix) { throw "MSBuild made no .msix for $arch in $built" }

    # The release's identity, in an unpacked copy of the package.
    $layout = Join-Path $Stage "layout-$arch"
    Invoke-MakeAppx @('unpack', '/p', $msix.FullName, '/d', $layout, '/o', '/nv')
    foreach ($generatedFile in 'AppxBlockMap.xml', '[Content_Types].xml', 'AppxSignature.p7x') {
        $path = Join-Path $layout $generatedFile
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    $metadata = Join-Path $layout 'AppxMetadata'
    if (Test-Path $metadata) { Remove-Item $metadata -Recurse -Force }
    $manifestPath = Join-Path $layout 'AppxManifest.xml'
    [xml]$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8
    $identity = $manifest.Package.Identity
    if ($identity.Name -ne $PackageName) { throw "The package is named $($identity.Name), not $PackageName" }
    if ($identity.ProcessorArchitecture -ne $arch) { throw "The $arch package says it's for $($identity.ProcessorArchitecture)" }
    $identity.SetAttribute('Publisher', $Signing.Subject)
    $identity.SetAttribute('Version', $PackageVersion)
    $runtime = @($manifest.Package.Dependencies.ChildNodes | Where-Object { $_.LocalName -eq 'PackageDependency' -and $_.Name -like 'Microsoft.WindowsAppRuntime*' })
    if (-not $FrameworkDependent -and $runtime.Count -gt 0) {
        throw "The $arch package still depends on $($runtime[0].Name): the Windows App SDK wasn't packaged with it (WindowsAppSDKSelfContained)."
    }
    $settings = New-Object System.Xml.XmlWriterSettings
    $settings.Encoding = New-Object System.Text.UTF8Encoding $false
    $settings.Indent = $true
    $writer = [System.Xml.XmlWriter]::Create($manifestPath, $settings)
    try { $manifest.Save($writer) } finally { $writer.Dispose() }
    $package = Join-Path $bundleInput "AutoPaper_${Version}_$arch.msix"
    Invoke-MakeAppx @('pack', '/d', $layout, '/p', $package, '/o', '/nv', '/h', 'SHA256')
    Invoke-Sign $package
    Write-Host ('{0:N0} bytes  {1}' -f (Get-Item $package).Length, $package)
}

Write-Step "Bundling $BundleName"
$Bundle = Join-Path $Output $BundleName
Invoke-MakeAppx @('bundle', '/d', $bundleInput, '/p', $Bundle, '/bv', $PackageVersion, '/o')

# ── Signing ───────────────────────────────────────────────────────────────────────────────────

Write-Step "Signing the bundle with the $($Signing.Kind)"
Invoke-Sign $Bundle

Write-Step 'Checking the signature'
$signature = Get-AuthenticodeSignature -LiteralPath $Bundle
if (-not $signature.SignerCertificate) { throw "$Bundle isn't signed." }
if ($signature.SignerCertificate.Subject -ne $Signing.Subject) {
    throw "The bundle is signed by '$($signature.SignerCertificate.Subject)', but its packages say Publisher '$($Signing.Subject)': Windows won't install it. Set AUTOPAPER_SIGN_PUBLISHER to the signer's subject."
}
[xml]$bundleManifest = [System.Text.Encoding]::UTF8.GetString((Read-ZipEntry $Bundle 'AppxMetadata/AppxBundleManifest.xml'))
if ($bundleManifest.Bundle.Identity.Publisher -ne $Signing.Subject -or $bundleManifest.Bundle.Identity.Version -ne $PackageVersion) {
    throw "The bundle's identity ($($bundleManifest.Bundle.Identity.Publisher), $($bundleManifest.Bundle.Identity.Version)) isn't the release's."
}
$verifyOutput = Invoke-Captured $SignTool @('verify', '/pa', '/v', $Bundle)
$verified = $LASTEXITCODE -eq 0
if ($verified) {
    Write-Host "signtool verify /pa: Successfully verified (signer $($signature.SignerCertificate.Subject), thumbprint $($signature.SignerCertificate.Thumbprint))"
} elseif ($DevCert -and $verifyOutput -match 'not trusted|untrusted') {
    # The development certificate is trusted for installing packages (TrustedPeople), not as a root for Authenticode.
    if ($signature.SignerCertificate.Thumbprint -ne $Signing.Thumbprint) { throw 'The bundle is signed by another certificate than the development one.' }
    Write-Host "signtool verify /pa: the signature is intact; its development certificate isn't a trusted root here (expected). Signer thumbprint $($signature.SignerCertificate.Thumbprint)."
} else {
    Write-Host $verifyOutput
    throw "signtool verify /pa failed for $Bundle"
}
if (-not $DevCert -and $verifyOutput -notmatch 'timestamp') { Write-Warning 'signtool verify didn''t mention a timestamp; check that the signature is timestamped.' }

# ── App Installer file and winget manifests ───────────────────────────────────────────────────

$InstallerSha256 = Get-Sha256 $Bundle
$SignatureSha256 = Get-BytesSha256 (Read-ZipEntry $Bundle 'AppxSignature.p7x')
$built = if ($DevCert) { ' DEVELOPMENT BUILD (development certificate): regenerate with Artifact Signing before publishing.' } else { '' }

Write-Step 'Writing the App Installer file'
$appInstaller = @"
<?xml version="1.0" encoding="UTF-8"?>
<!-- AutoPaper's update feed, made by scripts/windows-release.ps1 for v$Version and uploaded by scripts/publish-windows.sh.$built -->
<AppInstaller xmlns="http://schemas.microsoft.com/appx/appinstaller/2021" Version="$PackageVersion" Uri="$AppInstallerUri">
  <MainBundle Name="$PackageName" Publisher="$(ConvertTo-XmlAttribute $Signing.Subject)" Version="$PackageVersion" Uri="$BundleUri" />
  <UpdateSettings>
    <OnLaunch HoursBetweenUpdateChecks="0" />
    <AutomaticBackgroundTask />
  </UpdateSettings>
</AppInstaller>
"@
if ($appInstaller -match '[^\x09\x0A\x0D\x20-\x7E]') { throw 'The App Installer file must be ASCII only.' }
$feedFile = Join-Path $Output $FeedName
Write-Utf8 $feedFile $appInstaller
$null = [xml]$appInstaller

Write-Step 'Writing the winget manifests'
$wingetFolder = Join-Path $Repo "packaging\winget\manifests\m\$Owner\AutoPaper\$Version"
$identifier = "$Owner.AutoPaper"
$header = if ($DevCert) { "# DEVELOPMENT BUILD (development certificate): regenerate with Artifact Signing before submitting.`n" } else { '' }
$header += "# Made by scripts/windows-release.ps1 for v$Version. Not submitted anywhere by the script.`n"
$versionManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.12.0.schema.json
$header
PackageIdentifier: $identifier
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.12.0
"@
$localeManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.12.0.schema.json
$header
PackageIdentifier: $identifier
PackageVersion: $Version
PackageLocale: en-US
Publisher: Michael Sitarzewski
PublisherUrl: https://github.com/$Owner
PublisherSupportUrl: https://github.com/$Owner/AutoPaper/issues
PrivacyUrl: $Site/privacy.html
Author: Michael Sitarzewski
PackageName: AutoPaper
PackageUrl: $Site/
License: MIT
LicenseUrl: https://github.com/$Owner/AutoPaper/blob/main/LICENSE
Copyright: Copyright (c) 2026 Michael Sitarzewski
ShortDescription: New wallpapers from a few keywords, made by the AI you choose.
Description: |-
  Tell your computer what you'd like to see. AutoPaper makes new wallpapers from a few keywords, with the AI you
  choose (OpenAI, Google Gemini, or local models through Ollama and ComfyUI), and remembers what it has made so it
  doesn't repeat itself. It can set the lock screen too.
Moniker: autopaper
Tags:
- wallpaper
- desktop-background
- lock-screen
- ai
- image-generation
- comfyui
- ollama
ReleaseNotesUrl: https://github.com/$Owner/AutoPaper/releases/tag/v$Version
Documentations:
- DocumentLabel: Help
  DocumentUrl: $Site/help.html
ManifestType: defaultLocale
ManifestVersion: 1.12.0
"@
$installerEntries = ($Architectures | ForEach-Object {
@"
- Architecture: $_
  InstallerUrl: $ReleaseBundleUri
  InstallerSha256: $InstallerSha256
  SignatureSha256: $SignatureSha256
"@
}) -join "`n"
$installerManifest = @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.12.0.schema.json
$header
PackageIdentifier: $identifier
PackageVersion: $Version
Platform:
- Windows.Desktop
MinimumOSVersion: 10.0.22000.0
InstallerType: msix
PackageFamilyName: $FamilyName
ReleaseDate: $((Get-Date).ToString('yyyy-MM-dd'))
Installers:
$installerEntries
ManifestType: installer
ManifestVersion: 1.12.0
"@
if (Test-Path $wingetFolder) { Remove-Item $wingetFolder -Recurse -Force }
Write-Utf8 (Join-Path $wingetFolder "$identifier.yaml") $versionManifest
Write-Utf8 (Join-Path $wingetFolder "$identifier.locale.en-US.yaml") $localeManifest
Write-Utf8 (Join-Path $wingetFolder "$identifier.installer.yaml") $installerManifest
if (Get-Command winget -ErrorAction SilentlyContinue) {
    # Warnings are reported with a non-zero exit code too; only a failed validation stops the release.
    $validation = Invoke-Captured 'winget' @('validate', '--manifest', $wingetFolder, '--disable-interactivity')
    Write-Host $validation
    if ($validation -notmatch 'validation succeeded') { throw "winget validate didn't accept the manifests in $wingetFolder" }
} else {
    Write-Warning 'winget isn''t installed here: the manifests weren''t validated (winget validate --manifest <folder>).'
}

if ($CopyTo) {
    Write-Step "Copying the bundle, the App Installer file and the winget manifests to $CopyTo"
    $releaseTarget = Join-Path $CopyTo "build\windows\$Version"
    New-Item -ItemType Directory -Force $releaseTarget | Out-Null
    Copy-Item -LiteralPath $Bundle, $feedFile -Destination $releaseTarget -Force
    if ((Get-Sha256 (Join-Path $releaseTarget $BundleName)) -ne $InstallerSha256) { throw "The copy of the bundle in $releaseTarget doesn't match." }
    $wingetTarget = Join-Path $CopyTo "packaging\winget\manifests\m\$Owner\AutoPaper\$Version"
    if (Test-Path $wingetTarget) { Remove-Item $wingetTarget -Recurse -Force }
    New-Item -ItemType Directory -Force $wingetTarget | Out-Null
    Copy-Item -Path (Join-Path $wingetFolder '*') -Destination $wingetTarget -Force
}

Remove-Item $Stage -Recurse -Force

Write-Step 'Done'
Write-Host "Version          $Version (package $PackageVersion), signed with the $($Signing.Kind)"
Write-Host "Publisher        $($Signing.Subject)"
Write-Host "Package family   $FamilyName"
Write-Host ('Bundle           {0} ({1:N0} bytes)' -f $Bundle, (Get-Item $Bundle).Length)
Write-Host "  SHA-256        $InstallerSha256"
Write-Host "  Signature      $SignatureSha256 (AppxSignature.p7x)"
Write-Host "App Installer    $feedFile"
Write-Host "  feed           $AppInstallerUri"
Write-Host "  bundle         $BundleUri"
Write-Host "winget           $wingetFolder"
Write-Host "Publish: scripts/publish-windows.sh $Version on the Mac (the feed and its bundle), and attach $BundleName to"
Write-Host "the GitHub release v$Version ($ReleaseBundleUri), which the winget manifests point to."
