// Скрипт сборки: без внешних утилит (rc/windres) собирает COFF-объект с ресурсами:
//  - манифест (нужен Windows 8+ режим — без него «прозрачные» дочерние окна не работают,
//    плюс DPI-осведомлённость PerMonitorV2);
//  - значок приложения Hora (горы и солнце; рисуется программно, несколько размеров).
use std::{env, fs, path::PathBuf};

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Hora" version="{VER}" processorArchitecture="amd64"/>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // версия берётся из Cargo.toml — при её смене ресурсы надо пересобрать
    println!("cargo:rerun-if-changed=Cargo.toml");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    let sizes: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 128];
    // (тип ресурса, id, данные)
    let mut res: Vec<(u16, u16, Vec<u8>)> = Vec::new();
    let mut group = Vec::new();
    push16(&mut group, 0);
    push16(&mut group, 1);
    push16(&mut group, sizes.len() as u16);
    for (i, &s) in sizes.iter().enumerate() {
        let dib = icon_dib(s);
        group.push(if s >= 256 { 0 } else { s as u8 });
        group.push(if s >= 256 { 0 } else { s as u8 });
        group.push(0);
        group.push(0);
        push16(&mut group, 1);
        push16(&mut group, 32);
        push32(&mut group, dib.len() as u32);
        push16(&mut group, (i + 1) as u16);
        res.push((3, (i + 1) as u16, dib)); // RT_ICON
    }
    res.push((14, 1, group)); // RT_GROUP_ICON
    let ver = version4();
    res.push((24, 1, MANIFEST.replace("{VER}", &format!("{}.{}.{}.0", ver[0], ver[1], ver[2])).into_bytes())); // RT_MANIFEST
    res.push((16, 1, version_info(ver))); // RT_VERSION — «Свойства → Подробно» и имя в диспетчере задач

    let obj = coff_resources(&mut res);
    let path = out.join("wallclock_res.o");
    fs::write(&path, obj).unwrap();
    println!("cargo:rustc-link-arg-bins={}", path.display());

    let lc = out.join("hora_loadcfg.o");
    fs::write(&lc, load_config_object()).unwrap();
    println!("cargo:rustc-link-arg-bins={}", lc.display());
    // на структуру никто не ссылается — без этого её выбросит очистка неиспользуемых секций
    println!("cargo:rustc-link-arg-bins=-Wl,--undefined=_load_config_used");
}

/// Защита от подмены DLL. Системные библиотеки, которые программа подключает при запуске
/// (dwmapi, winhttp, d3d11, dxgi, d2d1, dcomp, mfplat…), Windows иначе ищет СНАЧАЛА в папке exe:
/// чужой dwmapi.dll в «Загрузках» рядом с установщиком выполнился бы (а при установке «для всех» —
/// с правами администратора). Поле DependentLoadFlags «конфигурации загрузки» PE велит искать их
/// только в System32 (Windows 10 1607+). Компоновщик сам заполняет каталог LOAD_CONFIG, если
/// находит глобальный символ _load_config_used — его и кладём в отдельный COFF-объект.
fn load_config_object() -> Vec<u8> {
    const SIZE: usize = 0x94;
    // перед структурой — метка: по ней tools/pe-harden.ps1 найдёт структуру в готовом exe
    // (GNU ld для x64 сам каталог LOAD_CONFIG не заполняет, поэтому его вписывает скрипт после сборки)
    let mut data = b"HORA_LOADCFG_v1\0".to_vec();
    let start = data.len();
    data.resize(start + SIZE, 0);
    data[start..start + 4].copy_from_slice(&(SIZE as u32).to_le_bytes()); // Size
    data[start + 0x4E..start + 0x50].copy_from_slice(&0x0800u16.to_le_bytes()); // DependentLoadFlags = LOAD_LIBRARY_SEARCH_SYSTEM32
    let name = b"_load_config_used\0";
    let raw_ptr = 20 + 40;
    let sym_ptr = raw_ptr + data.len();
    let mut o = Vec::new();
    push16(&mut o, 0x8664); // AMD64
    push16(&mut o, 1); // одна секция
    push32(&mut o, 0);
    push32(&mut o, sym_ptr as u32);
    push32(&mut o, 1); // один символ
    push16(&mut o, 0);
    push16(&mut o, 0);
    o.extend_from_slice(b".rdata\0\0");
    push32(&mut o, 0);
    push32(&mut o, 0);
    push32(&mut o, data.len() as u32);
    push32(&mut o, raw_ptr as u32);
    push32(&mut o, 0);
    push32(&mut o, 0);
    push16(&mut o, 0);
    push16(&mut o, 0);
    push32(&mut o, 0x4040_0040); // INITIALIZED_DATA | ALIGN_8 | MEM_READ
    o.extend_from_slice(&data);
    // символ: длинное имя — в таблице строк (4 нуля + смещение)
    push32(&mut o, 0);
    push32(&mut o, 4);
    push32(&mut o, start as u32); // значение: смещение структуры в секции (после метки)
    push16(&mut o, 1); // секция 1
    push16(&mut o, 0);
    o.push(2); // IMAGE_SYM_CLASS_EXTERNAL
    o.push(0);
    push32(&mut o, (4 + name.len()) as u32);
    o.extend_from_slice(name);
    o
}

