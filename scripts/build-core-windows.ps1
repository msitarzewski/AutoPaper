<#
.SYNOPSIS
Builds the AutoPaper core for Windows (autopaper_core.dll) and generates its C# bindings.

.DESCRIPTION
Run it INSIDE Windows (the "Windows 11" VM), from a local copy of the repository: Windows can't register
packages from the Parallels share, and cargo is slow there (memory-bank/techContext.md):

    robocopy \\Mac\Home\Clean\autopaper C:\Users\michael\dev\autopaper /MIR /XD .git bin obj target AppX .scratch /XF .env .env.*
    powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\michael\dev\autopaper\scripts\build-core-windows.ps1

It writes (git-ignored; replaced on every run):

    apps\windows\Generated\autopaper_core.cs             UniFFI's C# bindings: namespace AutoPaper.Core, public
    apps\windows\Generated\win-arm64\autopaper_core.dll
    apps\windows\Generated\win-x64\autopaper_core.dll    when x64 could be built

The app compiles the .cs (with <AllowUnsafeBlocks>true</AllowUnsafeBlocks>) and copies the DLL for its
RuntimeIdentifier next to AutoPaper.exe (docs/research/windows.md, "Where the DLL goes"). Binding settings:
core\uniffi.toml.

Needs:
- rustup, stable toolchain, targets aarch64-pc-windows-msvc (and x86_64-pc-windows-msvc for x64).
- Visual Studio Build Tools 2022, C++ workload with the ARM64 and x64 toolsets and a Windows 11 SDK.
- ARM64: clang-cl, which aws-lc-sys (TLS) uses for its assembly: the "C++ Clang Compiler for Windows"
  component (Microsoft.VisualStudio.Component.VC.Llvm.Clang) or clang-cl on PATH.
- x64: NASM on PATH, or else aws-lc-sys's prebuilt NASM objects (allowed here when nasm isn't found).
- uniffi-bindgen-cs v0.11.0+v0.31.0, the generator that matches the core's UniFFI 0.31:
  cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.11.0+v0.31.0

.PARAMETER Arch
arm64, x64 or all (the default). With all, ARM64 must build; x64 is built when the toolchain allows and
otherwise reported as skipped. The bindings come from the first DLL built (they're the same for both).

.PARAMETER TargetDir
Cargo's target directory, as an absolute path. Default: $env:CARGO_TARGET_DIR, else <repo>\target. Must be
on a local disk.
#>
[CmdletBinding()]
param(
    [ValidateSet('arm64', 'x64', 'all')]
    [string]$Arch = 'all',
    [string]$TargetDir = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).ProviderPath
$Generated = Join-Path $Repo 'apps\windows\Generated'
$Config = Join-Path $Repo 'core\uniffi.toml'
$BindgenVersion = '0.11.0+v0.31.0'
$Triples = @{ 'arm64' = 'aarch64-pc-windows-msvc'; 'x64' = 'x86_64-pc-windows-msvc' }

function Write-Step([string]$Message) {
    Write-Host ''
    Write-Host "==> $Message"
}

# Native commands run without 2>&1 (Windows PowerShell 5.1 turns redirected stderr into terminating errors
# under ErrorActionPreference Stop); their output goes to the console, not into a function's return value, and
# a non-zero exit code is an error.
function Invoke-Native([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "$Exe $($Arguments -join ' ') failed (exit code $LASTEXITCODE)"
    }
}

function Test-NetworkPath([string]$Path) {
    if ($Path.StartsWith('\\')) { return $true }
    $drive = New-Object System.IO.DriveInfo ([System.IO.Path]::GetPathRoot($Path))
    return $drive.DriveType -eq [System.IO.DriveType]::Network
}

# Where aws-lc-sys looks for clang-cl: PATH, then <Visual Studio>\VC\Tools\Llvm\{ARM64,x64}\bin.
function Find-ClangCl {
    $onPath = Get-Command clang-cl -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $null }
    foreach ($install in @(& $vswhere -products * -property installationPath)) {
        foreach ($toolsArch in 'ARM64', 'x64') {
            $candidate = Join-Path $install "VC\Tools\Llvm\$toolsArch\bin\clang-cl.exe"
            if (Test-Path $candidate) { return $candidate }
        }
    }
    return $null
}

