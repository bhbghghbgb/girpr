<#
    probe.ps1 - build one commit of gircap and capture its artifact bundle.

    Does two captures of the same binary and refuses to fingerprint unless they
    agree after normalisation.  That guard is what keeps the cross-commit diff
    from being vacuous: a normaliser that silently missed a volatile field would
    make every commit look identical.

    Fingerprints are cached at $FP/<profile>/<sha>.done, so no commit is built
    twice.  Logs land under $LOG.

    -Commit  <sha>   the commit to probe (detached checkout in the worktree)
    -Profile <name>  "default", "gzip512" (forces the .gz path), or an
                   armed-profile name, which only labels the cache
    -Arm <spec>      fail|<needle>|<status>|<body>|<times>  or
                    body|<needle>|<body>|<times>; patches the loopback mock
#>
param(
    [Parameter(Mandatory = $true)][string]$Commit,
    [string]$Profile = "default",
    [string]$Arm = "",
    [switch]$NoCache
)

$ErrorActionPreference = "Stop"
$H = if ($env:GIRCAP_PROBE_HOME) { $env:GIRCAP_PROBE_HOME } else { Join-Path ([IO.Path]::GetTempPath()) "gircap-probe" }
$env:CARGO_TARGET_DIR = "$H\target"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$Tools = $PSScriptRoot

$short = $Commit.Substring(0, 7)
$fpDir = "$H\fp\$Profile\$short"
$capDir = "$H\caps\$Profile\$short"
$log = "$H\log\$Profile-$short.txt"
New-Item -ItemType Directory -Force -Path "$H\log", "$H\fp\$Profile", "$H\caps\$Profile" | Out-Null

if (-not $NoCache -and (Test-Path "$H\fp\$Profile\$short.done")) {
    Write-Output "CACHED $short"
    exit 0
}

if ($Profile -eq "gzip512") { $profileArgs = @("--gzip-threshold", "512") }
else { $profileArgs = @() }

# One worktree for the whole search, checked out to each commit in turn.  It
# shares $CARGO_TARGET_DIR with nothing else, which is the point: dependencies
# compile once and only gircap itself is rebuilt per commit.
if (-not (Test-Path "$H\wt\.git")) {
    Write-Output "creating worktree at $H\wt"
    git -C $Repo worktree add --detach "$H\wt" $Commit 2>&1 | Out-Null
}
Set-Location "$H\wt"
git checkout --force --detach $Commit 2>&1 | Out-Null
$head = (git rev-parse --short HEAD).Trim()
if ($head -ne $short) { throw "checkout landed on $head, wanted $short" }
# Hostile-input profiles need a patched *mock* and the arming probe.  The
# ordinary search path uses neither, so the mock is byte-identical to the tree's
# own and a patched mock can be checked against an unpatched one.
$probeSrc = "$Tools\zz_probe.rs"
if ($Arm -ne "") {
    $probeSrc = "$Tools\zz_probe_arm.rs"
    $mockFile = "$H\wt\tools\capture\tests\common\mod.rs"
    $patch = & python "$Tools\mockpatch.py" $mockFile 2>&1
    $patch | Add-Content $log
    if ($LASTEXITCODE -ne 0) { Write-Output "MOCKPATCH-FAIL $short"; $patch; exit 7 }
}
Copy-Item $probeSrc "$H\wt\tools\capture\tests\zz_probe.rs" -Force

"=== $short $Profile args='$($profileArgs -join " ")' arm='$Arm' ===" | Set-Content $log

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$build = & cargo build --release -p gircap --tests 2>&1
$buildCode = $LASTEXITCODE
$buildSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
"--- build exit=$buildCode in ${buildSec}s ---" | Add-Content $log
$build | Add-Content $log
if ($buildCode -ne 0) { Write-Output "BUILD-FAIL $short"; exit 2 }

$env:GIRCAP_PROBE_OUT = $capDir
$env:GIRCAP_PROBE_N = "2"
$env:GIRCAP_PROBE_ARGS = ($profileArgs -join " ")
$env:GIRCAP_PROBE_ARM = $Arm
# an armed profile is expected to end the run early; its exit code is
# recorded, not asserted
if ($Arm -ne "") { $env:GIRCAP_PROBE_ALLOW_FAIL = "1" }
else { Remove-Item Env:\GIRCAP_PROBE_ALLOW_FAIL -ErrorAction SilentlyContinue }

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$out = & cargo nextest run --release -p gircap --test zz_probe --no-capture 2>&1
$testCode = $LASTEXITCODE
$testSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
"--- probe exit=$testCode in ${testSec}s ---" | Add-Content $log
$out | Add-Content $log
if ($testCode -ne 0) { Write-Output "CAPTURE-FAIL $short"; exit 3 }

# Fingerprint both slots. out-root must be the exact --out value, or the argv
# recorded in MANIFEST.json keeps the slot number and never normalises.
$fpA = "$H\fp\$Profile\.tmp\$short-a"
$fpB = "$H\fp\$Profile\.tmp\$short-b"
foreach ($d in @($fpA, $fpB)) { if (Test-Path $d) { Remove-Item -Recurse -Force $d } }
& python "$Tools\bundle_differ.py" save "$capDir\0" --out $fpA --out-root "$capDir\0" --strict | Out-Null
& python "$Tools\bundle_differ.py" save "$capDir\1" --out $fpB --out-root "$capDir\1" --strict | Out-Null

$selfDiff = & python "$Tools\bundle_differ.py" diff $fpA $fpB 2>&1 | Out-String
$selfDiff | Add-Content $log
if ($selfDiff -notmatch '"identical": true') {
    Write-Output "NONDETERMINISTIC $short"
    Write-Output $selfDiff
    exit 6
}

if (Test-Path $fpDir) { Remove-Item -Recurse -Force $fpDir }
Move-Item $fpA $fpDir
Remove-Item -Recurse -Force $fpB
"$short $(Get-Date -Format s)" | Set-Content "$H\fp\$Profile\$short.done"
$n = (Get-ChildItem -Recurse -File $fpDir).Count
Write-Output "OK $short files=$n build=${buildSec}s probe=${testSec}s"
exit 0