fn push16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn push32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn align8(v: &mut Vec<u8>) {
    while v.len() % 8 != 0 {
        v.push(0);
    }
}

/// Собирает COFF-объект с одной секцией .rsrc (как это делает windres).
/// Версия из Cargo.toml: [старшая, младшая, исправление]
fn version4() -> [u16; 3] {
    let v = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut n = [0u16; 3];
    for (i, part) in v.split(['.', '-']).take(3).enumerate() {
        n[i] = part.parse().unwrap_or(0);
    }
    n
}

/// Один узел VS_VERSIONINFO: длина, длина значения, тип, ключ UTF-16, значение и дети (всё выровнено по 4)
fn ver_node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    push16(&mut b, 0); // длина — допишем в конце
    push16(&mut b, value_len);
    push16(&mut b, text as u16);
    for c in key.encode_utf16().chain([0]) {
        push16(&mut b, c);
    }
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b.extend_from_slice(value);
    for ch in children {
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.extend_from_slice(ch);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b
}

fn ver_string(key: &str, val: &str) -> Vec<u8> {
    let mut v = Vec::new();
    let mut n = 0u16;
    for c in val.encode_utf16().chain([0]) {
        push16(&mut v, c);
        n += 1;
    }
    ver_node(key, &v, n, true, &[])
}

