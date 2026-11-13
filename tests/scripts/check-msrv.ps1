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
kept here. The same check runs in the `msrv` workflow.

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

        # The code behind an optional feature is not in the check above, so it
        # is checked on its own: a member's floor is the floor of every code it
        # ships, and an optional feature is code the member ships when it is
        # asked for. Each feature is checked alone, so a member whose features
        # are mutually exclusive still checks.
        foreach ($feature in (Get-OptionalFeatures $pkg)) {
            & cargo "+$ver" check -p $pkg.name --all-targets --locked --features $feature
            $featureExit = $LASTEXITCODE
            if ($featureExit -ne 0) {
                Write-Host "FAIL: $($pkg.name) does not build with Rust $ver and --features $feature (exit code $featureExit)"
                $failed += "$($pkg.name) ($ver, --features $feature, exit code $featureExit)"
            } else {
                Write-Host "ok: $($pkg.name) builds with Rust $ver and --features $feature"
            }
        }
    }

    if ($failed.Count -gt 0) {
        throw "MSRV check failed: $($failed -join ', ')"
    }
    Write-Host "all $($members.Count) workspace members build with their declared Rust version, optional features included"
}
finally {
    Pop-Location
}
