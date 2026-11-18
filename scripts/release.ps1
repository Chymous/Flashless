<#
Publishes the current version as a GitHub release: tag vX.Y.Z, notes from CHANGELOG.md, and the
installer + updater attached. GitHub adds the source code zip/tarball itself.

Needs the GitHub CLI signed in once (`gh auth login`, in your own PowerShell).

Usage:
  .\scripts\release.ps1 -DryRun   # show version, notes and files; change nothing
  .\scripts\release.ps1           # build, tag, push the tag, create the release

Before running: bump the version in Cargo.toml and assets/app.rc, add a "## X.Y.Z" section to
CHANGELOG.md, commit, and push.
#>
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$gh = 'C:\Program Files\GitHub CLI\gh.exe'

$version = (Select-String Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$tag = "v$version"

# release notes: this version's CHANGELOG section
$log = Get-Content CHANGELOG.md -Raw
$m = [regex]::Match($log, "(?ms)^## $([regex]::Escape($version))\s*\r?\n(.*?)(?=^## |\z)")
if (-not $m.Success) { throw "CHANGELOG.md has no '## $version' section." }
$notes = $m.Groups[1].Value.Trim() + "`n`n**Install:** download Flashless-Setup.exe and run it. **Already have Flashless?** Run Flashless-Update-$version.exe.`nWindows may say ""Windows protected your PC"": More info > Run anyway."

if (git status --porcelain) { throw 'Uncommitted changes: commit and push first.' }

$exe = 'target\release\Flashless.exe'
$out = Join-Path $env:TEMP "flashless-release-$version"
$setup = Join-Path $out 'Flashless-Setup.exe'
$update = Join-Path $out "Flashless-Update-$version.exe"

Write-Host "Release $tag"
Write-Host "Files: Flashless-Setup.exe, Flashless-Update-$version.exe (+ source code from GitHub)"
Write-Host "--- notes ---`n$notes`n-------------"
if ($DryRun) { Write-Host 'Dry run: nothing built, tagged or published.'; return }

& $gh auth status *> $null
if ($LASTEXITCODE -ne 0) { throw 'GitHub CLI is not signed in: run `gh auth login` in your own PowerShell first.' }

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
$built = (Get-Item $exe).VersionInfo.ProductVersion
if ($built -ne $version) { throw "built exe says $built, Cargo.toml says $version" }

New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $exe $setup -Force
Copy-Item $exe $update -Force

if (-not (git tag --list $tag)) { git tag -a $tag -m "Flashless $version" }
git push origin $tag
$notesFile = Join-Path $out 'notes.md'
[IO.File]::WriteAllText($notesFile, $notes, (New-Object Text.UTF8Encoding $false))
& $gh release create $tag $setup $update --title "Flashless $version" --notes-file $notesFile --verify-tag
if ($LASTEXITCODE -ne 0) { throw 'gh release create failed' }
Write-Host "Published $tag"
