# Host-local loop (M9 D5): start (or reuse) a local SpacetimeDB, publish the
# module, and launch the exported client against it.
#
#   ./scripts/host_local.ps1                    — publish + launch build/export
#   ./scripts/host_local.ps1 -Wipe              — wipe dev data on publish
#   ./scripts/host_local.ps1 -ExportDir <dir> -Module <name>
#
# This script never stops a running SpacetimeDB instance — it may hold state
# from other work. It only starts one (own window) when :3000 is quiet.
param(
    [string]$ExportDir = "build/export",
    [string]$Module = "rust-engine-dev",
    [switch]$Wipe
)

$RepoRoot = Split-Path $PSScriptRoot -Parent
$ServerUri = "http://127.0.0.1:3000"

$Exe = Join-Path (Join-Path $RepoRoot $ExportDir) "game.exe"
if (-not (Test-Path $Exe)) {
    Write-Host "ERROR: no exported client at $Exe — run scripts/export_windows.ps1 first" -ForegroundColor Red
    exit 1
}

function Test-Server {
    try {
        (Invoke-WebRequest -Uri "$ServerUri/v1/ping" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200
    } catch { $false }
}

if (Test-Server) {
    Write-Host "SpacetimeDB already running at $ServerUri (reusing)"
} else {
    Write-Host "Starting SpacetimeDB in its own window..." -ForegroundColor Yellow
    Start-Process spacetime -ArgumentList "start"
}

# Readiness gate = a publish succeeding; retries cover a cold `spacetime
# start`. Only sleep time counts toward the budget, so a slow module build
# inside publish.ps1 can't eat the retries.
$Publish = Join-Path $RepoRoot "server/publish.ps1"
$waited = 0
while ($true) {
    $pubArgs = @{ Module = $Module }
    if ($Wipe) { $pubArgs.Wipe = $true }
    & $Publish @pubArgs
    if ($LASTEXITCODE -eq 0) { break }
    if ($waited -ge 30) {
        Write-Host "ERROR: publish kept failing for ${waited}s of retries — is SpacetimeDB healthy?" -ForegroundColor Red
        exit 1
    }
    Write-Host "publish failed; retrying in 2 s..." -ForegroundColor Yellow
    Start-Sleep -Seconds 2
    $waited += 2
}

# Explicit --connect positionals defeat any stale net_config.ron in the
# export dir (M9 D1: CLI wins over config).
Write-Host "Launching client -> $ServerUri / $Module" -ForegroundColor Green
Start-Process -FilePath $Exe -WorkingDirectory (Split-Path $Exe) `
    -ArgumentList @("--connect", $ServerUri, $Module)
