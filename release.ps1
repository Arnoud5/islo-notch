# Islo: build a release and get the file ready for GitHub Releases.
#
#   1. Bump `version` in Cargo.toml (for example 0.3.9 -> 0.3.10).
#   2. Run:  powershell -ExecutionPolicy Bypass -File release.ps1
#   3. On GitHub: Releases -> Draft a new release, tag "v<version>", attach
#      dist\Islo.exe and publish. Every Islo with the updater sees it within
#      a day (or right away with Settings -> About -> Check now).
#
# Islo.exe is the whole app: new users download it, and the updater downloads
# the asset named exactly "Islo.exe", so don't rename it. The notifications
# package (package\) is baked into it.
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Building Islo $version..."
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$built = Get-ChildItem 'target\release\islo.exe', 'target\x86_64-pc-windows-gnu\release\islo.exe' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $built) { throw 'islo.exe was not found under target\' }

$dist = Join-Path $root 'dist'
Remove-Item -Recurse -Force $dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory $dist | Out-Null
Copy-Item $built.FullName (Join-Path $dist 'Islo.exe')

Write-Host ''
Write-Host "Ready in dist\:" -ForegroundColor Green
Get-ChildItem $dist | ForEach-Object { Write-Host ("  {0}  ({1:N1} MB)" -f $_.Name, ($_.Length / 1MB)) }
Write-Host ''
Write-Host "Next: create a GitHub release with tag v$version and attach dist\Islo.exe."
if (Get-Command gh -ErrorAction SilentlyContinue) {
    Write-Host 'Or with the GitHub CLI:'
    Write-Host "  gh release create v$version dist\Islo.exe --title `"Islo $version`" --notes `"What changed...`""
}
