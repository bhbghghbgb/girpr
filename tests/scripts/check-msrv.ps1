#!/usr/bin/env pwsh
<#
.SYNOPSIS
Checks every workspace member, with tests, benches and optional features, at
the rust-version its manifest declares.

.DESCRIPTION
The workspace is one lockfile with members that declare different minimum
Rust versions; each manifest's `rust-version` is the only place the number
lives. A check of the root package alone would never notice a change that
silently raises a lower member's floor.

The members and their minimums come from `cargo metadata`, so this script
follows the manifests instead of keeping a second list. Each member is then
checked with
`cargo +<rust-version> check -p <member> --all-targets --locked`, so test-only
and bench code cannot raise a member's floor unnoticed; a missing toolchain is
installed by rustup first.

The optional features are checked as well, one at a time, with the same command
plus `--features <feature>`. The first check builds what the member builds by
default, and code behind a feature that is off by default is not in it — so a
dependency or a syntax that only the optional code needs would raise the
member's floor without anything here noticing. The feature list comes from
`cargo metadata` too, so it follows the manifests rather than a second list
kept here. The feature names are compared exactly, the way cargo compares
them: cargo's feature names are case-sensitive, so the set the default build
turns on is the feature named `default` and not one named `Default`, and a
feature the default set names is the feature it spells.

A feature that does not build on its own is not a failure straight away: it
may be a feature that only builds in combination with another, and it is
checked with each of the other features before the member is failed — the
feature together with one other is a configuration that has to build on its
own, and one that does build is the partner the feature needs. A feature that
builds with no one other feature is broken on its own and fails the member.
A feature that needs two partners at once is in that shape too, and the
failure names the combination that does build it — the whole set of the
member's features is one of the configurations that is built, so the script
knows whether it holds. The same check runs in the `msrv` workflow.

The combinations of those features are checked too — each pair of them, and
the whole set of them at once — with the same command and `--features <a,b>`.
Cargo adds features up, so every combination is a configuration a user can ask
for, and a combination can need a newer Rust than the features it is made of
do: the checks above would not see that, because each of its parts builds on
its own. A member whose features cannot be enabled together is failed by that
check — the combination is one cargo allows, so it is a configuration that has
to build.

**Not every subset of the features is checked.** The subsets of two or more of
n features are 2^n - n - 1 of them, and a member with a dozen optional
features would ask cargo for four thousand builds. What is checked is each
feature alone, each pair of them, and the whole set at once, which is a
quadratic in the number of features: the code a middle subset compiles is
compiled by the whole set as well, and an incompatibility is caught by the pair
that holds it or by the whole set. The rules, and the reason for them, are in
`msrv-features.ps1`.

Run it from anywhere; it works from the repository root:

    pwsh -File tests/scripts/check-msrv.ps1

It needs rustup (not just cargo) and network access on the first run to
install the older toolchains. Their target directories are shared, so the
cost is one toolchain's dependency build per distinct rust-version.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
# The checks below branch on $LASTEXITCODE to report their own messages. Pin
# the preference that turns a non-zero native exit into a terminating error
# (7.3+), so a host that sets it cannot bypass those branches. Assigning here
# only affects this script's scope, which ends with the run.
$PSNativeCommandUseErrorActionPreference = $false

# The feature rules live in their own file, so that the self-test can drive
# them with a synthetic member: no member of the workspace has two optional
# features today, so a real run of this script never reaches the combination
# branch, and the branch would otherwise be untested.
. (Join-Path $PSScriptRoot 'msrv-features.ps1')

$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Push-Location $root
try {
    $metadataJson = cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata --no-deps --format-version 1 --locked failed with exit code $LASTEXITCODE"
    }
    $metadata = $metadataJson | ConvertFrom-Json
    $members = @($metadata.packages |
        Where-Object { $_.rust_version } |
        Sort-Object rust_version, name)
    if ($members.Count -eq 0) {
        throw "no workspace member declares rust-version"
    }

    $failed = @()
    foreach ($pkg in $members) {
        $ver = $pkg.rust_version
        Write-Host "==> $($pkg.name) requires Rust $ver"

        $probeOutput = & rustup run $ver rustc --version 2>&1
        $probeExit = $LASTEXITCODE
        if ($probeExit -ne 0) {
            $probeText = ($probeOutput | Out-String).Trim()
            # rustup exits 1 both for a toolchain it does not have and for a
            # broken one, so install only when rustup names the requested
            # toolchain as absent. It appends the host triple to a version
            # channel; accept that suffix but no other toolchain name.
            if ($probeText -notmatch "toolchain '$([regex]::Escape($ver))(-[^']*)?' is not installed") {
                throw "the $ver toolchain probe failed (exit code $probeExit): $probeText"
            }
            Write-Host "    installing the $ver toolchain"
            $installOutput = & rustup toolchain install $ver --profile minimal --no-self-update 2>&1
            $installExit = $LASTEXITCODE
            $installText = ($installOutput | Out-String).Trim()
            if ($installExit -ne 0) {
                throw "could not install the $ver toolchain (exit code $installExit): $installText"
            }
            if ($installText) {
                Write-Host $installText
            }
        }

        & cargo "+$ver" check -p $pkg.name --all-targets --locked
        $checkExit = $LASTEXITCODE
        if ($checkExit -ne 0) {
            Write-Host "FAIL: $($pkg.name) does not build with Rust $ver (exit code $checkExit)"
            $failed += "$($pkg.name) ($ver, exit code $checkExit)"
        } else {
            Write-Host "ok: $($pkg.name) builds with Rust $ver"
        }

        # The optional features of this member, and the configurations of them
        # that are checked.
        #
        # The build is handed to the rule as a scriptblock, so that the rule
        # can be driven without cargo. The build's own output goes to the
        # host, so a failing build still says why it failed.
        $check = {
            param($features)
            & cargo "+$ver" check -p $pkg.name --all-targets --locked --features $features | Out-Host
            $LASTEXITCODE
        }
        $result = Invoke-MemberFeatureChecks -Pkg $pkg -Ver $ver -Check $check
        $failed += @($result.Failures)
    }

    if ($failed.Count -gt 0) {
        throw "MSRV check failed: $($failed -join ', ')"
    }
    Write-Host "all $($members.Count) workspace members build with their declared Rust version, optional features and their combinations included"
}
finally {
    Pop-Location
}
