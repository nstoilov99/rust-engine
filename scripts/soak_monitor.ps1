# Soak monitor (M9.5 P3): samples a process's working set and handle count
# to CSV - the leak evidence for the hour co-op soak (flat-ish memory =
# pass, monotonic growth = investigate). Stops when the process exits.
#
#   ./scripts/soak_monitor.ps1                       - monitor game.exe
#   ./scripts/soak_monitor.ps1 -ProcessName game -IntervalSec 30 -OutCsv soak.csv
param(
    [string]$ProcessName = "game",
    [int]$IntervalSec = 30,
    [string]$OutCsv = "soak_monitor.csv"
)

$procs = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue)
if ($procs.Count -eq 0) {
    Write-Host "ERROR: no running process named '$ProcessName'" -ForegroundColor Red
    exit 1
}

"timestamp,pid,working_set_mb,private_mb,handles,threads" | Out-File $OutCsv -Encoding ascii
Write-Host "Monitoring $($procs.Count) '$ProcessName' process(es) every ${IntervalSec}s -> $OutCsv (Ctrl+C to stop)"

while ($true) {
    $procs = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue)
    if ($procs.Count -eq 0) {
        Write-Host "process exited - monitor done"
        break
    }
    $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    foreach ($p in $procs) {
        $ws = [math]::Round($p.WorkingSet64 / 1MB, 1)
        $priv = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        "$ts,$($p.Id),$ws,$priv,$($p.HandleCount),$($p.Threads.Count)" | Add-Content $OutCsv
    }
    Start-Sleep -Seconds $IntervalSec
}
