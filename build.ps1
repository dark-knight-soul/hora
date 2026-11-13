# Сборка Hora: собирает release-версию и кладёт exe в папку dist.
# Запуск: powershell -ExecutionPolicy Bypass -File build.ps1
$ErrorActionPreference = 'Stop'
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
Set-Location $PSScriptRoot
# пути вида C:\Users\<имя>\.cargo\... не должны попадать внутрь exe (их видно в выпуске)
# (эта переменная заменяет rustflags из .cargo\config.toml — поэтому встроенный MinGW указан и здесь; флаги разделяются символом 0x1F)
$env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~" + [char]0x1F + "-Clink-self-contained=yes"
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
# защита от подмены DLL рядом с exe (см. tools\pe-harden.ps1)
& "$PSScriptRoot\tools\pe-harden.ps1" target\release\Hora.exe

New-Item -ItemType Directory -Force dist | Out-Null
# если программа запущена — останавливаем, чтобы заменить exe
# (WallClock — прежнее имя программы)
Get-Process Hora, WallClock -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
Copy-Item target\release\Hora.exe dist\Hora.exe -Force
# старый exe под прежним именем больше не нужен
if (Test-Path dist\WallClock.exe) { Remove-Item dist\WallClock.exe -Force }
# рабочая папка dist — переносная версия: настройки и кэш рядом с программой
if (-not (Test-Path dist\portable.txt)) { Set-Content dist\portable.txt "Переносная версия: настройки и кэш хранятся в этой папке." -Encoding UTF8 }
Write-Host "Готово: dist\Hora.exe"
