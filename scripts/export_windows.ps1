# Export script for Windows (PowerShell)
# Builds the standalone game and copies all required files to an output directory.
#
# Usage: .\scripts\export_windows.ps1 [-OutputDir <path>] [-Profile <release|shipping>]

param(
    [string]$OutputDir = "build\export",
    [ValidateSet("release", "shipping")]
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"

$BinName = "game"

Write-Host "=== Rust Game Engine - Windows Export ===" -ForegroundColor Cyan
Write-Host "Profile : $Profile"
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

Write-Host ""
Write-Host "=== Export complete: $OutputDir ===" -ForegroundColor Cyan
