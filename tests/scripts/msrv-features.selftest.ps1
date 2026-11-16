#!/usr/bin/env pwsh
<#
.SYNOPSIS
Checks the MSRV script's feature rules against a synthetic member.

.DESCRIPTION
No member of the workspace has two optional features today — `girsync` has one
— so a real run of `check-msrv.ps1` never reaches the branch that checks a
combination of features. That branch is therefore driven here with a synthetic
member and a synthetic build: the rules are the script's own, taken from
`msrv-features.ps1`, and the builds are answers this script makes up.

What is checked:

- the configurations a member's optional features are checked in, and that the
  middle-sized subsets are not among them;
- a feature that does not build on its own but builds with a partner, which
  does not fail the member;
- the same rule in a member whose whole set of features does not build, where
  the partner is the only thing that can cover the feature;
- a combination that cannot be enabled together, which does fail the member,
  and the failure names it;
- a feature that is broken on its own, which fails the member;
- a feature that needs two partners at once, which fails on its own and whose
  failure names the combination that does build;
- a feature the default set turns on, which is not an optional feature at all;
- a feature whose name differs from `default` only in case, which is an
  ordinary feature and not the default set.

Run it from anywhere:

    pwsh -File tests/scripts/msrv-features.selftest.ps1

It needs nothing but pwsh: no cargo, no rustup, no network.
#>

$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'msrv-features.ps1')

function Assert-Equal($expected, $actual, $what) {
    if ($expected -ne $actual) {
        throw "${what}: expected [$expected], got [$actual]"
    }
}

function Assert-True($condition, $what) {
    if (-not $condition) {
        throw $what
    }
}

# A member with the given optional features, as `cargo metadata` reports one.
#
# The shape is the one the script reads: a name, a rust-version and a set of
# features, of which `default` is the one the default build turns on.
function New-SyntheticMember($features) {
    $declared = [ordered]@{}
    $declared['default'] = @()
    foreach ($feature in $features) {
        $declared[$feature] = @()
    }
    return [pscustomobject]@{
        name         = 'synthetic'
        rust_version = '1.89'
        features     = [pscustomobject]$declared
    }
}

# A build that answers from a table, and records what it was asked for.
#
# `$asked` is a list, so that a case can see every configuration the rule
# checked, and in which order. A configuration the table does not name builds.
#
# The table is keyed exactly: the rules compare feature names the way cargo
# does, so the harness must not answer for a name that is spelled differently.
#
# The closure is taken on the way out, because a scriptblock reaches for the
# variables of the scope it runs in rather than the ones it was written in,
# and this one is written in a function that has returned by the time it runs.
function New-FakeBuild($answers, $asked) {
    $table = [System.Collections.Hashtable]::new([System.StringComparer]::Ordinal)
    foreach ($key in $answers.Keys) {
        $table[$key] = $answers[$key]
    }
    return {
        param($features)
        $null = $asked.Add($features)
        if ($table.ContainsKey($features)) { return $table[$features] }
        return 0
    }.GetNewClosure()
}

# The configurations a member's features are checked in: the combinations
# first, so that a feature that needs a partner is judged against a pair that
# has already been built, and the features on their own after them.
#
# Four features have six pairs and one whole set, so eleven configurations are
# checked — and not the four triples in between, which would make fifteen.
$member = New-SyntheticMember @('a', 'b', 'c', 'd')
$asked = [System.Collections.Generic.List[string]]::new()
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check (New-FakeBuild @{} $asked)
Assert-Equal 'a,b|a,c|a,d|b,c|b,d|c,d|a,b,c,d|a|b|c|d' ($asked -join '|') 'the configurations checked'
Assert-Equal 0 $result.Failures.Count 'a member whose features all build is not failed'

# A feature that does not build on its own but builds with a partner is not a
# failure: the pair is the partner it needs, and the pair builds.
$member = New-SyntheticMember @('alpha', 'beta')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{ 'beta' = 1; 'alpha,beta' = 0 } $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
Assert-Equal 'alpha,beta|alpha|beta' ($asked -join '|') 'the pair is built before the features on their own'
Assert-Equal 0 $result.Failures.Count 'a feature that builds with a partner does not fail the member'

# The same rule where it is harder to tell: a member whose whole set of
# features does not build, because two of them cannot be enabled together.
# The feature that builds only with a partner is still covered by that
# partner — the whole set is not a partner it can be shown to need — so the
# member is not failed over the feature.
$member = New-SyntheticMember @('alpha', 'beta', 'gamma')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{
    'alpha'            = 1
    'alpha,beta'       = 0
    'alpha,gamma'      = 1
    'beta,gamma'       = 1
    'alpha,beta,gamma' = 1
} $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
$featureFailures = @($result.Failures | Where-Object { $_ -like '*--features alpha, exit code*' })
Assert-Equal 0 $featureFailures.Count 'a feature that builds with a partner is not failed'

