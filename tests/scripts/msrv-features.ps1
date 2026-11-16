# The feature logic of the MSRV check.
#
# Dot-sourced by `check-msrv.ps1` and by `msrv-features.selftest.ps1`. Nothing
# here runs cargo or touches the filesystem: the builds are handed in as a
# scriptblock, so the whole file can be driven by a synthetic member and a
# synthetic build — which is what the self-test does, because no member of the
# workspace has two optional features and the combination branch is therefore
# never reached by a real run of the script.
#
# The five rules it states:
#
# - A member's optional features are the ones its default feature set does not
#   turn on.
# - The configurations that are checked are each feature alone, each pair of
#   them, and the whole set at once.
# - A configuration that does not build fails the member, and the failure
#   names it: cargo allows the combination, so it is a configuration a user
#   can ask for.
# - A feature that does not build alone is not a failure when it builds with
#   one other feature: that pair is the partner it needs.
# - A feature that needs more than one partner at once has no single partner,
#   so it fails on its own — and the failure names the combination that does
#   build, so a reader is told which configuration to try.
#
# The names are compared exactly, because cargo compares them exactly: cargo's
# feature names are case-sensitive, so `default = ["alpha"]` turns on the
# feature named `alpha` and not one named `Alpha`, and a feature named
# `Default` is an ordinary feature rather than the set the default build turns
# on. A comparison that folded the case would drop a feature cargo leaves off,
# and the code behind it would never be held to the member's floor.

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
    # The property lookup is case-insensitive, so the name of the default set
    # is chosen here and the property is read by the name that was chosen.
    $default = $declared | Where-Object { $_ -ceq 'default' }
    $enabled = if ($null -eq $default) { @() } else { @($features.PSObject.Properties[$default].Value) }
    return @($declared | Where-Object { $_ -cne 'default' -and $enabled -cnotcontains $_ })
}

# The configurations of a member's optional features that are checked: each
# feature alone, each pair of them, and the whole set at once.
#
# **Not every subset.** The subsets of two or more of n features are 2^n - n - 1
# of them, so a member with a dozen optional features would ask cargo for four
# thousand builds, and the gate would spend its time on configurations nobody
# asks for. The three kinds here are a quadratic in n instead.
#
# They are the three worth paying for:
#
# - a feature alone, because the code behind it is held to the floor on its
#   own;
# - a pair, because two features that cannot be enabled together is a
#   configuration cargo allows and does not build, and because the pair is the
#   partner a feature that needs one is shown to build with;
# - the whole set, because that is the configuration a user asks for when they
#   want everything the member has.
#
# A subset in between is a configuration too, and it is not checked. What such
# a subset would catch that the three kinds do not is a combination that needs
# a newer Rust only when *exactly* those features are on: the code a subset
# compiles is compiled by the whole set as well, unless another feature
# replaces that code — which is the one shape the whole set cannot see, and a
# rare one. The incompatibilities are caught too: a pair that cannot be
# enabled together is caught by the pair, and an incompatibility written as
# `all(a, b, c)` is caught by the whole set, which holds it as well.
#
# For a member with three optional features or fewer the checked
# configurations are the same set the exhaustive enumeration would name, so
# only a member with four or more sees a difference.
#
# Each configuration is a name — the features it names, joined by commas, the
# way `--features` takes them — and the features it names. The name is written
# once here, so the pair that holds a feature is the same pair the check
# builds.
function Get-FeatureConfigurations($features) {
    $features = @($features)
    $configurations = @()
    foreach ($feature in $features) {
        $configurations += ,([pscustomobject]@{
                Name     = $feature
                Features = @($feature)
            })
    }
    for ($i = 0; $i -lt $features.Count; $i++) {
        for ($j = $i + 1; $j -lt $features.Count; $j++) {
            $pair = @($features[$i], $features[$j])
            $configurations += ,([pscustomobject]@{
                    Name     = ($pair -join ',')
                    Features = $pair
                })
        }
    }
    # The whole set, unless it is one of the pairs above — a member with two
    # optional features has no configuration of its own here.
    if ($features.Count -ge 3) {
        $configurations += ,([pscustomobject]@{
                Name     = ($features -join ',')
                Features = $features
            })
    }
    return $configurations
}

# The exit code of a configuration that was built.
#
# A configuration that is not in the table was never built, so its exit code
# is not known: the missing entry is not a build that failed. Reading it as
# one would fail a member over a configuration cargo was never asked for, and
# it would report the wrong reason for it, so the lookup refuses the missing
# entry instead.
#
# The name it is given is the name the configuration carries, so a lookup
# cannot ask for a spelling the check did not build.
function Get-BuiltExitCode($exitCodes, $name, $member) {
    if (-not $exitCodes.ContainsKey($name)) {
        throw "the configuration '$name' of $member was never built, so its exit code is unknown"
    }
    return [int]$exitCodes[$name]
}