# Builds autopaper_core.dll for one architecture; returns its path.
function Build-Core([string]$Name) {
    $triple = $Triples[$Name]
    if ($InstalledTargets -notcontains $triple) {
        throw "the $triple target isn't installed: rustup target add --toolchain stable $triple"
    }
    if ($Name -eq 'arm64' -and -not (Find-ClangCl)) {
        throw ('ARM64 needs clang-cl for aws-lc-sys. Add the "C++ Clang Compiler for Windows" component, e.g. ' +
            'as an administrator: & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vs_installer.exe" ' +
            'modify --installPath C:\BuildTools --add Microsoft.VisualStudio.Component.VC.Llvm.Clang --quiet --norestart')
    }
    if ($Name -eq 'x64' -and -not (Get-Command nasm -ErrorAction SilentlyContinue)) {
        Write-Host 'nasm not found: aws-lc-sys will use its prebuilt NASM objects for x86_64.'
        $env:AWS_LC_SYS_PREBUILT_NASM = '1'
    }
    Write-Step "Building the core for $triple (release)"
    Invoke-Native 'rustup' @('run', 'stable', 'cargo', 'rustc', '-p', 'autopaper-core', '--lib', '--release',
        '--target', $triple, '--crate-type', 'cdylib')
    $dll = Join-Path $TargetDir "$triple\release\autopaper_core.dll"
    if (-not (Test-Path $dll)) { throw "cargo finished but $dll isn't there" }
    return $dll
}

# ── Checks ──────────────────────────────────────────────────────────────────────────────────────

if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) { throw 'rustup isn''t installed (https://rustup.rs).' }
$Bindgen = Get-Command uniffi-bindgen-cs -ErrorAction SilentlyContinue
if (-not $Bindgen) {
    throw ("uniffi-bindgen-cs isn't installed: cargo install uniffi-bindgen-cs --git " +
        "https://github.com/NordSecurity/uniffi-bindgen-cs --tag v$BindgenVersion")
}
$listed = @(& rustup run stable cargo install --list) | Where-Object { $_ -match '^uniffi-bindgen-cs v' }
if ($listed -and -not ($listed -match [regex]::Escape("v$BindgenVersion"))) {
    throw "uniffi-bindgen-cs v$BindgenVersion is needed (the core uses UniFFI 0.31); installed: $listed"
}
if (-not $listed) { Write-Warning "uniffi-bindgen-cs wasn't installed by cargo; make sure it is v$BindgenVersion." }

if (-not $TargetDir) {
    $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Repo 'target' }
}
$TargetDir = [System.IO.Path]::GetFullPath($TargetDir)
if (Test-NetworkPath $TargetDir) {
    throw ("Cargo's target directory $TargetDir is on a network share. Copy the repository to a local disk " +
        '(see the help: Get-Help .\scripts\build-core-windows.ps1) or pass -TargetDir C:\...')
}
if (Test-NetworkPath $Repo) {
    Write-Warning "The repository is on a network share ($Repo): the build works, but the app can't be registered from there."
}
$env:CARGO_TARGET_DIR = $TargetDir
$InstalledTargets = @(& rustup target list --installed --toolchain stable)
if ($LASTEXITCODE -ne 0) { throw 'rustup target list failed' }

# ── Build ───────────────────────────────────────────────────────────────────────────────────────

$Built = [ordered]@{}
$Skipped = $null
Push-Location $Repo
try {
    switch ($Arch) {
        'arm64' { $Built['arm64'] = Build-Core 'arm64' }
        'x64' { $Built['x64'] = Build-Core 'x64' }
        'all' {
            $Built['arm64'] = Build-Core 'arm64'
            try {
                $Built['x64'] = Build-Core 'x64'
            } catch {
                $Skipped = $_.Exception.Message
                Write-Warning "x64 wasn't built: $Skipped"
            }
        }
    }

    Write-Step 'Generating the C# bindings'
    $staging = Join-Path $TargetDir 'autopaper-cs'
    if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
    New-Item -ItemType Directory $staging | Out-Null
    $source = @($Built.Values)[0]
    # Library mode reads the crate's metadata from the DLL; run from the repository root (cargo metadata).
    Invoke-Native $Bindgen.Source @('--library', $source, '--out-dir', $staging, '--config', $Config, '--no-format')
    if (-not (Get-ChildItem $staging -Filter *.cs)) { throw "uniffi-bindgen-cs wrote no C# file to $staging" }

    Write-Step "Installing into $Generated"
    if (Test-Path $Generated) { Remove-Item $Generated -Recurse -Force }
    New-Item -ItemType Directory $Generated | Out-Null
    Copy-Item (Join-Path $staging '*.cs') $Generated
    foreach ($name in $Built.Keys) {
        $folder = Join-Path $Generated "win-$name"
        New-Item -ItemType Directory $folder | Out-Null
        Copy-Item $Built[$name] $folder
    }
    Remove-Item $staging -Recurse -Force
} finally {
    Pop-Location
}

Write-Step 'Done'
Get-ChildItem $Generated -Recurse -File | ForEach-Object {
    '{0,12:N0}  {1}' -f $_.Length, $_.FullName.Substring($Generated.Length + 1)
}
if ($Skipped) { Write-Warning "x64 wasn't built: $Skipped" }
