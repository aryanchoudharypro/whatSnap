<#
    Builds whatSnap for release: both binaries, a portable zip, and the installer.

    Usage:
        packaging\build-release.ps1
        packaging\build-release.ps1 -Version 0.2.0
        packaging\build-release.ps1 -SkipInstaller     # zip only, no Inno Setup

    Signing (optional, but strongly recommended for public releases -- an
    unsigned binary hits the SmartScreen wall on every download):
        packaging\build-release.ps1 -SignCommand 'signtool sign /fd sha256 /tr http://timestamp.certum.pl /td sha256 /a "{0}"'
    The {0} placeholder is replaced with each file path in turn.
#>
[CmdletBinding()]
param(
    [string]$Version,
    [switch]$SkipInstaller,
    [string]$SignCommand
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$releaseDir = Join-Path $repo 'target\release'
$distDir = Join-Path $repo 'dist'

if (-not $Version) {
    $cargoToml = Get-Content (Join-Path $repo 'Cargo.toml') -Raw
    if ($cargoToml -match '(?m)^version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        throw "Could not read version from Cargo.toml; pass -Version explicitly."
    }
}
Write-Host "Building whatSnap $Version" -ForegroundColor Cyan

foreach ($tool in 'cargo', 'go') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool not found on PATH. Both a Rust and a Go toolchain are required."
    }
}

# build.rs drives `go build` for the backend, so this produces both binaries.
Push-Location $repo
$env:CMAKE_POLICY_VERSION_MINIMUM="3.5"
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    # The Windows Share menu helper (share-target/).
    cargo build --release -p whatsnap-share
    if ($LASTEXITCODE -ne 0) { throw "cargo build of the share helper failed" }
} finally {
    Pop-Location
}

$frontend = Join-Path $releaseDir 'whatSnap.exe'
$backend = Join-Path $releaseDir 'backend\backend.exe'
$shareHelper = Join-Path $releaseDir 'whatSnapShare.exe'

# The Share menu's sparse package: just the manifest and logos, left unsigned.
# Each PC that turns the menu on signs it with a certificate of its own (see
# share-target/src/sign.rs), so no signing key is needed here.
$makeappx = Get-Command makeappx -ErrorAction SilentlyContinue
if ($makeappx) {
    $makeappx = $makeappx.Source
} else {
    $makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter makeappx.exe -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.DirectoryName -like '*\x64' } | Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $makeappx) { throw "makeappx.exe not found. It comes with the Windows SDK (Visual Studio's C++ tools install it)." }
$sharePackage = Join-Path $releaseDir 'share\whatSnapShare.msix'
New-Item -ItemType Directory -Force -Path (Split-Path $sharePackage) | Out-Null
& $makeappx pack /o /nv /d (Join-Path $PSScriptRoot 'share') /p $sharePackage
if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

foreach ($f in $frontend, $backend, $shareHelper, $sharePackage) {
    if (-not (Test-Path $f)) { throw "Expected build output missing: $f" }
}
Write-Host "  frontend: $([math]::Round((Get-Item $frontend).Length / 1MB, 1)) MB"
Write-Host "  backend:  $([math]::Round((Get-Item $backend).Length / 1MB, 1)) MB"

if ($SignCommand) {
    Write-Host "Signing binaries..." -ForegroundColor Cyan
    foreach ($f in $frontend, $backend, $shareHelper) {
        $cmd = $SignCommand -f $f
        Invoke-Expression $cmd
        if ($LASTEXITCODE -ne 0) { throw "Signing failed for $f" }
    }
}

New-Item -ItemType Directory -Force -Path $distDir | Out-Null

# Portable layout. Mirrors the installed layout exactly: the app resolves
# backend.exe relative to its own directory, so a copied folder works with no
# install step. Notification sounds are embedded in the exe.
Write-Host "Staging portable zip..." -ForegroundColor Cyan
$stage = Join-Path $distDir "whatSnap-$Version-portable"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage, "$stage\backend", "$stage\share" | Out-Null

Copy-Item $frontend $stage
Copy-Item $shareHelper $stage
Copy-Item $sharePackage "$stage\share"
Copy-Item $backend "$stage\backend"
Copy-Item (Join-Path $repo 'LICENSE') $stage
Copy-Item (Join-Path $repo 'backend\LICENSE') "$stage\backend"
Copy-Item (Join-Path $repo 'THIRD-PARTY-NOTICES.md') $stage

$zip = Join-Path $distDir "whatSnap-$Version-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item $stage -Recurse -Force
Write-Host "  $zip"

if (-not $SkipInstaller) {
    $iscc = Get-Command iscc -ErrorAction SilentlyContinue
    if ($iscc) {
        $iscc = $iscc.Source
    } else {
        # Inno Setup installs per-machine or per-user; check both.
        $candidates = @(
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
            "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
        )
        $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    }

    if (-not $iscc) {
        Write-Warning "Inno Setup (ISCC.exe) not found - skipping installer. Install from https://jrsoftware.org/isdl.php"
    } else {
        Write-Host "Building installer..." -ForegroundColor Cyan
        & $iscc "/DAppVersion=$Version" (Join-Path $PSScriptRoot 'whatSnap.iss')
        if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

        $setup = Join-Path $distDir "whatSnap-$Version-setup.exe"
        if ($SignCommand -and (Test-Path $setup)) {
            Invoke-Expression ($SignCommand -f $setup)
            if ($LASTEXITCODE -ne 0) { throw "Signing failed for $setup" }
        }
        Write-Host "  $setup"
    }
}

Write-Host "`nDone. Artifacts in $distDir" -ForegroundColor Green
