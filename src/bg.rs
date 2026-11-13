// Подготовка фона для окна часов.
// В Windows 11 24H2 рабочий стол собирается так, что прозрачное окно внутри него не видно.
// Поэтому окно часов непрозрачное и закрывает весь монитор: оно само рисует обои + часы.
// Чтобы не держать картинку в памяти, обои один раз масштабируются под каждый монитор (как это делает Windows)
// и сохраняются в файлы cache\bg-N.raw; при перерисовке читается только полоска под часами.
use crate::util::{log, wide};
use std::ffi::c_void;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
use windows_sys::Win32::System::Registry::*;

const MAGIC: &[u8; 4] = b"WCBG";
const HEADER: u64 = 12;

fn reg_str(key: &str, name: &str) -> Option<String> {
    unsafe {
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut size);
        if r != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

/// Что сейчас стоит на рабочем столе: файл, способ растяжения, цвет фона.
pub struct Source {
    pub path: Option<PathBuf>,
    pub style: u32, // 0 центр, 1 плитка, 2 растянуть, 6 вписать, 10 заполнить, 22 на все экраны
    pub color: u32, // 0x00BBGGRR
}

pub fn current_source() -> Source {
    let mut path = reg_str("Control Panel\\Desktop", "WallPaper").map(PathBuf::from).filter(|p| p.is_file());
    if path.is_none() {
        // Windows хранит копию текущих обоев здесь
        if let Ok(appdata) = std::env::var("APPDATA") {
            let t = PathBuf::from(appdata).join("Microsoft\\Windows\\Themes\\TranscodedWallpaper");
            if t.is_file() {
                path = Some(t);
            }
        }
    }
    let mut style: u32 = reg_str("Control Panel\\Desktop", "WallpaperStyle").and_then(|s| s.trim().parse().ok()).unwrap_or(10);
    if style == 0 && reg_str("Control Panel\\Desktop", "TileWallpaper").map(|s| s.trim() == "1").unwrap_or(false) {
        style = 1;
    }
    let color = reg_str("Control Panel\\Colors", "Background")
        .map(|s| {
            let v: Vec<u32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 { v[0] | (v[1] << 8) | (v[2] << 16) } else { 0 }
        })
        .unwrap_or(0);
    Source { path, style, color }
}

use windows_sys::Win32::Foundation::RECT;

/// Файл фона для монитора с номером `i` (номер — в списке clock::monitors()).
pub fn file_for(cache: &Path, i: usize) -> PathBuf {
    cache.join(format!("bg-{i}.raw"))
}

/// Прошлый фон монитора — нужен для анимации перехода со старых обоев на новые.
pub fn prev_for(cache: &Path, i: usize) -> PathBuf {
    cache.join(format!("bg-{i}.prev.raw"))
}

/// Ключ, по которому понятно, что фон уже собран под эти обои и этот монитор.
fn key_of(path: &Option<PathBuf>, src: &Source, r: &RECT) -> String {
    let (p, m) = match path {
        Some(p) => {
            let m = std::fs::metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            (p.display().to_string(), m)
        }
        None => (String::new(), 0),
    };
    format!("{p}|{m}|{}|{}|{},{},{},{}", src.style, src.color, r.left, r.top, r.right, r.bottom)
}

/// Какие обои стоят на каждом из мониторов (по словам Windows; если не ответила — общие из реестра).
fn wallpapers_for(mons: &[RECT], src: &Source) -> Vec<Option<PathBuf>> {
    // «на все экраны»: картинка одна; Windows при этом отдаёт по мониторам устаревшие пути — берём общий
    if src.style == 22 {
        return mons.iter().map(|_| src.path.clone()).collect();
    }
    let dw = crate::wallpaper::DesktopWallpaper::new();
    let list = dw.as_ref().map(|d| d.monitors()).unwrap_or_default();
    mons.iter()
        .map(|r| {
            let own = list.iter().find(|(_, mr)| crate::util::same_rect(mr, r)).and_then(|(id, _)| dw.as_ref().and_then(|d| d.wallpaper_of(id)));
            own.filter(|p| p.is_file()).or_else(|| src.path.clone())
        })
        .collect()
}

fn key_matches(cache: &Path, i: usize, key: &str) -> bool {
    std::fs::read_to_string(cache.join(format!("bg-{i}.key"))).map(|k| k == key).unwrap_or(false) && valid_file(&file_for(cache, i))
}

/// Файл фона цел: верная подпись и размер совпадает с заголовком (битый — пересоберём).
fn valid_file(file: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(file) else { return false };
    let mut head = [0u8; 12];
    if f.read_exact(&mut head).is_err() || &head[..4] != MAGIC {
        return false;
    }
    let bw = i32::from_le_bytes(head[4..8].try_into().unwrap());
    let bh = i32::from_le_bytes(head[8..12].try_into().unwrap());
    sane_size(&f, bw, bh)
}

/// Фон уже собран для всех мониторов?
pub fn is_fresh(cache: &Path, mons: &[RECT]) -> bool {
    let _com = crate::wallpaper::ComGuard::new();
    let src = current_source();
    let paths = wallpapers_for(mons, &src);
    mons.iter().enumerate().all(|(i, r)| key_matches(cache, i, &key_of(&paths[i], &src, r)))
}

// --- GDI+ через LoadLibrary (декодирование JPG/PNG/BMP силами Windows) ---

#[repr(C)]
struct StartupInput {
    version: u32,
    debug_cb: usize,
    suppress_bg_thread: i32,
    suppress_ext_codecs: i32,
}

type FnStartup = unsafe extern "system" fn(*mut usize, *const StartupInput, *mut c_void) -> i32;
type FnShutdown = unsafe extern "system" fn(usize);
type FnFromFile = unsafe extern "system" fn(*const u16, *mut *mut c_void) -> i32;
type FnGetDim = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
type FnFromHdc = unsafe extern "system" fn(HDC, *mut *mut c_void) -> i32;
type FnSetMode = unsafe extern "system" fn(*mut c_void, i32) -> i32;
type FnCreateAttr = unsafe extern "system" fn(*mut *mut c_void) -> i32;
type FnWrap = unsafe extern "system" fn(*mut c_void, i32, u32, i32) -> i32;
type FnDraw = unsafe extern "system" fn(*mut c_void, *mut c_void, i32, i32, i32, i32, i32, i32, i32, i32, i32, *mut c_void, *mut c_void, *mut c_void) -> i32;
type FnDelete = unsafe extern "system" fn(*mut c_void) -> i32;

macro_rules! proc {
    ($lib:expr, $name:literal, $t:ty) => {{
        match GetProcAddress($lib, concat!($name, "\0").as_ptr()) {
            Some(f) => std::mem::transmute::<_, $t>(f),
            None => return Err(concat!("нет функции ", $name).into()),
        }
    }};
}

/// Рисует картинку в DC размером w×h так же, как Windows раскладывает обои.
/// `span` — для режима «на все экраны»: (ширина и высота всего рабочего стола, сдвиг монитора от его угла).
unsafe fn draw_image(dc: HDC, path: &Path, w: i32, h: i32, style: u32, span: (i32, i32, i32, i32)) -> Result<(), String> {
    let lib = crate::util::load_system_dll("gdiplus.dll");
    if lib.is_null() {
        return Err("нет gdiplus.dll".into());
    }
    let startup = proc!(lib, "GdiplusStartup", FnStartup);
    let shutdown = proc!(lib, "GdiplusShutdown", FnShutdown);
    let from_file = proc!(lib, "GdipCreateBitmapFromFile", FnFromFile);
    let get_w = proc!(lib, "GdipGetImageWidth", FnGetDim);
    let get_h = proc!(lib, "GdipGetImageHeight", FnGetDim);
    let from_hdc = proc!(lib, "GdipCreateFromHDC", FnFromHdc);
    let set_interp = proc!(lib, "GdipSetInterpolationMode", FnSetMode);
    let set_offset = proc!(lib, "GdipSetPixelOffsetMode", FnSetMode);
    let create_attr = proc!(lib, "GdipCreateImageAttributes", FnCreateAttr);
    let set_wrap = proc!(lib, "GdipSetImageAttributesWrapMode", FnWrap);
    let draw = proc!(lib, "GdipDrawImageRectRectI", FnDraw);
    let del_g = proc!(lib, "GdipDeleteGraphics", FnDelete);
    let del_img = proc!(lib, "GdipDisposeImage", FnDelete);
    let del_attr = proc!(lib, "GdipDisposeImageAttributes", FnDelete);

    // GDI+ запускается один раз на всю программу (см. util::gdiplus)
    let _ = (startup, shutdown, std::mem::size_of::<StartupInput>());
    if !crate::util::gdiplus() {
        return Err("GDI+ не запустился".into());
    }
    let mut result = Err("не удалось открыть картинку".to_string());
    let mut img: *mut c_void = std::ptr::null_mut();
    if from_file(wide(&path.display().to_string()).as_ptr(), &mut img) == 0 && !img.is_null() {
        let (mut iw, mut ih) = (0u32, 0u32);
        get_w(img, &mut iw);
        get_h(img, &mut ih);
        let mut g: *mut c_void = std::ptr::null_mut();
        if iw > 0 && ih > 0 && from_hdc(dc, &mut g) == 0 {
            set_interp(g, 7); // HighQualityBicubic
            set_offset(g, 2); // HighQuality
            let mut attr: *mut c_void = std::ptr::null_mut();
            create_attr(&mut attr);
            if !attr.is_null() {
                set_wrap(attr, 3, 0, 0); // TileFlipXY — без светлой каймы по краям
            }
            let (fw, fh) = (iw as f32, ih as f32);
            let (sw, sh) = (w as f32, h as f32);
            let mut rects: Vec<(i32, i32, i32, i32)> = Vec::new();
            match style {
                2 => rects.push((0, 0, w, h)), // растянуть
                6 => {
                    let s = (sw / fw).min(sh / fh);
                    let (dw, dh) = ((fw * s).round() as i32, (fh * s).round() as i32);
                    rects.push(((w - dw) / 2, (h - dh) / 2, dw, dh));
                }
                0 => rects.push(((w - iw as i32) / 2, (h - ih as i32) / 2, iw as i32, ih as i32)),
                1 if (w as i64 / iw.max(1) as i64 + 1) * (h as i64 / ih.max(1) as i64 + 1) > 4096 => {
                    // слишком мелкая картинка для плитки — просто растягиваем
                    rects.push((0, 0, w, h));
                }
                1 => {
                    let mut y = 0;
                    while y < h {
                        let mut x = 0;
                        while x < w {
                            rects.push((x, y, iw as i32, ih as i32));
                            x += iw as i32;
                        }
                        y += ih as i32;
                    }
                }
                22 => {
                    // «на все экраны»: картинка заполняет весь рабочий стол, монитору достаётся его кусок
                    let (vw, vh, ox, oy) = span;
                    let s = (vw as f32 / fw).max(vh as f32 / fh);
                    let (dw, dh) = ((fw * s).round() as i32, (fh * s).round() as i32);
                    rects.push(((vw - dw) / 2 - ox, (vh - dh) / 2 - oy, dw, dh));
                }
                _ => {
                    // заполнить
                    let s = (sw / fw).max(sh / fh);
                    let (dw, dh) = ((fw * s).round() as i32, (fh * s).round() as i32);
                    rects.push(((w - dw) / 2, (h - dh) / 2, dw, dh));
                }
            }
            let mut ok = true;
            for (x, y, dw, dh) in rects {
                ok &= draw(g, img, x, y, dw, dh, 0, 0, iw as i32, ih as i32, 2, attr, std::ptr::null_mut(), std::ptr::null_mut()) == 0;
            }
            if !attr.is_null() {
                del_attr(attr);
            }
            del_g(g);
            result = if ok { Ok(()) } else { Err("GDI+ не смог нарисовать картинку".into()) };
        }
        del_img(img);
    }
    windows_sys::Win32::Foundation::FreeLibrary(lib);
    result
}

/// Собирает фон под текущие обои для каждого монитора (только там, где он устарел). Выполняется в фоновом потоке.
/// Возвращает битовую маску мониторов, у которых фон поменялся.
pub fn build(cache: &Path, mons: &[RECT]) -> Result<usize, String> {
    let _com = crate::wallpaper::ComGuard::new();
    let src = current_source();
    let paths = wallpapers_for(mons, &src);
    // общий прямоугольник всех мониторов — для режима «на все экраны»
    let vs = mons.iter().fold(mons.first().copied().unwrap_or(RECT { left: 0, top: 0, right: 1, bottom: 1 }), |a, r| RECT {
        left: a.left.min(r.left),
        top: a.top.min(r.top),
        right: a.right.max(r.right),
        bottom: a.bottom.max(r.bottom),
    });
    let mut changed = 0usize;
    for (i, r) in mons.iter().enumerate() {
        let key = key_of(&paths[i], &src, r);
        if key_matches(cache, i, &key) {
            continue;
        }
        let span = (vs.right - vs.left, vs.bottom - vs.top, r.left - vs.left, r.top - vs.top);
        match build_one(cache, i, r.right - r.left, r.bottom - r.top, &paths[i], &src, span, &key) {
            Ok(()) => changed |= 1 << i.min(31),
            Err(e) => log(&format!("фон монитора {} не собран: {e}", i + 1)),
        }
    }
    // старые файлы от мониторов, которых больше нет
    for i in mons.len()..8 {
        let _ = std::fs::remove_file(file_for(cache, i));
        let _ = std::fs::remove_file(prev_for(cache, i));
        let _ = std::fs::remove_file(cache.join(format!("bg-{i}.key")));
    }
    let _ = std::fs::remove_file(cache.join("bg.raw"));
    let _ = std::fs::remove_file(cache.join("bg.key"));
    Ok(changed)
}

fn build_one(cache: &Path, i: usize, w: i32, h: i32, path: &Option<PathBuf>, src: &Source, span: (i32, i32, i32, i32), key: &str) -> Result<(), String> {
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() {
            DeleteDC(dc);
            return Err("не хватило памяти под фон".into());
        }
        let old = SelectObject(dc, bmp as _);
        // сначала заливаем цветом фона Windows
        let brush = CreateSolidBrush(src.color);
        FillRect(dc, &windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: w, bottom: h }, brush);
        DeleteObject(brush as _);
        let drawn = match path {
            Some(p) => draw_image(dc, p, w, h, src.style, span),
            None => Ok(()),
        };
        GdiFlush();
        let res = match drawn {
            Ok(()) => {
                let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                let tmp = cache.join(format!("bg-{i}.raw.part"));
                let write = (|| -> std::io::Result<()> {
                    let mut f = std::fs::File::create(&tmp)?;
                    f.write_all(MAGIC)?;
                    f.write_all(&w.to_le_bytes())?;
                    f.write_all(&h.to_le_bytes())?;
                    f.write_all(px)?;
                    drop(f);
                    // старый фон не удаляем, а откладываем — с него пойдёт анимация перехода
                    let cur = file_for(cache, i);
                    if cur.is_file() {
                        let _ = std::fs::rename(&cur, prev_for(cache, i));
                    }
                    std::fs::rename(&tmp, cur)?;
                    std::fs::write(cache.join(format!("bg-{i}.key")), key)
                })();
                write.map_err(|e| e.to_string())
            }
            Err(e) => Err(e),
        };
        SelectObject(dc, old);
        DeleteObject(bmp as _);
        DeleteDC(dc);
        if let Err(e) = &res {
            log(&format!("фон не собран: {e}"));
        }
        res
    }
}

