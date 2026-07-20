# M8 D5 scenario runner: splits -Total bots into processes of -PerProcess
# (SDK panics on dead sockets kill a whole process, so blast radius stays
# bounded) and prints every process report when done.
#
# Examples:
#   ./run_scenarios.ps1 -Scenario uniform -Total 300 -Duration 120
#   ./run_scenarios.ps1 -Scenario hotspot -Total 150 -Duration 120 -Disperse 60
#   ./run_scenarios.ps1 -Scenario churn   -Total 100 -Duration 120
#   ./run_scenarios.ps1 -Scenario thrash  -Total 50  -Duration 120
param(
    [Parameter(Mandatory)][string]$Scenario,
    [int]$Total = 300,
    [int]$PerProcess = 50,
    [int]$Duration = 120,
    [string]$Center = "32,32",
    [double]$Disperse = 0,
    [double]$Area = 200,
    # M9.5 P2: target a non-default instance/module (e.g. the smoke or a
    # packaged-published database). Defaults match the exe's own defaults.
    [string]$TargetHost = "http://127.0.0.1:3000",
    [string]$Module = "rust-engine-dev"
)
$exe = Join-Path $PSScriptRoot "..\..\target\release\net_bots.exe"
$outDir = Join-Path $PSScriptRoot "out"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$procs = [int][math]::Ceiling($Total / $PerProcess)
$jobs = @()
for ($i = 0; $i -lt $procs; $i++) {
    $n = [math]::Min($PerProcess, $Total - $i * $PerProcess)
    $argList = @(
        "--bots", $n, "--prefix", "ld-$Scenario-p$i", "--scenario", $Scenario,
        "--duration", $Duration, "--center", $Center, "--area", $Area,
        "--host", $TargetHost, "--module", $Module
    )
    if ($Disperse -gt 0) { $argList += @("--disperse", $Disperse) }
    $jobs += Start-Process -FilePath $exe -ArgumentList $argList `
        -RedirectStandardOutput (Join-Path $outDir "$Scenario-p$i.txt") `
        -RedirectStandardError (Join-Path $outDir "$Scenario-p$i.err.txt") `
        -PassThru
}
Write-Host "spawned $procs processes ($Total bots), waiting ${Duration}s..."
$jobs | Wait-Process
for ($i = 0; $i -lt $procs; $i++) {
    $f = Join-Path $outDir "$Scenario-p$i.txt"
    Write-Host "== $Scenario-p$i.txt"; Get-Content $f
}
