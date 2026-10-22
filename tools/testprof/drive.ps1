# drive.ps1 — run measure.ps1 over every test binary in the workspace
#
# Produces one CSV per test binary under -OutDir, named <crate>__<target>.csv,
# plus a binary_summary.csv with the serial and the default-thread totals.
#
# Usage:
#   .\drive.ps1                       # serial + parallel, everything
#   .\drive.ps1 -SkipParallel         # serial only (half the time)
#   .\drive.ps1 -Only girsync__why    # one binary

param(
  [string]$OutDir = "$PSScriptRoot\..\..\target\testprof",
  [switch]$SkipParallel,
  [string[]]$Only = @()
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$measure = Join-Path $here 'measure.ps1'

# cargo lists every artifact it built; the ones under target\debug\deps are the
# test harnesses (the plain target\debug\*.exe are the programs themselves).
$json = & cargo test --workspace --no-run --message-format=json 2>$null |
  ForEach-Object { $_ | ConvertFrom-Json } |
  Where-Object { $_.reason -eq 'compiler-artifact' -and $_.executable -and ($_.executable -match '\\deps\\') } |
  ForEach-Object {
    $mp = $_.manifest_path -replace '\\', '/'
    $pkg = if ($mp -match '/crates/log/') { 'girpr_log' }
           elseif ($mp -match '/tools/capture/') { 'gircap' }
           elseif ($mp -match '/tools/sync/') { 'girsync' }
           else { 'girpr' }
    $kind = ($_.target.kind -join '+')
    $tgt = if ($kind -eq 'test') { $_.target.name } else { "$($_.target.name)($kind)" }
    [pscustomobject]@{ pkg = $pkg; name = $tgt; exe = $_.executable }
  }

$bins = $json | Sort-Object pkg, name
if ($Only.Count -gt 0) { $bins = $bins | Where-Object { $Only -contains "$($_.pkg)__$($_.name)" } }

$summary = @()
foreach ($b in $bins) {
  $stem = "$($b.pkg)__$($b.name)"
  Write-Host "=== serial: $stem"
  $s = & $measure -Exe $b.exe -Threads 1 -Csv (Join-Path $OutDir "$stem.csv") -Quiet | ConvertFrom-Json
  $summary += [pscustomobject]@{
    pkg = $b.pkg; name = $b.name; mode = 'serial'
    tests = $s.tests; total_ms = $s.total_ms; test_ms = $s.test_ms; cpu_ms = $s.cpu_ms; exit = $s.exit
  }
  if (-not $SkipParallel) {
    Write-Host "=== parallel: $stem"
    $s2 = & $measure -Exe $b.exe -Threads 0 -Quiet | ConvertFrom-Json
    $summary += [pscustomobject]@{
      pkg = $b.pkg; name = $b.name; mode = 'parallel'
      tests = $s2.tests; total_ms = $s2.total_ms; test_ms = $s2.test_ms; cpu_ms = $s2.cpu_ms; exit = $s2.exit
    }
  }
}

$summary | Export-Csv -Path (Join-Path $OutDir 'binary_summary.csv') -NoTypeInformation -Encoding UTF8
$summary | Sort-Object mode, total_ms -Descending | Format-Table -AutoSize
