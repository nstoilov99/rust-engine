# Publish the game module to the local SpacetimeDB standalone instance.
# Prereq: `spacetime start` running in another terminal.
#   ./publish.ps1        — incremental publish (keeps data)
#   ./publish.ps1 -Wipe  — publish and clear all dev data
param([switch]$Wipe)

$modArgs = @("publish", "--module-path", "$PSScriptRoot/game_module", "rust-engine-dev")
if ($Wipe) { $modArgs += @("--delete-data=always", "--yes") }
& spacetime @modArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "spacetime publish failed (exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}

# M6 D1: the module embeds ~4.3 MB of collision data; keep an eye on size.
$wasm = Get-ChildItem "$PSScriptRoot/game_module/target/wasm32-unknown-unknown/release/*.wasm" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime | Select-Object -Last 1
if ($wasm) {
    "module size: {0:N0} bytes ({1})" -f $wasm.Length, $wasm.Name
}
