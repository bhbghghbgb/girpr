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
kept here.

A feature that does not build on its own is not a failure straight away: it
may be a feature that only builds in combination with another, and a member
with one is checked with the group of its optional features instead — the
group is the configuration that holds every optional feature at once, and it
is the only one that builds such a member. The same check runs in the `msrv`
workflow.

The combinations of those features are checked too — every subset of two or
more of them, with the same command and `--features <a,b>`. Cargo adds
features up, so every combination is a configuration a user can ask for, and a
combination can need a newer Rust than the features it is made of do: the
checks above would not see that, because each of its parts builds on its own. A
member whose features cannot be enabled together is failed by that check — the
combination is one cargo allows, so it is a configuration that has to build.

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

# The features a package declares that its default feature set does not turn on.
#
# The list comes from `cargo metadata`, so it follows the manifests instead of
# keeping a second one here. A package that declares nothing beyond its default
# set has no code the default check misses, and the list is empty for it.
#
# `default` itself is not a feature to check: it is the set the first check
# already builds, and a feature it turns on is built by that check too.
function Get-OptionalFeatures($pkg) {
    $features = $pkg.features
    if ($null -eq $features) {
        return @()
    }
    $declared = @($features.PSObject.Properties | ForEach-Object { $_.Name })
    $default = $features.PSObject.Properties['default']
    $enabled = if ($null -eq $default) { @() } else { @($default.Value) }
    return @($declared | Where-Object { $_ -ne 'default' -and $enabled -notcontains $_ })
}

# The combinations of a member's optional features: every subset of two or
# more of them.
#
# Cargo adds features up, so a user can ask for any combination of a member's
# features, and each combination is a configuration that has to build. The
# subsets are named in the order the features are declared, so the same
# combination always has the same name — and the name of the whole set is the
# group's, which the fallback below looks up.
function Get-FeatureCombinations($features) {
    $features = @($features)
    $combinations = @()
    for ($mask = 1; $mask -lt (1 -shl $features.Count); $mask++) {
        $members = @()
        for ($bit = 0; $bit -lt $features.Count; $bit++) {
            if ($mask -band (1 -shl $bit)) {
                $members += $features[$bit]
            }
        }
        if ($members.Count -ge 2) {
            $combinations += ,($members -join ',')
        }
    }
    return $combinations
}

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

        # The optional features of this member, and the name of the group: the
        # configuration that holds every one of them at once.
        $optional = @(Get-OptionalFeatures $pkg)
        $group = $optional -join ','

        # The combinations of the optional features.
        #
        # Each feature is checked on its own below, and a combination is a
        # configuration a user can ask for as well — so a combination can need
        # a newer Rust than the features it is made of do, and the checks
        # below would not see that, because each of its parts builds on its
        # own. Every subset of two or more of the features is checked here.
        #
        # A member whose features cannot be enabled together is failed by
        # this: cargo allows the combination, so it is a configuration that
        # has to build.
        $combinationExit = @{}
        foreach ($combination in @(Get-FeatureCombinations $optional)) {
            & cargo "+$ver" check -p $pkg.name --all-targets --locked --features $combination
            $combinationCode = $LASTEXITCODE
            $combinationExit[$combination] = $combinationCode
            if ($combinationCode -eq 0) {
                Write-Host "ok: $($pkg.name) builds with Rust $ver and --features $combination"
                continue
            }
            Write-Host "FAIL: $($pkg.name) does not build with Rust $ver and --features $combination (exit code $combinationCode)"
            $failed += "$($pkg.name) ($ver, --features $combination, exit code $combinationCode)"
        }

        # The code behind an optional feature is not in the check above, so it
        # is checked on its own: a member's floor is the floor of every code it
        # ships, and an optional feature is code the member ships when it is
        # asked for.
        #
        # A feature is checked alone, so the code behind it is held to the
        # floor on its own. A member whose features cannot be enabled together
        # is failed by the combination check above, which names the
        # combination; what this check adds is the feature that cannot be
        # enabled on its own. A feature that does not build alone may be
        # one that only builds in combination with another, and a member with
        # one is checked with the group of its optional features instead: the
        # group is the configuration that holds every optional feature at once,
        # and it is the only one that builds such a member. The group is one
        # of the combinations checked above, so its result is looked up there
        # rather than checked twice. The group is looked up once per member,
        # and only once a feature has failed alone.
        # $null until the group has been looked up. A member with one optional
        # feature has no group to enable, so a feature that does not build
        # alone is a failure of its own.
        $groupExit = $null
        foreach ($feature in $optional) {
            & cargo "+$ver" check -p $pkg.name --all-targets --locked --features $feature
            $featureExit = $LASTEXITCODE
            if ($featureExit -eq 0) {
                Write-Host "ok: $($pkg.name) builds with Rust $ver and --features $feature"
                continue
            }
            # The feature does not build on its own. It may be one that only
            # builds in combination with another, so the group is checked
            # before the member is failed.
            if ($null -eq $groupExit -and $optional.Count -gt 1) {
                $groupExit = $combinationExit[$group]
                if ($groupExit -eq 0) {
                    Write-Host "ok: $($pkg.name) builds with Rust $ver and --features $group"
                } else {
                    Write-Host "    the group --features $group does not build either (exit code $groupExit)"
                }
            }
            if ($groupExit -eq 0) {
                Write-Host "ok: $($pkg.name) builds with Rust $ver and --features $group, which holds $feature"
                continue
            }
            Write-Host "FAIL: $($pkg.name) does not build with Rust $ver and --features $feature (exit code $featureExit)"
            $failed += "$($pkg.name) ($ver, --features $feature, exit code $featureExit)"
        }
    }

    if ($failed.Count -gt 0) {
        throw "MSRV check failed: $($failed -join ', ')"
    }
    Write-Host "all $($members.Count) workspace members build with their declared Rust version, optional features and their combinations included"
}
finally {
    Pop-Location
}
