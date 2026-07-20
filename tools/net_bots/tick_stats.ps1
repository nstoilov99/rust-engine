# Prints move_tick duration percentiles from recent module logs.
# Usage: ./tick_stats.ps1 [-Lines 20000] [-Module rust-engine-dev]
param(
    [int]$Lines = 20000,
    [string]$Module = "rust-engine-dev"
)
$v = spacetime logs $Module -n $Lines 2>$null |
    Select-String 'Timing span "move_tick"' | ForEach-Object {
        $t = ($_ -split ' ')[-1]
        if ($t -match 'µs') { [double]($t -replace 'µs', '') / 1000 }
        else { [double]($t -replace 'ms', '') }
    } | Sort-Object
if (-not $v) { Write-Host "no move_tick lines found"; exit 1 }
$p50 = $v[[int]($v.Count * 0.5)]; $p95 = $v[[int]($v.Count * 0.95)]
"n=$($v.Count) p50=$([math]::Round($p50,1))ms p95=$([math]::Round($p95,1))ms max=$([math]::Round($v[-1],1))ms"
