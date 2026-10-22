# Build the per-test task list `iotime` consumes from nextest's metadata.
#
#   cargo nextest list --workspace --message-format json > tests.json
#   ./make-tasks.ps1 -Tests tests.json -Out tasks.tsv
#
# One line per test: `<exe><TAB><test name><TAB><cwd>`. The `cwd` is the
# package directory nextest recorded, because a few tests resolve fixtures
# relative to it. Ignored tests are included: invoked directly they report
# "ignored" and exit 0, which is what makes the row count equal the listed
# test count.

param(
    [string]$Tests,
    [string]$Out
)

$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $Tests) { $Tests = Join-Path $repo 'target\io-profile\tests.json' }
if (-not $Out) { $Out = Join-Path $repo 'target\io-profile\tasks.tsv' }
$Tests = [IO.Path]::GetFullPath($Tests)
$Out = [IO.Path]::GetFullPath($Out)

New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null

$j = Get-Content $Tests -Raw | ConvertFrom-Json
$rows = foreach ($p in $j.'rust-suites'.PSObject.Properties) {
    $bin = $p.Value.'binary-path'
    $cwd = $p.Value.cwd
    foreach ($t in $p.Value.testcases.PSObject.Properties) {
        "{0}`t{1}`t{2}" -f $bin, $t.Name, $cwd
    }
}

[IO.File]::WriteAllLines($Out, $rows, [Text.UTF8Encoding]::new($false))
'{0} tasks -> {1}' -f $rows.Count, $Out
