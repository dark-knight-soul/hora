// Мелкие помощники: строки для WinAPI, пути, случайные числа, лог, сброс памяти.
use std::ffi::OsStr;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Строка → UTF-16 с нулём на конце (формат строк WinAPI).
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Копирует строку в буфер фиксированной длины (для структур вроде NOTIFYICONDATA).
pub fn copy_to_buf(dst: &mut [u16], s: &str) {
    let mut i = 0;
    for c in s.encode_utf16() {
        if i + 1 >= dst.len() {
            break;
        }
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}

/// Папка с данными программы: настройки, кэш, обои, избранное, журнал.
/// Переносная версия (рядом с exe лежит portable.txt) хранит всё рядом с собой;
/// установленная — в %LOCALAPPDATA%\Hora (своя у каждого пользователя, писать в Program Files нельзя).
pub fn app_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        if exe_dir.join("portable.txt").is_file() {
            return exe_dir;
        }
        match std::env::var_os("LOCALAPPDATA") {
            Some(base) => {
                let d = PathBuf::from(base).join("Hora");
                let _ = std::fs::create_dir_all(&d);
                d
            }
            None => exe_dir,
        }
    })
    .clone()
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Простой и быстрый генератор случайных чисел (xorshift64*), без внешних библиотек.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let addr = &t as *const u64 as u64;
        Rng((t ^ addr.rotate_left(17)) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Пишет строку в wallclock.log рядом с exe (лог не растёт больше ~200 КБ).
// Результаты фоновых потоков приходят окну сообщением с указателем в lParam. Сообщение в наше окно
// может послать любой процесс того же пользователя — поэтому указатель принимаем, только если сами
// его выдали (и того же типа), иначе мусорный lParam испортил бы память.
static PARCELS: std::sync::Mutex<Vec<(usize, std::any::TypeId)>> = std::sync::Mutex::new(Vec::new());

/// Упаковать значение для отправки окну (номер для lParam).
pub fn parcel_new<T: Send + 'static>(v: T) -> isize {
    let p = Box::into_raw(Box::new(v)) as usize;
    PARCELS.lock().unwrap_or_else(|e| e.into_inner()).push((p, std::any::TypeId::of::<T>()));
    p as isize
}

/// Забрать посылку по lParam — только если это наша посылка нужного типа.
pub fn parcel_take<T: 'static>(lp: isize) -> Option<T> {
    let mut g = PARCELS.lock().unwrap_or_else(|e| e.into_inner());
    let i = g.iter().position(|(p, t)| *p == lp as usize && *t == std::any::TypeId::of::<T>())?;
    g.swap_remove(i);
    drop(g);
    // SAFETY: указатель выдан parcel_new::<T> и ещё не забран
    Some(*unsafe { Box::from_raw(lp as *mut T) })
}

static LOG_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Включить/выключить журнал (процесс с правами администратора не пишет в профиль пользователя).
pub fn set_log(on: bool) {
    LOG_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Загрузить системную библиотеку только из System32 (не из папки рядом с exe — защита от подмены).
pub fn load_system_dll(name: &str) -> windows_sys::Win32::Foundation::HMODULE {
    use windows_sys::Win32::System::LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    unsafe { LoadLibraryExW(wide(name).as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) }
}

pub fn log(msg: &str) {
    // HORA_DEBUG_LOG=<файл> — для проверок: писать и туда, даже там, где журнал обычно молчит
    // (помощник обновления, процесс с правами администратора)
    if let Some(p) = std::env::var_os("HORA_DEBUG_LOG") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            use std::io::Write;
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100_000_000).unwrap_or(0);
            let _ = writeln!(f, "{t:>9} [{}] {msg}", std::process::id());
        }
    }
    if !LOG_ON.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    // пишут несколько потоков (часы, обои, живые обои) — по очереди, иначе строки перемешиваются
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = app_dir().join("wallclock.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 200 * 1024 {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let mut st = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
        // одной записью — строка целиком
        let line = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}  {}
", st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, msg);
        let _ = f.write_all(line.as_bytes());
    }
}

/// Отдаёт системе неиспользуемые страницы памяти процесса.
/// Это то, что видно в Диспетчере задач как «память» — после сброса там остаётся минимум.
pub fn trim_memory() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    unsafe {
        // освобождённые куски кучи возвращаем системе, потом сбрасываем рабочий набор
        windows_sys::Win32::System::Memory::HeapCompact(windows_sys::Win32::System::Memory::GetProcessHeap(), 0);
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// Запускает GDI+ один раз на всё время работы программы и больше не выгружает:
/// повторные запуск/выгрузка GDI+ из разных потоков приводили к зависанию.
pub fn gdiplus() -> bool {
    use std::sync::OnceLock;
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| unsafe {
        use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
        let lib = load_system_dll("gdiplus.dll");
        if lib.is_null() {
            return false;
        }
        #[repr(C)]
        struct Input {
            version: u32,
            cb: usize,
            no_thread: i32,
            no_codecs: i32,
        }
        let Some(f) = GetProcAddress(lib, b"GdiplusStartup\0".as_ptr()) else { return false };
        let startup: unsafe extern "system" fn(*mut usize, *const Input, *mut std::ffi::c_void) -> i32 = std::mem::transmute(f);
        let mut token = 0usize;
        startup(&mut token, &Input { version: 1, cb: 0, no_thread: 0, no_codecs: 0 }, std::ptr::null_mut()) == 0
    })
}

/// Совпадают ли два прямоугольника (у RECT из Windows нет сравнения «==»).
pub fn same_rect(a: &windows_sys::Win32::Foundation::RECT, b: &windows_sys::Win32::Foundation::RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

pub fn is_image(p: &Path) -> bool {
    match p.extension().and_then(|e| e.to_str()) {
        Some(e) => matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "bmp"
        ),
        None => false,
    }
}