/// Размеры из заголовка правдоподобны и файл действительно такого размера (не битый).
fn sane_size(f: &std::fs::File, bw: i32, bh: i32) -> bool {
    (1..=32768).contains(&bw)
        && (1..=32768).contains(&bh)
        && f.metadata().map(|m| m.len() >= HEADER + bw as u64 * bh as u64 * 4).unwrap_or(false)
}

/// Читает из файла фона прямоугольник (x, y, w, h) в буфер BGRA (строки сверху вниз).
pub fn read_rect(file: &Path, x: i32, y: i32, w: i32, h: i32, out: &mut [u32]) -> bool {
    let Ok(mut f) = std::fs::File::open(file) else { return false };
    let mut head = [0u8; 12];
    if f.read_exact(&mut head).is_err() || &head[..4] != MAGIC {
        return false;
    }
    let bw = i32::from_le_bytes(head[4..8].try_into().unwrap());
    let bh = i32::from_le_bytes(head[8..12].try_into().unwrap());
    if !sane_size(&f, bw, bh) || w <= 0 || h <= 0 || out.len() < (w * h) as usize {
        return false;
    }
    // что вылезло за край экрана, заполняем крайними точками (для размытия и оценки яркости это честнее нулей)
    let cx0 = x.max(0);
    let cx1 = (x as i64 + w as i64).min(bw as i64) as i32;
    if cx0 >= cx1 {
        out[..(w * h) as usize].fill(0); // прямоугольник целиком за краем
        return true;
    }
    let n = (cx1 - cx0) as usize;
    let off = (cx0 - x) as usize;
    let mut row = vec![0u8; n * 4];
    for r in 0..h {
        let sy = (y as i64 + r as i64).clamp(0, bh as i64 - 1) as u64;
        let dst = &mut out[(r * w) as usize..((r + 1) * w) as usize];
        let pos = HEADER + (sy * bw as u64 + cx0 as u64) * 4;
        if f.seek(SeekFrom::Start(pos)).is_err() || f.read_exact(&mut row).is_err() {
            return false;
        }
        for i in 0..n {
            dst[off + i] = u32::from_le_bytes([row[i * 4], row[i * 4 + 1], row[i * 4 + 2], 0]);
        }
        let (first, last) = (dst[off], dst[off + n - 1]);
        dst[..off].fill(first);
        dst[off + n..].fill(last);
    }
    true
}

