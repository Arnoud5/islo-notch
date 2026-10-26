# Islo: turn on notifications in the island (run once).
#
# Windows only lets apps with a "package identity" read notifications. This
# registers package\AppxManifest.xml as a *sparse package* for the Islo.exe in
# this folder: nothing is copied or installed, Windows just gives Islo.exe an
# identity. Registering an unsigned package needs Developer Mode, which needs
# administrator rights once. Afterwards no admin is needed.
#
#   install-notifications.cmd              register (or re-register after an update)
#   install-notifications.cmd -Uninstall   remove the registration
param([switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$manifest = Join-Path $root 'package\AppxManifest.xml'

if ($Uninstall) {
    Get-AppxPackage -Name Islo | Remove-AppxPackage
    Write-Host 'Islo package registration removed.'
    return
}

# 1. The manifest points at Islo.exe, so the newest build (Islo-new.exe or
#    Islo-X.Y.Z.exe) becomes Islo.exe.
$exe = Join-Path $root 'Islo.exe'
Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -like 'Islo*' } | Stop-Process -Force
Start-Sleep -Milliseconds 800
$builds = Get-ChildItem $root -Filter 'Islo-*.exe' | Sort-Object LastWriteTime -Descending
if ($builds -and (-not (Test-Path $exe) -or $builds[0].LastWriteTime -gt (Get-Item $exe).LastWriteTime)) {
    Copy-Item -Force $builds[0].FullName $exe
    Write-Host "Using $($builds[0].Name) as Islo.exe"
}
if (-not (Test-Path $exe)) { throw "Islo.exe was not found in $root" }

# 2. Developer Mode (lets Windows register an unsigned package).
$key = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
$dev = (Get-ItemProperty $key -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
if ($dev -ne 1) {
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if ($admin) {
        New-Item $key -Force | Out-Null
        Set-ItemProperty $key -Name AllowDevelopmentWithoutDevLicense -Value 1 -Type DWord
        Write-Host 'Developer Mode turned on.'
    } else {
        Write-Host 'Developer Mode is off. Turning it on needs administrator rights once.' -ForegroundColor Yellow
        Write-Host "Windows Settings will open: switch on 'Developer Mode', then run install-notifications.cmd again."
        Start-Process 'ms-settings:developers'
        Read-Host 'Press Enter to close'
        return
    }
}

# 3. Logos: Windows reads them from the package folder, and some shells look
#    beside the exe (the external location), so keep a copy in both places.
$assets = Join-Path $root 'Assets'
New-Item -ItemType Directory -Force $assets | Out-Null
Copy-Item -Force (Join-Path $root 'package\Assets\*') $assets -ErrorAction SilentlyContinue

# 4. Register the sparse package for this folder.
Get-AppxPackage -Name Islo | Remove-AppxPackage -ErrorAction SilentlyContinue
Add-AppxPackage -Register $manifest -ExternalLocation $root
Write-Host 'Islo is registered.' -ForegroundColor Green

Start-Process $exe
Write-Host 'Islo restarted. In Islo settings > Modules, turn on Notifications and allow access when Windows asks.'
Read-Host 'Press Enter to close'
