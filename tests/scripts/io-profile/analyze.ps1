# Turn one (or two) `iotime` sweeps into the tables used by
# `test-io-report.md`.
#
#   ./analyze.ps1 -Tests tests.json -Csv tasks.tsv.csv -Out analysis
#   ./analyze.ps1 -Tests tests.json -Csv hdd\tasks.tsv.csv `
#                 -CompareCsv ram\tasks.tsv.csv -Out analysis
#
# `WaitMs` is wall minus CPU. It is deliberately *not* called "I/O wait":
# it also contains every deliberate sleep a test performs. Only the second
# sweep separates the two: a sleep test has the same wall in both.
#
# Outputs under -Out:
#   tests-ranked.csv     every test, sorted by wait
#   by-suite.csv         per-suite totals
#   by-crate.csv         per-crate totals
#   compare-tests.csv    only with -CompareCsv: per-test delta
#   compare-suites.csv   only with -CompareCsv: per-suite delta
#
# In the report the baseline is the HDD (`E:`) sweep and the comparison is
# the RAM-disk (`Z:`) sweep, so `DeltaS` is storage-attributable time.

param(
    [string]$Tests,
    [string]$Csv,
    [string]$CompareCsv,
    [string]$Out
)

$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $Out) { $Out = Join-Path $repo 'target\io-profile\analysis' }
if (-not $Tests) { $Tests = Join-Path $repo 'target\io-profile\tests.json' }
if (-not $Csv) { $Csv = Join-Path $repo 'target\io-profile\tasks.tsv.csv' }
$Tests = [IO.Path]::GetFullPath($Tests)
$Csv = [IO.Path]::GetFullPath($Csv)
$Out = [IO.Path]::GetFullPath($Out)

New-Item -ItemType Directory -Force -Path $Out | Out-Null

$j = Get-Content $Tests -Raw | ConvertFrom-Json
$suiteOf = @{}
foreach ($p in $j.'rust-suites'.PSObject.Properties) {
    $suiteOf[$p.Value.'binary-path'] = @{ Suite = $p.Name; Crate = $p.Value.'package-name' }
}

function Convert-Rows([string]$Path) {
    Import-Csv $Path | ForEach-Object {
        $s = $suiteOf[$_.exe]
        $wall = [double]$_.wall_ms
        $cpu = [double]$_.cpu_period_ms
        [pscustomobject]@{
            Suite    = if ($s) { $s.Suite } else { '?' }
            Crate    = if ($s) { $s.Crate } else { '?' }
            Test     = $_.name
            WallMs   = $wall
            CpuMs    = $cpu
            WaitMs   = [math]::Max(0, $wall - $cpu)
            ReadOps  = [int64]$_.read_ops
            WriteOps = [int64]$_.write_ops
            OtherOps = [int64]$_.other_ops
            ReadMB   = [math]::Round([int64]$_.read_bytes / 1MB, 3)
            WriteMB  = [math]::Round([int64]$_.write_bytes / 1MB, 3)
            Ops      = [int64]$_.read_ops + [int64]$_.write_ops + [int64]$_.other_ops
            Exit     = [int]$_.exit
        }
    }
}

$rows = @(Convert-Rows $Csv)
if ($rows.Count -eq 0) { throw "no rows in $Csv" }

function Sum($items, $prop) { ($items | Measure-Object $prop -Sum).Sum }

$bySuite = $rows | Group-Object Suite | ForEach-Object {
    $g = $_.Group
    [pscustomobject]@{
        Suite    = $_.Name
        Tests    = $g.Count
        WallS    = [math]::Round((Sum $g WallMs) / 1000, 1)
        CpuS     = [math]::Round((Sum $g CpuMs) / 1000, 1)
        WaitS    = [math]::Round((Sum $g WaitMs) / 1000, 1)
        WaitPct  = [math]::Round(100 * (Sum $g WaitMs) / [math]::Max(1, (Sum $g WallMs)), 1)
        ReadOps  = (Sum $g ReadOps)
        WriteOps = (Sum $g WriteOps)
        OtherOps = (Sum $g OtherOps)
        WriteMB  = [math]::Round((Sum $g WriteMB), 1)
        ReadMB   = [math]::Round((Sum $g ReadMB), 1)
    }
} | Sort-Object WallS -Descending

$byCrate = $rows | Group-Object Crate | ForEach-Object {
    $g = $_.Group
    [pscustomobject]@{
        Crate   = $_.Name
        Tests   = $g.Count
        WallS   = [math]::Round((Sum $g WallMs) / 1000, 1)
        CpuS    = [math]::Round((Sum $g CpuMs) / 1000, 1)
        WaitS   = [math]::Round((Sum $g WaitMs) / 1000, 1)
        WriteMB = [math]::Round((Sum $g WriteMB), 1)
        ReadMB  = [math]::Round((Sum $g ReadMB), 1)
    }
} | Sort-Object WallS -Descending

