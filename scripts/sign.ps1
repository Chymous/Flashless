<#
Signs the Flashless exes so Windows SmartScreen shows a publisher instead of "Unknown publisher".

Needs ONE of these (nothing secret is stored in this repo):
  A) A code-signing certificate in your Windows certificate store:
       $env:FLASHLESS_SIGN_THUMBPRINT = '<certificate thumbprint>'
  B) Azure Trusted Signing (cheapest option): install the Trusted Signing client tools, then set
       $env:FLASHLESS_TS_DLIB     = 'path\to\Azure.CodeSigning.Dlib.dll'
       $env:FLASHLESS_TS_METADATA = 'path\to\metadata.json'   # endpoint, account, certificate profile
     and sign in with `az login` first.

Usage:
  .\scripts\sign.ps1 -DryRun      # print what would be signed, sign nothing
  .\scripts\sign.ps1              # sign target\release\Flashless.exe and the dist copies
#>
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$timestamp = 'http://timestamp.acs.microsoft.com'

$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
if (-not $signtool) { throw 'signtool.exe not found: install the Windows SDK.' }

$files = @('target\release\Flashless.exe', 'dist\Flashless.exe', 'dist\Flashless-Setup.exe', 'dist\share\Flashless-Setup.exe') +
    @(Get-ChildItem (Join-Path $root 'dist\update') -Filter 'Flashless-Update-*.exe' -ErrorAction SilentlyContinue | ForEach-Object { "dist\update\$($_.Name)" })
$files = $files | ForEach-Object { Join-Path $root $_ } | Where-Object { Test-Path $_ }

if ($env:FLASHLESS_SIGN_THUMBPRINT) {
    $mode = 'certificate store'
    $args0 = @('sign', '/fd', 'SHA256', '/sha1', $env:FLASHLESS_SIGN_THUMBPRINT, '/tr', 'http://timestamp.digicert.com', '/td', 'SHA256')
} elseif ($env:FLASHLESS_TS_DLIB -and $env:FLASHLESS_TS_METADATA) {
    $mode = 'Azure Trusted Signing'
    $args0 = @('sign', '/fd', 'SHA256', '/tr', $timestamp, '/td', 'SHA256', '/dlib', $env:FLASHLESS_TS_DLIB, '/dmdf', $env:FLASHLESS_TS_METADATA)
} else {
    throw 'No signing identity: set FLASHLESS_SIGN_THUMBPRINT, or FLASHLESS_TS_DLIB and FLASHLESS_TS_METADATA (see the top of this script).'
}

Write-Host "Signing with $mode using $signtool"
foreach ($f in $files) {
    if ($DryRun) { Write-Host "  would sign $f"; continue }
    Write-Host "  signing $f"
    & $signtool @args0 $f
    if ($LASTEXITCODE -ne 0) { throw "signtool failed on $f" }
    & $signtool verify /pa /q $f
    if ($LASTEXITCODE -ne 0) { throw "signature check failed on $f" }
}
if ($DryRun) { Write-Host 'Dry run: nothing was signed.' } else { Write-Host 'Done. Rebuild the zips after signing.' }
