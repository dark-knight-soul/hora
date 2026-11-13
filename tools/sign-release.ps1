# Подписывает файл выпуска ключом обновлений Hora: рядом появляется <файл>.sig (64 байта, ECDSA P-256
# над SHA-256 в формате r||s — так её проверяет программа через BCryptVerifySignature).
#   powershell -ExecutionPolicy Bypass -File tools\sign-release.ps1 release\HoraSetup.exe
#   powershell -ExecutionPolicy Bypass -File tools\sign-release.ps1 -Backup D:\флешка\hora-key-backup.bin
#     — резервная копия закрытого ключа БЕЗ шифрования DPAPI (храните в надёжном месте!)
param([string]$File, [string]$Backup)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security, System.Core
$keyFile = Join-Path $env:USERPROFILE ".hora-signing\update-key.bin"
if (-not (Test-Path $keyFile)) { throw "Нет ключа подписи ($keyFile). Создайте его: tools\new-signing-key.ps1" }
$priv = [System.Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($keyFile), $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)

if ($Backup) {
    [IO.File]::WriteAllBytes($Backup, $priv)
    Write-Host "Резервная копия ключа: $Backup (не шифрована — не выкладывайте её никуда)"
    return
}
if (-not $File) { throw "Укажите файл: tools\sign-release.ps1 путь\к\файлу" }

$key = [System.Security.Cryptography.CngKey]::Import($priv, [System.Security.Cryptography.CngKeyBlobFormat]::EccPrivateBlob)
$ec = New-Object System.Security.Cryptography.ECDsaCng $key
$ec.HashAlgorithm = [System.Security.Cryptography.CngAlgorithm]::Sha256
$data = [IO.File]::ReadAllBytes($File)
$sig = $ec.SignData($data)
if (-not $ec.VerifyData($data, $sig)) { throw "Подпись не прошла самопроверку" }
[IO.File]::WriteAllBytes("$File.sig", $sig)
Write-Host ("Подписано: {0}.sig ({1} байт)" -f $File, $sig.Length)