# The member does fail, over the combinations that did not build, and the
# failure list names them: the feature is not among them, because its
# partner covers it.
Assert-Equal 3 $result.Failures.Count 'a combination that does not build fails the member'
foreach ($combination in @('alpha,gamma', 'beta,gamma', 'alpha,beta,gamma')) {
    $named = @($result.Failures | Where-Object { $_ -like "*--features $combination, exit code*" })
    Assert-Equal 1 $named.Count "the failure names the combination $combination"
}

# The same member, with a pair that cannot be enabled together: cargo allows
# the combination, so it is a configuration that has to build, and the member
# fails with the combination named.
$member = New-SyntheticMember @('alpha', 'beta')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{ 'alpha,beta' = 1 } $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
Assert-Equal 1 $result.Failures.Count 'a combination that cannot be enabled together fails the member'
Assert-True ($result.Failures[0].Contains('--features alpha,beta')) 'the failure names the combination'

# A feature that is broken on its own: it builds with no other feature either,
# so the member fails over the combination and over the feature.
$member = New-SyntheticMember @('alpha', 'beta')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{ 'alpha' = 1; 'alpha,beta' = 1 } $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
Assert-Equal 2 $result.Failures.Count 'a feature that is broken on its own fails the member'
Assert-True ($result.Failures[0].Contains('--features alpha,beta, exit code')) 'the failure names the combination'
Assert-True ($result.Failures[1].Contains('--features alpha, exit code')) 'the failure names the feature'

# A feature that needs two partners at once: no pair holding it builds, so it
# is broken on its own and the member fails over it — but the whole set does
# build, and the failure names it, so a reader is told which configuration to
# try.
$member = New-SyntheticMember @('alpha', 'beta', 'gamma')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{
    'alpha'           = 1
    'alpha,beta'      = 1
    'alpha,gamma'     = 1
    'beta,gamma'      = 0
    'alpha,beta,gamma' = 0
} $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
$featureFailures = @($result.Failures | Where-Object { $_ -like '*--features alpha, exit code*' })
Assert-Equal 1 $featureFailures.Count 'a feature that needs two partners fails on its own'
Assert-True ($featureFailures[0].Contains('--features alpha,beta,gamma')) 'the failure names the combination that builds'

# A member with one optional feature has no partner to be shown to need, so a
# feature that does not build alone is a failure of its own.
$member = New-SyntheticMember @('alpha')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{ 'alpha' = 1 } $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
Assert-Equal 1 $result.Failures.Count 'a lone feature that does not build fails the member'
Assert-True ($result.Failures[0].Contains('--features alpha, exit code')) 'the failure names the feature'

# The whole set of a member's features is a configuration too, and a member
# whose features cannot all be enabled at once fails on it.
$member = New-SyntheticMember @('a', 'b', 'c')
$asked = [System.Collections.Generic.List[string]]::new()
$build = New-FakeBuild @{ 'a,b,c' = 1 } $asked
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check $build
Assert-Equal 1 $result.Failures.Count 'the whole set that cannot be enabled fails the member'
Assert-True ($result.Failures[0].Contains('--features a,b,c')) 'the failure names the whole set'

# A feature the default set turns on is not an optional feature: the first
# check already builds it, so it is not checked again.
$member = [pscustomobject]@{
    name         = 'synthetic'
    rust_version = '1.89'
    features     = [pscustomobject]@{ default = @('alpha'); alpha = @(); beta = @() }
}
$asked = [System.Collections.Generic.List[string]]::new()
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check (New-FakeBuild @{} $asked)
Assert-Equal 'beta' ($asked -join '|') 'a feature the default set turns on is not checked again'

# Cargo's feature names are case-sensitive, so a feature named `Default` is an
# ordinary feature and not the set the default build turns on: it is checked
# on its own, and the feature it names is an optional feature of its own.
$member = [pscustomobject]@{
    name         = 'synthetic'
    rust_version = '1.89'
    features     = [pscustomobject]@{ Default = @('alpha'); alpha = @() }
}
$asked = [System.Collections.Generic.List[string]]::new()
$result = Invoke-MemberFeatureChecks -Pkg $member -Ver '1.89' -Check (New-FakeBuild @{} $asked)
Assert-Equal 'Default,alpha|Default|alpha' ($asked -join '|') 'a feature named Default is an ordinary optional feature'

Write-Host "ok: the MSRV script's feature rules hold for a synthetic member"
