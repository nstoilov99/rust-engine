# Publish the game module to the local SpacetimeDB standalone instance.
# Prereq: `spacetime start` running in another terminal.
#   ./publish.ps1        — incremental publish (keeps data)
#   ./publish.ps1 -Wipe  — publish and clear all dev data
param([switch]$Wipe)

$modArgs = @("publish", "--module-path", "$PSScriptRoot/game_module", "rust-engine-dev")
if ($Wipe) { $modArgs += @("--delete-data=always", "--yes") }
& spacetime @modArgs
