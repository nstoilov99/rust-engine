# Publish the game module to a SpacetimeDB instance.
# Prereq (local default): `spacetime start` running in another terminal.
#   ./publish.ps1                          - incremental publish to local, keeps data
#   ./publish.ps1 -Wipe                    - publish and clear all dev data
#   ./publish.ps1 -Server <uri> -Module <name>  - remote/named target (M9 D3)
param(
    [switch]$Wipe,
    [string]$Server = "",
    [string]$Module = "rust-engine-dev"
)

if ($Module -notmatch '^[a-z0-9]+(-[a-z0-9]+)*$') {
    Write-Error "invalid module name '$Module' (must match ^[a-z0-9]+(-[a-z0-9]+)*$)"
    exit 1
}

$modArgs = @("publish", "--module-path", "$PSScriptRoot/game_module", $Module)
if ($Server) { $modArgs += @("--server", $Server) }
if ($Wipe) { $modArgs += "--delete-data=always" }
# --yes: wipe skips the data-destruction prompt; non-local targets skip the
# "publish to a non-local server?" prompt (M9.5: scripted Maincloud publish).
if ($Wipe -or $Server) { $modArgs += "--yes" }
& spacetime @modArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "spacetime publish failed (exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}

# M9 D4: `init` only runs on the first publish - stamp the build id every
# time so Config tracks the deployed WASM. Soft stamp: failure is a warning.
$callArgs = @("call")
if ($Server) { $callArgs += @("--server", $Server) }
$callArgs += @($Module, "set_build_id")
& spacetime @callArgs
if ($LASTEXITCODE -ne 0) {
    Write-Warning "set_build_id call failed (exit $LASTEXITCODE) - Config.build_id may be stale"
}

# M6 D1: the module embeds ~4.3 MB of collision data; keep an eye on size.
$wasm = Get-ChildItem "$PSScriptRoot/game_module/target/wasm32-unknown-unknown/release/*.wasm" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime | Select-Object -Last 1
if ($wasm) {
    "module size: {0:N0} bytes ({1})" -f $wasm.Length, $wasm.Name
}

# Explicit success: a warn-only set_build_id failure must not leak its exit
# code to callers gating on $LASTEXITCODE (scripts/host_local.ps1).
exit 0
