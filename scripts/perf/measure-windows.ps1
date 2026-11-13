# Measure a ZapFast process tree using Windows process counters.
# This reads process metadata only; it never opens chat databases or logs.
param(
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 2147483647)]
    [int] $ProcessId,
    [ValidateRange(1, 100)]
    [int] $Samples = 5,
    [ValidateRange(1, 3600)]
    [int] $IntervalSeconds = 10
)

$ErrorActionPreference = "Stop"

function Get-ProcessTreeIds([int] $Root) {
    $all = @(Get-CimInstance Win32_Process)
    $children = @{}
    foreach ($item in $all) {
        $parent = [int] $item.ParentProcessId
        if (-not $children.ContainsKey($parent)) {
            $children[$parent] = [System.Collections.Generic.List[int]]::new()
        }
        $children[$parent].Add([int] $item.ProcessId)
    }

    $queue = [System.Collections.Generic.Queue[int]]::new()
    $seen = [System.Collections.Generic.HashSet[int]]::new()
    $queue.Enqueue($Root)
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if (-not $seen.Add($current)) { continue }
        if ($children.ContainsKey($current)) {
            foreach ($child in $children[$current]) { $queue.Enqueue($child) }
        }
    }
    return @($seen)
}

function Get-Snapshot([int] $Root) {
    [int64] $private = 0
    [int64] $working = 0
    [double] $cpu = 0
    $count = 0
    foreach ($id in (Get-ProcessTreeIds $Root)) {
        try {
            $p = Get-Process -Id $id -ErrorAction Stop
            $private += [int64] $p.PrivateMemorySize64
            $working += [int64] $p.WorkingSet64
            if ($null -ne $p.CPU) { $cpu += [double] $p.CPU }
            $count++
        } catch {
            # A child may exit between the process-tree and counter snapshots.
        }
    }
    return [PSCustomObject]@{
        Count = $count
        PrivateMiB = $private / 1MB
        WorkingSetMiB = $working / 1MB
        CpuSeconds = $cpu
    }
}

$null = Get-Process -Id $ProcessId -ErrorAction Stop
$cores = [Environment]::ProcessorCount
Write-Output "ZapFast process-tree benchmark; root PID $ProcessId"
Write-Output "Samples: $Samples; interval: $IntervalSeconds seconds"
Write-Output ("{0,6} {1,6} {2,14} {3,17} {4,12} {5,14}" -f "Sample", "PIDs", "Private MiB", "Working Set MiB", "CPU %/core", "CPU %/system")

for ($i = 1; $i -le $Samples; $i++) {
    $before = Get-Snapshot $ProcessId
    Start-Sleep -Seconds $IntervalSeconds
    $after = Get-Snapshot $ProcessId
    $delta = [Math]::Max(0.0, $after.CpuSeconds - $before.CpuSeconds)
    $oneCore = ($delta / $IntervalSeconds) * 100.0
    $system = $oneCore / $cores
    Write-Output ("{0,6} {1,6} {2,14:N2} {3,17:N2} {4,12:N3} {5,14:N3}" -f $i, $after.Count, $after.PrivateMiB, $after.WorkingSetMiB, $oneCore, $system)
}
Write-Output "CPU is an interval average; one-core usage can exceed 100%."
Write-Output "Private MiB is private committed memory; working set includes shared pages."