$bySuite | Export-Csv (Join-Path $Out 'by-suite.csv') -NoTypeInformation
$byCrate | Export-Csv (Join-Path $Out 'by-crate.csv') -NoTypeInformation
$rows | Sort-Object WaitMs -Descending | Export-Csv (Join-Path $Out 'tests-ranked.csv') -NoTypeInformation

'=== TOTAL ==='
[pscustomobject]@{
    Tests   = $rows.Count
    WallS   = [math]::Round((Sum $rows WallMs) / 1000, 1)
    CpuS    = [math]::Round((Sum $rows CpuMs) / 1000, 1)
    WaitS   = [math]::Round((Sum $rows WaitMs) / 1000, 1)
    WriteMB = [math]::Round((Sum $rows WriteMB), 1)
    ReadMB  = [math]::Round((Sum $rows ReadMB), 1)
    Ops     = (Sum $rows Ops)
    NonZeroExit = @($rows | Where-Object { $_.Exit -ne 0 }).Count
} | Format-List

'=== BY CRATE ==='
$byCrate | Format-Table -AutoSize | Out-String -Width 200

'=== BY SUITE ==='
$bySuite | Format-Table -AutoSize | Out-String -Width 220

'=== TOP 40 BY WAIT ==='
$rows | Sort-Object WaitMs -Descending |
    Select-Object -First 40 Suite, Test,
        @{n = 'WallS'; e = { [math]::Round($_.WallMs / 1000, 1) } },
        @{n = 'CpuS'; e = { [math]::Round($_.CpuMs / 1000, 2) } },
        @{n = 'WaitS'; e = { [math]::Round($_.WaitMs / 1000, 1) } },
        Ops, WriteMB, ReadMB |
    Format-Table -AutoSize | Out-String -Width 240

if ($CompareCsv) {
    $CompareCsv = [IO.Path]::GetFullPath($CompareCsv)
    $other = @(Convert-Rows $CompareCsv)
    if ($other.Count -eq 0) { throw "no rows in $CompareCsv" }

    $baseMap = @{}
    foreach ($r in $rows) { $baseMap["$($r.Suite)|$($r.Test)"] = $r }

    $cmp = foreach ($r in $other) {
        $b = $baseMap["$($r.Suite)|$($r.Test)"]
        if (-not $b) { continue }
        [pscustomobject]@{
            Suite        = $r.Suite
            Test         = $r.Test
            BaseWallS    = [math]::Round($b.WallMs / 1000, 2)
            OtherWallS   = [math]::Round($r.WallMs / 1000, 2)
            DeltaS       = [math]::Round(($b.WallMs - $r.WallMs) / 1000, 2)
            BaseCpuS     = [math]::Round($b.CpuMs / 1000, 2)
            OtherCpuS    = [math]::Round($r.CpuMs / 1000, 2)
            BaseWriteMB  = $b.WriteMB
            OtherWriteMB = $r.WriteMB
            BaseWriteOps = $b.WriteOps
            OtherWriteOps = $r.WriteOps
            Exit         = $r.Exit
        }
    }

    $cmp | Sort-Object DeltaS -Descending |
        Export-Csv (Join-Path $Out 'compare-tests.csv') -NoTypeInformation

    $cmpSuites = $cmp | Group-Object Suite | ForEach-Object {
        $g = $_.Group
        [pscustomobject]@{
            Suite      = $_.Name
            Tests      = $g.Count
            BaseWallS  = [math]::Round((Sum $g BaseWallS), 1)
            OtherWallS = [math]::Round((Sum $g OtherWallS), 1)
            DeltaS     = [math]::Round((Sum $g DeltaS), 1)
        }
    } | Sort-Object DeltaS -Descending
    $cmpSuites | Export-Csv (Join-Path $Out 'compare-suites.csv') -NoTypeInformation

    '=== COMPARISON TOTAL ==='
    [pscustomobject]@{
        Tests      = $cmp.Count
        BaseWallS  = [math]::Round((Sum $cmp BaseWallS), 1)
        OtherWallS = [math]::Round((Sum $cmp OtherWallS), 1)
        DeltaS     = [math]::Round((Sum $cmp DeltaS), 1)
        NonZeroExit = @($cmp | Where-Object { $_.Exit -ne 0 }).Count
    } | Format-List

    '=== STORAGE DELTA BY SUITE ==='
    $cmpSuites | Format-Table -AutoSize | Out-String -Width 120

    '=== TOP 40 BY DELTA ==='
    $cmp | Sort-Object DeltaS -Descending | Select-Object -First 40 |
        Format-Table -AutoSize | Out-String -Width 200
}
