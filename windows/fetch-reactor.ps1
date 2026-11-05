# A sparse checkout of windows-rs at the commit the app is built and tested with:
# windows-reactor + windows-reactor-setup and the crates they use.
param([string]$Work = 'D:\aula', [string]$Commit = 'e9772320b')
$ErrorActionPreference = 'Stop'
$git = 'C:\Program Files\Git\cmd\git.exe'
$wrs = Join-Path $Work 'wrs'
if (-not (Test-Path (Join-Path $wrs '.git'))) {
  & $git clone -q --filter=blob:none --no-checkout https://github.com/microsoft/windows-rs.git $wrs
}
Push-Location $wrs
& $git sparse-checkout init --cone
& $git sparse-checkout set crates/libs/reactor crates/libs/reactor-setup crates/libs/core crates/libs/collections crates/libs/future crates/libs/reference crates/libs/threading crates/libs/time crates/libs/window crates/libs/implement crates/libs/interface crates/libs/link crates/libs/result crates/libs/strings crates/tests/interop
& $git fetch -q origin $Commit 2>$null
& $git checkout -q $Commit
& $git log --oneline -1
Pop-Location
