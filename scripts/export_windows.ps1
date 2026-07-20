# Export script for Windows (PowerShell)
# Builds the standalone game and copies all required files to an output directory.
#
# Usage: .\scripts\export_windows.ps1 [-OutputDir <path>] [-Profile <release|shipping>]
#                                     [-Target <standalone|mp-client>]
#                                     [-ServerUri <uri>] [-Module <name>]
#
# Targets (M9 D2): same binary either way — the target is configuration.
#   standalone : no net config in the bundle (and deletes a stale one)
#   mp-client  : writes net_config.ron (auto_connect) next to the exe

param(
    [string]$OutputDir = "build\export",
    [ValidateSet("release", "shipping")]
    [string]$Profile = "release",
    [ValidateSet("standalone", "mp-client")]
    [string]$Target = "standalone",
    [string]$ServerUri = "http://127.0.0.1:3000",
    [string]$Module = "rust-engine-dev"
)

$ErrorActionPreference = "Stop"

$BinName = "game"

if ($Target -eq "mp-client") {
    if ($Module -notmatch '^[a-z0-9]+(-[a-z0-9]+)*$') {
        Write-Host "ERROR: invalid module name '$Module' (must match ^[a-z0-9]+(-[a-z0-9]+)*$)" -ForegroundColor Red
        exit 1
    }
    $parsed = $null
    if (-not [System.Uri]::TryCreate($ServerUri, [System.UriKind]::Absolute, [ref]$parsed)) {
        Write-Host "ERROR: invalid server URI '$ServerUri'" -ForegroundColor Red
        exit 1
    }
}

Write-Host "=== Rust Game Engine - Windows Export ===" -ForegroundColor Cyan
Write-Host "Profile : $Profile"
Write-Host "Target  : $Target"
Write-Host "Output  : $OutputDir"
Write-Host ""

# Build
Write-Host "Building ($Profile)..." -ForegroundColor Yellow
if ($Profile -eq "shipping") {
    cargo build --profile shipping
} else {
    cargo build --release
}
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build FAILED" -ForegroundColor Red
    exit 1
}
Write-Host "Build OK" -ForegroundColor Green

# Determine build output directory
if ($Profile -eq "shipping") {
    $BuildDir = "target\shipping"
} else {
    $BuildDir = "target\release"
}

# Create output directory
if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
}

# Copy executable
$ExePath = Join-Path $BuildDir "$BinName.exe"
if (Test-Path $ExePath) {
    Copy-Item $ExePath -Destination $OutputDir -Force
    $exeSize = (Get-Item $ExePath).Length / 1MB
    Write-Host ("Copied {0}.exe ({1:N1} MB)" -f $BinName, $exeSize) -ForegroundColor Green
} else {
    Write-Host "ERROR: $ExePath not found" -ForegroundColor Red
    exit 1
}

# Copy DLLs (if any)
$dlls = Get-ChildItem -Path $BuildDir -Filter "*.dll" -ErrorAction SilentlyContinue
foreach ($dll in $dlls) {
    Copy-Item $dll.FullName -Destination $OutputDir -Force
    Write-Host "Copied $($dll.Name)"
}

# Cook static collision for all scenes (skips up-to-date cooks)
$scenes = Get-ChildItem -Path "content\scenes" -Filter "*.scene" -ErrorAction SilentlyContinue
if ($scenes) {
    Write-Host "Cooking static collision..." -ForegroundColor Yellow
    foreach ($scene in $scenes) {
        cargo run --release --bin collision_cooker -- "scenes/$($scene.Name)"
        if ($LASTEXITCODE -ne 0) {
            Write-Host "ERROR: collision cook failed for $($scene.Name)" -ForegroundColor Red
            exit 1
        }
    }
}

# Pack content into game.pak
$ContentSrc = "content"
$PakDst = Join-Path $OutputDir "game.pak"
if (Test-Path $ContentSrc) {
    Write-Host "Packing content/ into game.pak..." -ForegroundColor Yellow
    cargo run --release --bin pak_tool -- pack $ContentSrc $PakDst
    if ($LASTEXITCODE -ne 0) {
        Write-Host "WARNING: pak_tool failed, falling back to raw copy" -ForegroundColor Yellow
        $ContentDst = Join-Path $OutputDir "content"
        if (Test-Path $ContentDst) { Remove-Item $ContentDst -Recurse -Force }
        Copy-Item $ContentSrc -Destination $ContentDst -Recurse -Force
        $fileCount = (Get-ChildItem $ContentDst -Recurse -File).Count
        Write-Host "Copied content/ ($fileCount files)" -ForegroundColor Green
    } else {
        $pakSize = (Get-Item $PakDst).Length / 1MB
        Write-Host ("Created game.pak ({0:N1} MB)" -f $pakSize) -ForegroundColor Green
    }
} else {
    Write-Host "WARNING: content/ directory not found" -ForegroundColor Yellow
}

# Net config (M9 D2): targets own their marker files — standalone deletes a
# stale config so re-exporting over an mp-client bundle can't auto-connect.
$NetConfigPath = Join-Path $OutputDir "net_config.ron"
if ($Target -eq "mp-client") {
    @"
NetConfig(
    host: "$ServerUri",
    module: "$Module",
    auto_connect: true,
)
"@ | Set-Content -Path $NetConfigPath -Encoding utf8
    Write-Host "Wrote net_config.ron -> $ServerUri / $Module" -ForegroundColor Green
} elseif (Test-Path $NetConfigPath) {
    Remove-Item $NetConfigPath -Force
    Write-Host "Removed stale net_config.ron (standalone target)"
}

Write-Host ""
Write-Host "=== Export complete: $OutputDir ($Target) ===" -ForegroundColor Cyan
