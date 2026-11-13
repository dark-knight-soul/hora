# Создаёт ключ подписи обновлений Hora (ECDSA P-256) — ОДИН РАЗ на все выпуски.
# Закрытый ключ хранится в профиле пользователя (%USERPROFILE%\.hora-signing\update-key.bin),
# зашифрованный DPAPI: расшифровать его может только эта учётная запись Windows.
# Открытый ключ печатается массивом для src/update.rs — его программа использует для проверки.
# ВАЖНО: если ключ потерять, уже установленные копии не примут следующие обновления
# (их придётся один раз поставить вручную). Сделайте резервную копию: tools\sign-release.ps1 -Backup.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security, System.Core
$dir = Join-Path $env:USERPROFILE ".hora-signing"
$file = Join-Path $dir "update-key.bin"
if (Test-Path $file) { throw "Ключ уже есть: $file. Новый не создаю — иначе установленные копии перестанут принимать обновления." }
New-Item -ItemType Directory -Force $dir | Out-Null

$p = New-Object System.Security.Cryptography.CngKeyCreationParameters
$p.ExportPolicy = [System.Security.Cryptography.CngExportPolicies]::AllowPlaintextExport
$key = [System.Security.Cryptography.CngKey]::Create([System.Security.Cryptography.CngAlgorithm]::ECDsaP256, $null, $p)
$priv = $key.Export([System.Security.Cryptography.CngKeyBlobFormat]::EccPrivateBlob)
$pub = $key.Export([System.Security.Cryptography.CngKeyBlobFormat]::EccPublicBlob)
$enc = [System.Security.Cryptography.ProtectedData]::Protect($priv, $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)
[IO.File]::WriteAllBytes($file, $enc)
[IO.File]::WriteAllBytes((Join-Path $dir "update-key.pub"), $pub)
Write-Host "Ключ создан: $file"
Write-Host ("const PUBKEY: [u8; {0}] = [{1}];" -f $pub.Length, (($pub | ForEach-Object { "0x{0:X2}" -f $_ }) -join ", "))
