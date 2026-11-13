# Сборка пакетов для выпуска (в папку release):
#   HoraSetup.exe      — установщик (спрашивает: только для меня или для всех)
#   Hora-portable.zip  — переносная версия: распаковал и запустил, всё хранится рядом
#   SHA256SUMS.txt     — контрольные суммы
# Запуск: powershell -ExecutionPolicy Bypass -File package.ps1
$ErrorActionPreference = 'Stop'
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
Set-Location $PSScriptRoot
# пути вида C:\Users\<имя>\.cargo\... не должны попадать внутрь exe (их видно в выпуске)
$env:CARGO_ENCODED_RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~"
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
# защита от подмены DLL рядом с exe (см. tools\pe-harden.ps1)
& "$PSScriptRoot\tools\pe-harden.ps1" target\release\Hora.exe

$out = "release"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory $out | Out-Null

# установщик — та же программа: имя с «Setup» включает режим установки
Copy-Item target\release\Hora.exe "$out\HoraSetup.exe"
# подпись для обновлений (программа ставит обновление, только если подпись сходится)
if (Test-Path (Join-Path $env:USERPROFILE ".hora-signing\update-key.bin")) {
    & "$PSScriptRoot\tools\sign-release.ps1" "$out\HoraSetup.exe"
} else {
    Write-Warning "Нет ключа подписи — выпуск без HoraSetup.exe.sig: установленные копии не примут его как обновление"
}

# переносная версия
$p = Join-Path $out "Hora"
New-Item -ItemType Directory $p, "$p\licenses" | Out-Null
Copy-Item target\release\Hora.exe "$p\Hora.exe"
Set-Content "$p\portable.txt" "Переносная версия Hora: настройки, кэш и обои хранятся в этой папке. Удалите этот файл, чтобы хранить их в %LOCALAPPDATA%\Hora." -Encoding UTF8
Copy-Item README.md, LICENSE $p
Copy-Item assets\fonts\OFL-*.txt "$p\licenses"
Compress-Archive -Path $p -DestinationPath "$out\Hora-portable.zip" -CompressionLevel Optimal
Remove-Item $p -Recurse -Force

Get-ChildItem $out -File | ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name } | Set-Content "$out\SHA256SUMS.txt" -Encoding ASCII
Get-ChildItem $out | Format-Table Name, Length -AutoSize
