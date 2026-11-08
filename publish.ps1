<#
  Publish a new Nook version. Every running copy picks it up by itself (within ~20 minutes).

  1. Bump `version` in Cargo.toml (it must be higher than the last release, e.g. 0.3.1 -> 0.3.2)
  2. Run:   .\publish.ps1 -Notes "What changed"

  The script builds the release exe, then creates the GitHub release `v<version>` on
  Anassninab/nook with two files: nook.exe and latest.json ({version, sha256, size}).
  The app reads latest.json, downloads nook.exe, checks size + SHA-256, swaps itself and restarts.
#>
param(
  [string]$Notes = "",
  [string]$Repo = "Anassninab/nook"
)
# native tools (cargo, gh) write progress to stderr; judge them by exit code, not by that
$ErrorActionPreference = "Continue"
$root = $PSScriptRoot

$m = Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
$ver = $m.Matches.Groups[1].Value
Write-Host "Publishing Nook $ver to $Repo"

gh release view "v$ver" --repo $Repo 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) { throw "Release v$ver already exists. Bump the version in Cargo.toml first." }

if ($Notes) { [IO.File]::WriteAllText("$rootssets\whatsnew.txt", $Notes, (New-Object Text.UTF8Encoding($false))) }
Push-Location $root
try { cmd /c "cargo build --release 2>&1" | Out-Host; if ($LASTEXITCODE -ne 0) { throw "build failed" } } finally { Pop-Location }

$exe = "$root\target\release\nook.exe"
$sha = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $exe).Length

$stage = Join-Path $env:TEMP "nook-release-$ver"
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $stage | Out-Null
Copy-Item $exe "$stage\nook.exe"
$json = '{"version":"' + $ver + '","sha256":"' + $sha + '","size":' + $size + '}'
[IO.File]::WriteAllText("$stage\latest.json", $json, (New-Object Text.UTF8Encoding($false)))

if (-not $Notes) { $Notes = "Nook $ver" }
gh release create "v$ver" "$stage\nook.exe" "$stage\latest.json" --repo $Repo --title "Nook $ver" --notes $Notes --latest 2>&1 | Out-Host
if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
Write-Host "Done. Running copies will update on their next check."
