# Packaged smoke test (M9.5 P1): wipe-publish the module to an isolated
# smoke database, launch the exported client against it, and assert - SQL
# as primary truth, client log as corroboration - connect + own-entity
# spawn, then that a hard kill clears the session server-side (M5 cleanup).
# Deterministic: exit 0 only if every assertion passes. No interaction.
#
#   ./scripts/smoke_packaged.ps1              - smoke build/export
#   ./scripts/smoke_packaged.ps1 -Export      - run standalone export first
#   ./scripts/smoke_packaged.ps1 -ExportDir <dir> -Module <name>
#
# Owns only the smoke module and build/smoke/. Never stops a running
# SpacetimeDB; starts one (own window) only when :3000 is quiet.
param(
    [string]$ExportDir = "build/export",
    [string]$Module = "rust-engine-smoke",
    [switch]$Export
)

$RepoRoot = Split-Path $PSScriptRoot -Parent
$ServerUri = "http://127.0.0.1:3000"
$SmokeDir = Join-Path $RepoRoot "build/smoke"
$ClientLog = Join-Path $SmokeDir "client.log"
$ClientErr = Join-Path $SmokeDir "client.err"

if ($Export) {
    & (Join-Path $RepoRoot "scripts/export_windows.ps1") -OutputDir $ExportDir -Target standalone
    if ($LASTEXITCODE -ne 0) {
        Write-Host "ERROR: export failed" -ForegroundColor Red
        exit 1
    }
}

$Exe = Join-Path (Join-Path $RepoRoot $ExportDir) "game.exe"
if (-not (Test-Path $Exe)) {
    Write-Host "ERROR: no exported client at $Exe - run scripts/export_windows.ps1 (or pass -Export)" -ForegroundColor Red
    exit 1
}

New-Item -ItemType Directory -Force -Path $SmokeDir | Out-Null
Remove-Item $ClientLog, $ClientErr -Force -ErrorAction SilentlyContinue

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

# Readiness gate = wipe-publish succeeding; retries cover a cold start.
# Only sleep time counts toward the budget (host_local convention).
$Publish = Join-Path $RepoRoot "server/publish.ps1"
$waited = 0
while ($true) {
    & $Publish -Module $Module -Wipe
    if ($LASTEXITCODE -eq 0) { break }
    if ($waited -ge 30) {
        Write-Host "ERROR: publish kept failing for ${waited}s of retries - is SpacetimeDB healthy?" -ForegroundColor Red
        exit 1
    }
    Write-Host "publish failed; retrying in 2 s..." -ForegroundColor Yellow
    Start-Sleep -Seconds 2
    $waited += 2
}

# Data rows of `select entity_id, session from player` (skips header/rule).
function Get-PlayerRows {
    $out = spacetime sql $Module "select entity_id, session from player" 2>$null | Out-String
    @($out -split "`r?`n" | Where-Object { $_ -match '^\s*\d+\s*\|' })
}

function Read-ClientLog {
    try {
        $c = Get-Content $ClientLog -Raw -ErrorAction Stop
        if ($null -eq $c) { "" } else { $c }
    } catch { "" }
}

Write-Host "Launching client -> $ServerUri / $Module" -ForegroundColor Green
$proc = Start-Process -FilePath $Exe -WorkingDirectory (Split-Path $Exe) `
    -ArgumentList @("--connect", $ServerUri, $Module) `
    -RedirectStandardOutput $ClientLog -RedirectStandardError $ClientErr -PassThru

$failures = @()

# --- Assertion 1: connect + spawn (server truth), 30 s budget -------------
$PollTrace = Join-Path $SmokeDir "poll_trace.txt"
$spawned = $false
$waited = 0
while ($waited -lt 30) {
    if ($proc.HasExited) { break }
    # @() at the call site: function return unrolls a 1-element array to a
    # scalar string, and indexing a string yields a char.
    $rows = @(Get-PlayerRows)
    Add-Content $PollTrace "$(Get-Date -Format HH:mm:ss.fff) rows=$($rows.Count) [$($rows -join ' / ')]"
    if ($rows.Count -eq 1 -and $rows[0] -match '\(some') { $spawned = $true; break }
    Start-Sleep -Seconds 1
    $waited += 1
}
if ($spawned) {
    Write-Host "PASS: player row with live session (${waited}s)" -ForegroundColor Green
} elseif ($proc.HasExited) {
    $failures += "client exited early (code $($proc.ExitCode)) - see $ClientLog / $ClientErr"
} else {
    $failures += "no live player session within 30s"
}

# --- Assertion 2: client log markers (corroboration), 10 s budget ---------
if ($spawned) {
    $markers = @("net: in world as", "net: local player bound to entity")
    $waited = 0
    while ($waited -lt 10) {
        $log = Read-ClientLog
        $missing = @($markers | Where-Object { -not $log.Contains($_) })
        if ($missing.Count -eq 0) { break }
        Start-Sleep -Seconds 1
        $waited += 1
    }
    if ($missing.Count -eq 0) {
        Write-Host "PASS: client log markers present" -ForegroundColor Green
    } else {
        $failures += "client log missing marker(s): $($missing -join ', ')"
    }
    # Only a hard failure with -Export: then exporter and publisher ran from
    # the same tree and the stamps must agree. A pre-existing export may
    # legitimately be older than the just-published module.
    if ((Read-ClientLog).Contains("WARNING: build mismatch")) {
        if ($Export) {
            $failures += "build mismatch despite same-tree export+publish - stamping pipeline regressed"
        } else {
            Write-Host "WARN: build mismatch (export predates this publish; rerun with -Export for the strict gate)" -ForegroundColor Yellow
        }
    } else {
        Write-Host "PASS: no build mismatch warning" -ForegroundColor Green
    }
}

# --- Assertion 3: crash-disconnect clears session, 15 s budget ------------
if (-not $proc.HasExited) {
    Stop-Process -Id $proc.Id -Force
    Write-Host "Client killed (socket drop) - waiting for server-side cleanup..."
    $cleared = $false
    $waited = 0
    while ($waited -lt 15) {
        $rows = @(Get-PlayerRows)
        if ($rows.Count -ge 1 -and -not ($rows -match '\(some')) { $cleared = $true; break }
        Start-Sleep -Seconds 1
        $waited += 1
    }
    if ($cleared) {
        Write-Host "PASS: session cleared after kill (${waited}s)" -ForegroundColor Green
    } else {
        $failures += "session not cleared within 15s of client kill"
    }
}

# --- Report ---------------------------------------------------------------
# Post-mortem snapshot next to the client log.
spacetime sql $Module "select entity_id, session from player" 2>$null |
    Out-File (Join-Path $SmokeDir "player_rows.txt")

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "SMOKE PASS" -ForegroundColor Green
    exit 0
}
foreach ($f in $failures) { Write-Host "FAIL: $f" -ForegroundColor Red }
Write-Host "SMOKE FAIL ($($failures.Count) assertion(s)) - artifacts in $SmokeDir" -ForegroundColor Red
exit 1
