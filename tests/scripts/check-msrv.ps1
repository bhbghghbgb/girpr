#!/usr/bin/env pwsh
<#
.SYNOPSIS
Builds every workspace member with the rust-version its manifest declares.

.DESCRIPTION
The workspace is one lockfile with members that have different minimum
Rust versions: the product and `girsync` need 1.89, `gircap` needs 1.88 and
the two shared policy crates need 1.85. A check of the root package alone
would never notice a change that silently raises a lower member's floor.

The members and their minimums come from `cargo metadata`, so this script
follows the manifests instead of keeping a second list. Each member is then
checked with `cargo +<rust-version> check -p <member> --locked`; a missing
toolchain is installed by rustup first. The same check runs in the `msrv`
workflow.

Run it from anywhere; it works from the repository root:

    pwsh -File tests/scripts/check-msrv.ps1

It needs rustup (not just cargo) and network access on the first run to
install the older toolchains. Their target directories are shared, so the
cost is one toolchain's dependency build per distinct rust-version.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Push-Location $root
try {
    $metadataJson = cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed"
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
            if ($LASTEXITCODE -ne 0) {
                throw "could not install the $ver toolchain"
            }
        }

        & cargo "+$ver" check -p $pkg.name --locked
        if ($LASTEXITCODE -ne 0) {
            Write-Host "FAIL: $($pkg.name) does not build with Rust $ver"
            $failed += "$($pkg.name) ($ver)"
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
