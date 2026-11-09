#!/usr/bin/env pwsh
<#
.SYNOPSIS
Checks every workspace member, with tests and benches, at the rust-version its
manifest declares.

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
installed by rustup first. The same check runs in the `msrv` workflow.

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

        & rustup run $ver rustc --version 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Host "    installing the $ver toolchain"
            & rustup toolchain install $ver --profile minimal --no-self-update
            $installExit = $LASTEXITCODE
            if ($installExit -ne 0) {
                throw "could not install the $ver toolchain (exit code $installExit)"
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
    }

    if ($failed.Count -gt 0) {
        throw "MSRV check failed: $($failed -join ', ')"
    }
    Write-Host "all $($members.Count) workspace members build with their declared Rust version"
}
finally {
    Pop-Location
}
