# aggregate.ps1 — turn the per-test CSVs into the tables the report uses
#
# Usage:
#   .\aggregate.ps1 -ProfDir <dir with the per-test CSVs>

param(
  [string]$ProfDir = "$PSScriptRoot\..\..\target\testprof",
  [string]$OutDir = "$PSScriptRoot\..\..\target\testprof\agg"
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$rows = @()
Get-ChildItem $ProfDir -Filter '*.csv' | Where-Object { $_.Name -ne 'binary_summary.csv' } | ForEach-Object {
  $stem = $_.BaseName
  $pkg, $tgt = $stem -split '__', 2
  Import-Csv $_.FullName | ForEach-Object {
    $rows += [pscustomobject]@{
      pkg = $pkg; target = $tgt; test = $_.name; verdict = $_.verdict
      wall_ms = [double]$_.wall_ms; cpu_ms = [double]$_.cpu_ms
      read_ops = [int64]$_.read_ops; read_kb = [double]$_.read_kb
      write_ops = [int64]$_.write_ops; write_kb = [double]$_.write_kb
    }
  }
}
$rows | Export-Csv (Join-Path $OutDir 'all_tests.csv') -NoTypeInformation -Encoding UTF8

"tests measured : $($rows.Count)"
"total wall (ms): $([math]::Round((($rows | Measure-Object wall_ms -Sum).Sum),0))"
"total cpu  (ms): $([math]::Round((($rows | Measure-Object cpu_ms -Sum).Sum),0))"

"`n== per binary =="
$rows | Group-Object pkg, target | ForEach-Object {
  $g = $_.Group
  [pscustomobject]@{
    binary = $_.Name; tests = $g.Count
    wall_s = [math]::Round((($g | Measure-Object wall_ms -Sum).Sum) / 1000, 2)
    cpu_s  = [math]::Round((($g | Measure-Object cpu_ms -Sum).Sum) / 1000, 2)
    wait_s = [math]::Round(((($g | Measure-Object wall_ms -Sum).Sum) - (($g | Measure-Object cpu_ms -Sum).Sum)) / 1000, 2)
    write_MB = [math]::Round((($g | Measure-Object write_kb -Sum).Sum) / 1024, 1)
  }
} | Sort-Object wall_s -Descending | Format-Table -AutoSize

"`n== per crate =="
$rows | Group-Object pkg | ForEach-Object {
  $g = $_.Group
  [pscustomobject]@{
    crate = $_.Name; tests = $g.Count
    wall_s = [math]::Round((($g | Measure-Object wall_ms -Sum).Sum) / 1000, 2)
    cpu_s  = [math]::Round((($g | Measure-Object cpu_ms -Sum).Sum) / 1000, 2)
  }
} | Sort-Object wall_s -Descending | Format-Table -AutoSize

"`n== wall-time buckets =="
$rows | ForEach-Object {
  $b = if ($_.wall_ms -ge 10000) { '>= 10 s' } elseif ($_.wall_ms -ge 3000) { '3-10 s' }
       elseif ($_.wall_ms -ge 1000) { '1-3 s' } elseif ($_.wall_ms -ge 250) { '0.25-1 s' }
       elseif ($_.wall_ms -ge 50) { '0.05-0.25 s' } else { '< 0.05 s' }
  [pscustomobject]@{ bucket = $b; wall = $_.wall_ms; cpu = $_.cpu_ms }
} | Group-Object bucket | ForEach-Object {
  [pscustomobject]@{
    bucket = $_.Name; tests = $_.Count
    wall_s = [math]::Round((($_.Group | Measure-Object wall -Sum).Sum) / 1000, 2)
    cpu_s  = [math]::Round((($_.Group | Measure-Object cpu -Sum).Sum) / 1000, 2)
  }
} | Format-Table -AutoSize

"`n== top 30 slowest tests =="
$rows | Sort-Object wall_ms -Descending | Select-Object -First 30 pkg, target, test, wall_ms, cpu_ms, write_kb |
  Format-Table -AutoSize -Wrap