/// Ресурс версии: описание, версия, авторские права (язык — английский США, Unicode)
fn version_info(v: [u16; 3]) -> Vec<u8> {
    let ms = (v[0] as u32) << 16 | v[1] as u32;
    let ls = (v[2] as u32) << 16;
    let mut fixed = Vec::new();
    for x in [0xFEEF_04BD, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        push32(&mut fixed, x);
    }
    let text = format!("{}.{}.{}", v[0], v[1], v[2]);
    let strings = [
        ver_string("CompanyName", "Hora contributors"),
        ver_string("FileDescription", "Hora"),
        ver_string("FileVersion", &text),
        ver_string("InternalName", "Hora"),
        ver_string("LegalCopyright", "MIT License, Hora contributors"),
        ver_string("OriginalFilename", "Hora.exe"),
        ver_string("ProductName", "Hora"),
        ver_string("ProductVersion", &text),
    ];
    let table = ver_node("040904B0", &[], 0, true, &strings);
    let sfi = ver_node("StringFileInfo", &[], 0, true, &[table]);
    let mut tr = Vec::new();
    push16(&mut tr, 0x0409);
    push16(&mut tr, 0x04B0);
    let var = ver_node("Translation", &tr, 4, false, &[]);
    let vfi = ver_node("VarFileInfo", &[], 0, true, &[var]);
    ver_node("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &[sfi, vfi])
}

fn coff_resources(res: &mut Vec<(u16, u16, Vec<u8>)>) -> Vec<u8> {
    res.sort_by_key(|r| (r.0, r.1));
    let mut types: Vec<u16> = res.iter().map(|r| r.0).collect();
    types.dedup();

    // 1) считаем смещения всех частей дерева ресурсов
    let mut off = 16 + 8 * types.len();
    let mut type_dir = Vec::new();
    for t in &types {
        type_dir.push(off);
        off += 16 + 8 * res.iter().filter(|r| r.0 == *t).count();
    }
    let mut lang_dir = Vec::new();
    for _ in res.iter() {
        lang_dir.push(off);
        off += 16 + 8;
    }
    let mut data_entry = Vec::new();
    for _ in res.iter() {
        data_entry.push(off);
        off += 16;
    }
    off = (off + 7) & !7;
    let mut blob = Vec::new();
    for r in res.iter() {
        blob.push(off);
        off += (r.2.len() + 7) & !7;
    }

    // 2) пишем дерево
    let mut s = Vec::new();
    let dir_header = |s: &mut Vec<u8>, n: usize| {
        push32(s, 0);
        push32(s, 0);
        push16(s, 0);
        push16(s, 0);
        push16(s, 0);
        push16(s, n as u16);
    };
    dir_header(&mut s, types.len());
    for (i, t) in types.iter().enumerate() {
        push32(&mut s, *t as u32);
        push32(&mut s, 0x8000_0000 | type_dir[i] as u32);
    }
    for t in &types {
        let items: Vec<usize> = (0..res.len()).filter(|&i| res[i].0 == *t).collect();
        dir_header(&mut s, items.len());
        for i in items {
            push32(&mut s, res[i].1 as u32);
            push32(&mut s, 0x8000_0000 | lang_dir[i] as u32);
        }
    }
    for i in 0..res.len() {
        dir_header(&mut s, 1);
        push32(&mut s, 1033);
        push32(&mut s, data_entry[i] as u32);
    }
    let mut relocs = Vec::new();
    for (i, r) in res.iter().enumerate() {
        relocs.push(s.len() as u32);
        push32(&mut s, blob[i] as u32); // будет превращено в RVA линковщиком
        push32(&mut s, r.2.len() as u32);
        push32(&mut s, 0);
        push32(&mut s, 0);
    }
    align8(&mut s);
    for r in res.iter() {
        s.extend_from_slice(&r.2);
        align8(&mut s);
    }

    // 3) заголовки COFF
    let raw_ptr = 20 + 40;
    let reloc_ptr = raw_ptr + s.len();
    let sym_ptr = reloc_ptr + relocs.len() * 10;
    let mut o = Vec::new();
    push16(&mut o, 0x8664); // AMD64
    push16(&mut o, 1);
    push32(&mut o, 0);
    push32(&mut o, sym_ptr as u32);
    push32(&mut o, 1);
    push16(&mut o, 0);
    push16(&mut o, 0);
    o.extend_from_slice(b".rsrc\0\0\0");
    push32(&mut o, 0);
    push32(&mut o, 0);
    push32(&mut o, s.len() as u32);
    push32(&mut o, raw_ptr as u32);
    push32(&mut o, reloc_ptr as u32);
    push32(&mut o, 0);
    push16(&mut o, relocs.len() as u16);
    push16(&mut o, 0);
    push32(&mut o, 0x4000_0040); // INITIALIZED_DATA | MEM_READ
    o.extend_from_slice(&s);
    for r in relocs {
        push32(&mut o, r);
        push32(&mut o, 0); // символ секции .rsrc
        push16(&mut o, 3); // IMAGE_REL_AMD64_ADDR32NB
    }
    o.extend_from_slice(b".rsrc\0\0\0");
    push32(&mut o, 0);
    push16(&mut o, 1);
    push16(&mut o, 0);
    o.push(3); // IMAGE_SYM_CLASS_STATIC
    o.push(0);
    push32(&mut o, 4); // пустая таблица строк
    o
}

/// Рисует значок Hora размера s×s: светлая плитка со скруглёнными углами, две горные вершины
/// и закатное солнце над ними. Возвращает его в формате DIB для RT_ICON.
fn icon_dib(s: u32) -> Vec<u8> {
    let n = s as f32;
    let u = n / 120.0; // рисунок задан в сетке 120×120
    let ss = if s <= 32 { 6 } else { 4 }; // суперсэмплинг для сглаживания (маленьким — погуще)
    let mut px = vec![[0f32; 4]; (s * s) as usize]; // премультиплицированные RGBA
    // на маленьких размерах линии и солнце толще, иначе в трее они растворяются
    let stroke = (10.0 * u).max(if s <= 20 { 2.1 } else if s <= 32 { 2.6 } else { 0.0 });
    let sun_r = (8.0 * u).max(if s <= 32 { 2.0 } else { 0.0 });
    let radius = 28.0 * u;
    let peaks = [(28.0, 94.0), (47.0, 34.0), (60.0, 68.0), (73.0, 34.0), (92.0, 94.0)];
    let pts: Vec<(f32, f32)> = peaks.iter().map(|(x, y)| (x * u, (y + 6.0) * u)).collect();
    let (sx0, sy0) = (86.0 * u, 26.0 * u);
    const CREAM: [f32; 3] = [0.957, 0.929, 0.894];
    const INK: [f32; 3] = [0.122, 0.141, 0.200];
    const SUN: [f32; 3] = [0.910, 0.463, 0.227];
    for y in 0..s {
        for x in 0..s {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = x as f32 + (sx as f32 + 0.5) / ss as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / ss as f32;
                    // плитка: скруглённый квадрат на весь значок
                    let qx = ((fx - n / 2.0).abs() - (n / 2.0 - radius)).max(0.0);
                    let qy = ((fy - n / 2.0).abs() - (n / 2.0 - radius)).max(0.0);
                    if (qx * qx + qy * qy).sqrt() > radius {
                        continue;
                    }
                    let mut col = CREAM;
                    let on_line = pts.windows(2).any(|w| seg_dist(fx, fy, w[0].0, w[0].1, w[1].0, w[1].1) <= stroke / 2.0);
                    if on_line {
                        col = INK;
                    }
                    if ((fx - sx0).powi(2) + (fy - sy0).powi(2)).sqrt() <= sun_r {
                        col = SUN;
                    }
                    acc[0] += col[0];
                    acc[1] += col[1];
                    acc[2] += col[2];
                    acc[3] += 1.0;
                }
            }
            let k = (ss * ss) as f32;
            px[(y * s + x) as usize] = [acc[0] / k, acc[1] / k, acc[2] / k, acc[3] / k];
        }
    }
    let mask_row = ((s + 31) / 32 * 4) as usize;
    let mut v = Vec::new();
    push32(&mut v, 40);
    push32(&mut v, s);
    push32(&mut v, s * 2);
    push16(&mut v, 1);
    push16(&mut v, 32);
    push32(&mut v, 0);
    push32(&mut v, s * s * 4 + (mask_row as u32) * s);
    push32(&mut v, 0);
    push32(&mut v, 0);
    push32(&mut v, 0);
    push32(&mut v, 0);
    for y in (0..s).rev() {
        for x in 0..s {
            let p = px[(y * s + x) as usize];
            let a = p[3];
            let un = |ch: f32| if a > 0.0 { (ch / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            v.push(un(p[2]));
            v.push(un(p[1]));
            v.push(un(p[0]));
            v.push((a * 255.0).round() as u8);
        }
    }
    v.extend(std::iter::repeat(0u8).take(mask_row * s as usize));
    v
}

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (qx, qy) = (ax + dx * t, ay + dy * t);
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}