/// Средняя яркость (0..255) фона в прямоугольнике — берём каждый 4-й пиксель, этого достаточно.
/// Сколько под текстом светлых и тёмных точек (доли от 0 до 1) — по каждой 4-й точке.
pub fn luma_split(file: &Path, x: i32, y: i32, w: i32, h: i32) -> Option<(f32, f32)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut buf = vec![0u32; (w * h) as usize];
    if !read_rect(file, x, y, w, h, &mut buf) {
        return None;
    }
    let (mut light, mut dark, mut n) = (0u32, 0u32, 0u32);
    for row in buf.chunks(w as usize).step_by(4) {
        for p in row.iter().step_by(4) {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            let l = (r * 299 + g * 587 + b * 114) / 1000;
            if l > 165 {
                light += 1;
            } else if l < 90 {
                dark += 1;
            }
            n += 1;
        }
    }
    if n == 0 { None } else { Some((light as f32 / n as f32, dark as f32 / n as f32)) }
}

#[allow(dead_code)]
pub fn avg_luma(file: &Path, x: i32, y: i32, w: i32, h: i32) -> Option<u32> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut buf = vec![0u32; (w * h) as usize];
    if !read_rect(file, x, y, w, h, &mut buf) {
        return None;
    }
    let (mut sum, mut n) = (0u64, 0u64);
    for row in buf.chunks(w as usize).step_by(4) {
        for p in row.iter().step_by(4) {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            sum += ((r * 299 + g * 587 + b * 114) / 1000) as u64;
            n += 1;
        }
    }
    if n == 0 { None } else { Some((sum / n) as u32) }
}

