# Build AULA F87 Control for Windows on the build PC (ribs03).
#   powershell -File windows\build.ps1 [-Test] [-Dist] [-Installer]
# Everything lands on D: (sources, target, crate cache, runtime cache, temp).
# windows-reactor 0.100.0 on crates.io lacks Application / NotifyIcon, so both
# reactor crates are patched to a windows-rs checkout at a pinned commit
# (windows\fetch-reactor.ps1 makes it).
param([switch]$Test, [switch]$Dist, [switch]$Installer, [string]$Work = 'D:\aula')
$ErrorActionPreference = 'Stop'
(Get-Process -Id $PID).PriorityClass = 'BelowNormal'
$src = Join-Path $Work 'src'
$wrs = (Join-Path $Work 'wrs') -replace '\\', '/'
foreach ($d in 'cargo-home', 'target', 'cache', 'tmp', 'dist') { New-Item -ItemType Directory -Force (Join-Path $Work $d) | Out-Null }
$env:CARGO_HOME = Join-Path $Work 'cargo-home'
$env:CARGO_TARGET_DIR = Join-Path $Work 'target'
$env:LOCALAPPDATA = Join-Path $Work 'cache'   # windows-reactor-setup's download cache
$env:TEMP = Join-Path $Work 'tmp'
$env:TMP = $env:TEMP
$env:CARGO_BUILD_JOBS = '6'
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$patch = @(
  '--config', "patch.crates-io.windows-reactor.path='$wrs/crates/libs/reactor'",
  '--config', "patch.crates-io.windows-reactor-setup.path='$wrs/crates/libs/reactor-setup'"
)
# PS 5.1 turns a native tool's stderr into errors under 'Stop'; check exit codes instead.
$ErrorActionPreference = 'Continue'
if ($Test) {
  Push-Location $src
  & $cargo test --no-default-features 2>&1 | ForEach-Object { "$_" }
  if ($LASTEXITCODE -ne 0) { throw "core tests failed" }
  Pop-Location
}
Push-Location (Join-Path $src 'windows')
& $cargo build --release @patch 2>&1 | ForEach-Object { "$_" }
if ($LASTEXITCODE -ne 0) { throw "build failed" }
Pop-Location
if ($Dist) {
  $out = Join-Path $env:CARGO_TARGET_DIR 'release'
  $distDir = Join-Path $Work 'dist\AULA F87 Control'
  if (Test-Path $distDir) { Remove-Item -Recurse -Force $distDir }
  New-Item -ItemType Directory -Force $distDir | Out-Null
  # The exe, the staged Windows App Runtime (DLLs + resource folders) and the tray icon.
  $skip = @('build', 'deps', 'examples', 'incremental', '.fingerprint')
  Get-ChildItem $out | Where-Object {
    -not ($skip -contains $_.Name) -and $_.Extension -notin '.d', '.pdb', '.rlib', '.lock', '.exp', '.lib' -and -not $_.Name.StartsWith('.')
  } | ForEach-Object { Copy-Item -Recurse -Force $_.FullName $distDir }
  Copy-Item -Force (Join-Path $src 'windows\assets\aula-f87-control.ico') $distDir
  Copy-Item -Force (Join-Path $src 'LICENSE') $distDir
  "{0} files, {1:N1} MB in {2}" -f (Get-ChildItem -Recurse -File $distDir).Count, ((Get-ChildItem -Recurse -File $distDir | Measure-Object Length -Sum).Sum / 1MB), $distDir
}

if ($Installer) {
  # NSIS from the build PC's MSYS2 (only run, nothing installed).
  $nsis = 'D:\msys64\ucrt64\bin\makensis.exe'
  $version = (Select-String -Path (Join-Path $src 'windows\Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
  $distDir = Join-Path $Work 'dist\AULA F87 Control'
  $setup = Join-Path $Work ("dist\AULA-F87-Control-{0}-Setup.exe" -f $version)
  Push-Location (Join-Path $src 'windows')
  & $nsis -V2 "-DVERSION=$version" "-DDIST=$distDir" "-DOUT=$setup" installer.nsi 2>&1 | ForEach-Object { "$_" }
  if ($LASTEXITCODE -ne 0) { throw "installer failed" }
  Pop-Location
  "{0} ({1:N1} MB) sha256 {2}" -f $setup, ((Get-Item $setup).Length / 1MB), (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
}
