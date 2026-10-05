#Requires -Version 5.1
# Usage: pwsh .github/verify-executable-versions.ps1 [-RepoRoot <path>]
param(
    [string]$RepoRoot = $env:GITHUB_WORKSPACE
)

$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path

$manifest = [IO.File]::ReadAllText((Join-Path $RepoRoot "Cargo.toml"))
$package = [regex]::Match($manifest, '(?ms)^\[workspace\.package\][ \t]*\r?\n(?<package>.*?)(?=^\[|\z)')
$version = [regex]::Match($package.Groups['package'].Value, '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<version>[^"]+)"')
if (-not $version.Success) {
    throw "Cargo.toml under $RepoRoot has no [workspace.package] version"
}
$expectedVersion = $version.Groups['version'].Value

$paths = @(
    "target/aarch64-pc-windows-msvc/release/leopardwm.exe",
    "target/aarch64-pc-windows-msvc/release/leopardwm-watchdog.exe",
    "target/aarch64-pc-windows-msvc/release/leopardwm-cli.exe",
    "target/aarch64-pc-windows-msvc/release/lwm.exe"
)

foreach ($relative in $paths) {
    $path = Join-Path $RepoRoot $relative
    if (-not (Test-Path -LiteralPath $path)) {
        throw "$relative not found under $RepoRoot; build with 'cargo build --release' first"
    }
    $info = (Get-Item -LiteralPath $path).VersionInfo
    if ([string]::IsNullOrWhiteSpace($info.ProductVersion) -or [string]::IsNullOrWhiteSpace($info.FileVersion)) {
        throw "$relative is unversioned; expected ProductVersion and fixed FileVersion $expectedVersion"
    }
    if ($info.ProductVersion -cne $expectedVersion) {
        throw "$relative ProductVersion is '$($info.ProductVersion)'; expected '$expectedVersion'"
    }
    $fixedVersion = "$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart)"
    if ($fixedVersion -cne $expectedVersion) {
        throw "$relative fixed FileVersion is '$fixedVersion'; expected '$expectedVersion'"
    }
    Write-Host "$relative version OK (ProductVersion $($info.ProductVersion), fixed FileVersion $fixedVersion)"
}
