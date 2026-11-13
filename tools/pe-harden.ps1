# Защита готового exe от подмены DLL: вписывает в заголовок PE каталог LOAD_CONFIG, указывающий
# на структуру с DependentLoadFlags = LOAD_LIBRARY_SEARCH_SYSTEM32 (её кладёт build.rs, перед ней
# метка HORA_LOADCFG_v1). Тогда системные DLL, нужные при запуске (dwmapi, winhttp, d3d11 и т. д.),
# Windows ищет только в System32, а не в папке рядом с exe (например, в «Загрузках»).
# GNU ld для x64 этот каталог сам не заполняет — поэтому делаем это после сборки.
# Запуск: powershell -ExecutionPolicy Bypass -File tools\pe-harden.ps1 путь\к\Hora.exe
param([Parameter(Mandatory = $true)][string]$Exe)
$ErrorActionPreference = 'Stop'

$b = [IO.File]::ReadAllBytes($Exe)
$magic = [Text.Encoding]::ASCII.GetBytes("HORA_LOADCFG_v1")

# ищем метку (сначала по первому байту — так быстро даже на мегабайтах)
$idx = -1
$i = [Array]::IndexOf($b, $magic[0])
while ($i -ge 0 -and $i -le $b.Length - $magic.Length) {
    $ok = $true
    for ($k = 1; $k -lt $magic.Length; $k++) { if ($b[$i + $k] -ne $magic[$k]) { $ok = $false; break } }
    if ($ok) { $idx = $i; break }
    $i = [Array]::IndexOf($b, $magic[0], $i + 1)
}
if ($idx -lt 0) { throw "метка HORA_LOADCFG_v1 не найдена в $Exe" }
$off = $idx + 16 # структура сразу после метки (16 байт с нулём на конце)
if ([BitConverter]::ToUInt32($b, $off) -ne 0x94) { throw "после метки нет структуры конфигурации загрузки" }

# переводим смещение в файле в адрес в памяти (RVA) по таблице секций
$pe = [BitConverter]::ToInt32($b, 0x3C)
if ([BitConverter]::ToUInt32($b, $pe) -ne 0x4550) { throw "это не PE-файл" }
$opt = $pe + 24
if ([BitConverter]::ToUInt16($b, $opt) -ne 0x20B) { throw "ожидался 64-битный PE" }
$nsec = [BitConverter]::ToUInt16($b, $pe + 6)
$sec = $opt + [BitConverter]::ToUInt16($b, $pe + 20)
$rva = $null
for ($s = 0; $s -lt $nsec; $s++) {
    $h = $sec + $s * 40
    $va = [BitConverter]::ToUInt32($b, $h + 12)
    $rsize = [BitConverter]::ToUInt32($b, $h + 16)
    $raw = [BitConverter]::ToUInt32($b, $h + 20)
    if ($off -ge $raw -and $off -lt $raw + $rsize) { $rva = $va + ($off - $raw); break }
}
if ($null -eq $rva) { throw "структура не попала ни в одну секцию" }

# каталог данных №10 (LOAD_CONFIG): адрес и размер
$dir = $opt + 112 + 10 * 8
[BitConverter]::GetBytes([uint32]$rva).CopyTo($b, $dir)
[BitConverter]::GetBytes([uint32]0x94).CopyTo($b, $dir + 4)
[IO.File]::WriteAllBytes($Exe, $b)
Write-Host ("LOAD_CONFIG вписан: RVA 0x{0:X}, DependentLoadFlags 0x{1:X}" -f $rva, [BitConverter]::ToUInt16($b, $off + 0x4E))
