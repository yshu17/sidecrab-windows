# Windows counterpart of `brew install zvoque/tap/sidecrab`: builds from source and
# installs sidecrab.exe + sidecrab-hook.exe into %LOCALAPPDATA%\Programs\sidecrab,
# which is added to the user PATH. Re-run (or `sidecrab-update`) to rebuild after local changes.
$ErrorActionPreference = 'Continue'  # native stderr must not abort; exit codes checked below
$repo = $PSScriptRoot
$dest = Join-Path $env:LOCALAPPDATA 'Programs\sidecrab'

Push-Location (Join-Path $repo 'src-tauri')
try {
    cargo build --release -p sidecrab-hook
    if ($LASTEXITCODE) { throw 'hook build failed' }
    $triple = ((rustc -vV) | Select-String '^host: ').ToString().Substring(6).Trim()
    New-Item -ItemType Directory -Force binaries | Out-Null
    Copy-Item target\release\sidecrab-hook.exe "binaries\sidecrab-hook-$triple.exe" -Force

    Push-Location $repo
    npm install --no-audit --no-fund
    if ($LASTEXITCODE) { throw 'npm install failed' }
    npx tauri build --no-bundle
    if ($LASTEXITCODE) { throw 'app build failed' }
    Pop-Location

    Get-Process sidecrab -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; $_.WaitForExit(5000) | Out-Null }
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item target\release\sidecrab.exe, target\release\sidecrab-hook.exe $dest -Force -ErrorAction Stop
} finally {
    Pop-Location
}

# cmd.exe reads .cmd files in the OEM codepage, so a non-ASCII profile path
# (e.g. Cyrillic user name) gets mangled; reference it via %USERPROFILE% instead.
$cmdRepo = $repo
if ($repo.StartsWith($env:USERPROFILE, [StringComparison]::OrdinalIgnoreCase)) {
    $cmdRepo = '%USERPROFILE%' + $repo.Substring($env:USERPROFILE.Length)
}
# sidecrab-update: pull the tracked branch (fast-forward only) if there is one, then rebuild.
Set-Content -Encoding ascii (Join-Path $dest 'sidecrab-update.cmd') @"
@echo off
git -C "$cmdRepo" rev-parse --abbrev-ref --symbolic-full-name @{u} >nul 2>&1 && (git -C "$cmdRepo" pull --ff-only || exit /b 1) || echo No upstream branch: rebuilding the local checkout as is.
powershell -NoProfile -ExecutionPolicy Bypass -File "$cmdRepo\install-windows.ps1"
"@

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dest) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$dest"), 'User')
    Write-Host "Added $dest to user PATH (open a new terminal)."
}
Write-Host 'Installed. Run `sidecrab` to summon him.'

# Keep the local Claude Code plugin's binaries in sync with this build.
$pluginBin = Join-Path $repo 'plugin\bin'
if (Test-Path $pluginBin) {
    Copy-Item (Join-Path $dest 'sidecrab.exe'), (Join-Path $dest 'sidecrab-hook.exe') $pluginBin -Force -ErrorAction Stop
    # Claude Code caches plugins by version: it only picks these up after a version bump.
    if (Get-Command claude -ErrorAction SilentlyContinue) {
        claude plugin marketplace update local | Out-Host
        claude plugin update sidecrab@local | Out-Host
    } else {
        Write-Host 'Plugin binaries updated. Run: claude plugin marketplace update local; claude plugin update sidecrab@local'
    }
}
