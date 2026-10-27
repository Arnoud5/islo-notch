# Islo dev loop: like `vite dev`, but for a Rust app.
# Rebuilds when you save a file in src\ or ui\ (or Cargo.toml / build.rs) and
# restarts Islo. If a build fails, the running Islo stays up; fix and save again.
# Ctrl+C stops the loop and quits the dev Islo.
#
#   .\dev.ps1
#
# The dev Islo runs from target\dev\ (a copy, so the next build can replace
# target\debug\islo.exe), with its own islo.ini there. A dev build closes any
# running Islo by itself (see main.rs), including the one from dist\.

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$built = Join-Path $PSScriptRoot 'target\debug\islo.exe'
$runDir = Join-Path $PSScriptRoot 'target\dev'
$runExe = Join-Path $runDir 'islo.exe'
$watched = @('src', 'ui', 'Cargo.toml', 'build.rs')

# Newest change time among the watched files.
function Get-Stamp {
    Get-ChildItem -Path $watched -Recurse -File -ErrorAction SilentlyContinue |
        Measure-Object -Property LastWriteTimeUtc -Maximum |
        ForEach-Object { $_.Maximum }
}

function Build-And-Restart {
    Write-Host ''
    Write-Host "[dev] building..." -ForegroundColor Cyan
    cargo build
    if ($LASTEXITCODE -ne 0) {
        Write-Host '[dev] build failed: the running Islo keeps going. Fix it and save again.' -ForegroundColor Red
        return
    }
    New-Item -ItemType Directory -Force $runDir | Out-Null
    # A running exe can't be overwritten, but it can be renamed out of the way.
    # (The new Islo deletes islo.exe.old once the old one has quit.)
    if (Test-Path $runExe) {
        Remove-Item "$runExe.old" -Force -ErrorAction SilentlyContinue
        Move-Item $runExe "$runExe.old" -Force
    }
    Copy-Item $built $runExe
    # The new Islo asks the old one to quit cleanly, then takes over.
    Start-Process $runExe -WorkingDirectory $runDir | Out-Null
    Write-Host "[dev] Islo restarted at $(Get-Date -Format T). Watching src\ and ui\ (Ctrl+C to stop)." -ForegroundColor Green
}

try {
    Build-And-Restart
    $last = Get-Stamp
    while ($true) {
        Start-Sleep -Milliseconds 500
        $now = Get-Stamp
        if ($now -ne $last) {
            # editors often save in a few steps: wait until it settles
            Start-Sleep -Milliseconds 300
            $last = Get-Stamp
            Build-And-Restart
        }
    }
}
finally {
    # Ctrl+C: quit the dev Islo cleanly (tray icon removed, volume flyout restored)
    if (Test-Path $runExe) {
        & $runExe --quit
    }
    Write-Host '[dev] stopped.' -ForegroundColor Yellow
}