/// Уменьшенная копия фона монитора (для окна настроек): берём по 2×2 точки на пиксель.
pub fn thumbnail(file: &Path, tw: i32, th: i32) -> Option<Vec<u32>> {
    if tw <= 0 || th <= 0 {
        return None;
    }
    let mut f = std::fs::File::open(file).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != MAGIC {
        return None;
    }
    let bw = i32::from_le_bytes(head[4..8].try_into().unwrap());
    let bh = i32::from_le_bytes(head[8..12].try_into().unwrap());
    if !sane_size(&f, bw, bh) || tw > 16384 || th > 16384 {
        return None;
    }
    let mut out = vec![0u32; (tw * th) as usize];
    let mut rows = [vec![0u8; (bw * 4) as usize], vec![0u8; (bw * 4) as usize]];
    for y in 0..th {
        for (k, row) in rows.iter_mut().enumerate() {
            let sy = (((y as f32 + 0.25 + 0.5 * k as f32) * bh as f32 / th as f32) as i32).clamp(0, bh - 1);
            f.seek(SeekFrom::Start(HEADER + sy as u64 * bw as u64 * 4)).ok()?;
            f.read_exact(row).ok()?;
        }
        for x in 0..tw {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for row in &rows {
                for dx in [0.25f32, 0.75] {
                    let sx = (((x as f32 + dx) * bw as f32 / tw as f32) as i32).clamp(0, bw - 1) as usize * 4;
                    b += row[sx] as u32;
                    g += row[sx + 1] as u32;
                    r += row[sx + 2] as u32;
                }
            }
            out[(y * tw + x) as usize] = 0xFF00_0000 | ((r / 4) << 16) | ((g / 4) << 8) | (b / 4);
        }
    }
    Some(out)
}