# Check a member's optional features and judge what came back.
#
# The build is handed in, so that the rule this file states can be driven
# without cargo: the script hands it a build of the member at its own
# rust-version, and the self-test hands it a synthetic one.
#
# The scriptblock is called with the name of a configuration and returns the
# exit code of the build.
#
# The configurations of two or more features are checked first: a feature that
# does not build on its own may be one that only builds with a partner, and
# the partner is a pair. A configuration that does not build fails the member,
# and the failure names it: cargo allows the combination, so it is a
# configuration a user can ask for.
#
# A feature that does not build on its own is not failed straight away: it may
# be one that only builds in combination with another, and it is looked at
# with each pair that holds it before the member is failed. A feature that
# builds with no one other feature is broken on its own — and so is one that
# needs two partners at once, because no single partner builds with it
# either: the whole optional set building says nothing about it, because
# every optional feature at once is a configuration of its own and not a
# partner the feature can be shown to need.
#
# A feature that needs two partners at once is failed over the feature, and
# the combination that does build it is named in the failure, because a reader
# holding such a feature has to be told which configuration to try. The whole
# set of the member's features is one of the configurations that is built, so
# the script knows whether it holds: the failure names it when it does.
#
# The failures come back as an array of strings, one per failed configuration,
# and the caller says them.
function Invoke-MemberFeatureChecks {
    param(
        $Pkg,
        [string]$Ver,
        [scriptblock]$Check
    )

    $configurations = @(Get-FeatureConfigurations @(Get-OptionalFeatures $Pkg))
    $failures = @()
    # The exit code of every configuration of two or more features, keyed by
    # its name. The keys are compared exactly: a configuration is named after
    # the features it holds, and cargo does not fold their case. A lookup
    # takes the name from the configuration it is about, so it cannot ask for
    # a spelling the check did not build.
    $exitCodes = [System.Collections.Hashtable]::new([System.StringComparer]::Ordinal)

    foreach ($configuration in $configurations) {
        if ($configuration.Features.Count -lt 2) { continue }
        $code = [int](& $Check $configuration.Name)
        $exitCodes[$configuration.Name] = $code
        if ($code -eq 0) {
            Write-Host "ok: $($Pkg.name) builds with Rust $Ver and --features $($configuration.Name)"
            continue
        }
        Write-Host "FAIL: $($Pkg.name) does not build with Rust $Ver and --features $($configuration.Name) (exit code $code)"
        $failures += "$($Pkg.name) ($Ver, --features $($configuration.Name), exit code $code)"
    }

    foreach ($configuration in $configurations) {
        if ($configuration.Features.Count -ne 1) { continue }
        $feature = $configuration.Features[0]
        $code = [int](& $Check $feature)
        if ($code -eq 0) {
            Write-Host "ok: $($Pkg.name) builds with Rust $Ver and --features $feature"
            continue
        }
        # The feature does not build on its own. It may be one that only
        # builds in combination with another, so the pairs holding it are
        # looked at before the member is failed.
        $holding = $null
        $pairs = 0
        foreach ($pair in $configurations) {
            if ($pair.Features.Count -ne 2) { continue }
            if ($pair.Features -cnotcontains $feature) { continue }
            $pairs++
            if ((Get-BuiltExitCode $exitCodes $pair.Name $Pkg.name) -eq 0) {
                $holding = $pair.Name
                break
            }
        }
        if ($null -ne $holding) {
            Write-Host "ok: $($Pkg.name) builds with Rust $Ver and --features $holding, which holds $feature"
            continue
        }
        if ($pairs -gt 0) {
            Write-Host "    $feature does not build with any one other feature either"
        }
        # A feature that needs two partners at once builds in a combination
        # rather than in a pair, so no pair holds it. The combinations of
        # three or more were built before the features on their own, and the
        # whole set of the member's features is one of them, so the one that
        # holds the feature and does build is known here.
        $combination = $null
        foreach ($held in $configurations) {
            if ($held.Features.Count -lt 3) { continue }
            if ($held.Features -cnotcontains $feature) { continue }
            if ((Get-BuiltExitCode $exitCodes $held.Name $Pkg.name) -eq 0) {
                $combination = $held.Name
                break
            }
        }
        if ($null -ne $combination) {
            Write-Host "    $feature does build with --features $combination"
        }
        Write-Host "FAIL: $($Pkg.name) does not build with Rust $Ver and --features $feature (exit code $code)"
        $detail = if ($null -ne $combination) { "; builds with --features $combination" } else { '' }
        $failures += "$($Pkg.name) ($Ver, --features $feature, exit code $code$detail)"
    }

    return [pscustomobject]@{ Failures = @($failures) }
}
