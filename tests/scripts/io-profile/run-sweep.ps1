# One serial sweep: list the tests, build the accountant, run every test in
# its own job object, then write the analysis tables.
#
#   ./run-sweep.ps1
#   ./run-sweep.ps1 -TempRoot Z:\tmp -Out target\io-profile\ram
#
# The sweep is serial on purpose: parallel tests overlap their I/O and make
# per-test attribution meaningless. Use `cargo nextest run` for the parallel
# wall-clock baseline, and run this once on the disk under test and once on a
# RAM disk to separate storage time from CPU time and deliberate sleeps.
#
# Outputs, under -Out (default target\io-profile, which git ignores):
#   tests.json         nextest's metadata, the input to analyze.ps1
#   tasks.tsv          exe<TAB>test<TAB>cwd for every listed test
#   tasks.tsv.csv      one row per test: wall, CPU, I/O counters
#   progress.txt       stderr progress from iotime
#   sweep-elapsed.txt  SWEEP_SECONDS=...
#   analysis\          tests-ranked.csv, by-suite.csv, by-crate.csv, summary.txt
#                      and, with -CompareCsv, compare-tests.csv / compare-suites.csv

param(
    [string]$Out,
    [string]$TempRoot,
    [string]$CompareCsv,
    [int]$Concurrency = 1,
    [switch]$SkipList
)

$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $Out) { $Out = Join-Path $repo 'target\io-profile' }
$Out = [IO.Path]::GetFullPath($Out)
New-Item -ItemType Directory -Force $Out | Out-Null

$testsJson = Join-Path $Out 'tests.json'
$tasks = Join-Path $Out 'tasks.tsv'
$iotimeTarget = Join-Path $Out 'iotime-target'
$iotime = Join-Path $iotimeTarget 'release\iotime.exe'

Push-Location $repo
try {
    if (-not $SkipList) {
        'listing tests...'
        & cargo nextest list --workspace --message-format json |
            Out-File -FilePath $testsJson -Encoding utf8
        if ($LASTEXITCODE -ne 0) { throw 'cargo nextest list failed' }
    }
    if (-not (Test-Path $testsJson)) {
        throw "missing $testsJson (drop -SkipList or pass an existing -Out)"
    }

    & (Join-Path $PSScriptRoot 'make-tasks.ps1') -Tests $testsJson -Out $tasks

    if (-not (Test-Path $iotime)) {
        'building iotime...'
        & cargo build --release --manifest-path (Join-Path $PSScriptRoot 'iotime\Cargo.toml') `
            --target-dir $iotimeTarget
        if ($LASTEXITCODE -ne 0) { throw 'iotime build failed' }
    }

    $saved = @{}
    foreach ($name in 'TEMP', 'TMP', 'TMPDIR') {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name)
    }
    try {
        if ($TempRoot) {
            $TempRoot = [IO.Path]::GetFullPath($TempRoot)
            New-Item -ItemType Directory -Force $TempRoot | Out-Null
            foreach ($name in 'TEMP', 'TMP', 'TMPDIR') {
                [Environment]::SetEnvironmentVariable($name, $TempRoot)
            }
            "TEMP=$TempRoot"
        }

        'running the sweep; this takes minutes...'
        $sw = [Diagnostics.Stopwatch]::StartNew()
        & $iotime $tasks $Concurrency 2> (Join-Path $Out 'progress.txt')
        $code = $LASTEXITCODE
        $sw.Stop()
        if ($code -ne 0) { throw "iotime exited $code" }
    }
    finally {
        foreach ($name in $saved.Keys) {
            [Environment]::SetEnvironmentVariable($name, $saved[$name])
        }
    }

    $seconds = $sw.Elapsed.TotalSeconds.ToString('0.0000000', [Globalization.CultureInfo]::InvariantCulture)
    "SWEEP_SECONDS=$seconds" | Set-Content (Join-Path $Out 'sweep-elapsed.txt')
    "swept $tasks in $seconds s"

    $analysis = Join-Path $Out 'analysis'
    $splat = @{
        Tests = $testsJson
        Csv   = "$tasks.csv"
        Out   = $analysis
    }
    if ($CompareCsv) {
        $splat.CompareCsv = $CompareCsv
    }
    & (Join-Path $PSScriptRoot 'analyze.ps1') @splat |
        Tee-Object -FilePath (Join-Path $analysis 'summary.txt')
}
finally {
    Pop-Location
}
