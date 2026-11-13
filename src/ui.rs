// Окно настроек WallClock.
// Нарисовано вручную (GDI+ для сглаженных фигур, GDI для текста) — без браузерных движков,
// поэтому пока окно открыто, программа занимает всего на несколько мегабайт больше,
// а после закрытия всё освобождается. Каждое действие идёт через тот же код, что и меню в трее.
use crate::i18n::tr;
use crate::util::wide;
use crate::*;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_MOUSELEAVE_: u32 = 0x02A3;
const T_REFRESH: usize = 1;
const WM_FRAME: u32 = WM_APP + 22; // кадр анимации — приходит в такт обновлению монитора

// Поток кадров: ждёт обновления экрана (DwmFlush) и шлёт окну WM_FRAME. Живёт, только пока
// что-то движется, — в покое процессор не тратится.
static ANIM_RUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ANIM_THREAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FRAME_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static ANIM_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
const NAV: usize = 2000; // +номер страницы (выше всех номеров команд меню)

const PAGES: [&str; 8] = ["Обои", "Категории", "Часы", "Оформление", "Погода", "Мониторы", "Живые обои", "Прочее"];
// значки разделов и короткие подсказки под заголовком
const PAGE_ICONS: [Ic; 8] = [Ic::Image, Ic::Grid, Ic::Clock, Ic::Palette, Ic::Cloud, Ic::Monitor, Ic::Sparkle, Ic::Sliders];
const PAGE_HINTS: [&str; 8] = [
    "Что ставить на рабочий стол и как часто",
    "Темы картинок — можно отметить несколько",
    "Стиль и поведение часов на рабочем столе",
    "Цвет, размер, положение и шрифты",
    "Виджет погоды на рабочем столе",
    "Где показывать часы и как раскладывать обои",
    "Снег, дождь, звёзды поверх обоев или видео вместо картинки",
    "Автозапуск, тема окна и файлы программы",
];

/// Значки интерфейса (рисуются линиями, как в Windows 11).
#[derive(Clone, Copy, PartialEq)]
enum Ic {
    Image,
    Grid,
    Clock,
    Palette,
    Cloud,
    Monitor,
    Sliders,
    Next,
    Folder,
    Search,
    Refresh,
    File,
    List,
    Power,
    Heart,
    HeartFull,
    Ban,
    Sparkle,
}
const P_WEATHER: usize = 4;
const P_MONITORS: usize = 5;
const P_LIVE: usize = 6;
const CITY_SEARCH: usize = 1000; // кнопка «Найти»
const SLIDER_PLATE: usize = 1100; // ползунок прозрачности подложки
const CITY_PICK: usize = 1001; // +номер найденного города
const WM_SEARCH: u32 = WM_APP + 20; // Enter в поле поиска города
const WM_SEARCH_DONE: u32 = WM_APP + 21; // результаты поиска города

// ---------------- цвета (тёмная тема) ----------------
const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16) // COLORREF для GDI
}
const fn argb(r: u32, g: u32, b: u32) -> u32 {
    0xFF00_0000 | (r << 16) | (g << 8) | b // цвет для GDI+
}
// ---------------- темы: тёмная и светлая ----------------
thread_local! {
    // true — сейчас рисуем светлую тему
    static LIGHT: Cell<bool> = const { Cell::new(false) };
}

/// Цвет для текущей темы: (тёмная, светлая).
fn pick(dark: u32, light: u32) -> u32 {
    if LIGHT.with(|l| l.get()) { light } else { dark }
}

/// Цвет текста GDI (0x00BBGGRR) → цвет GDI+ (0xAARRGGBB).
fn argb_of(c: u32) -> u32 {
    0xFF00_0000 | ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
}

// Тёмная тема — «чернильная»: мягкий тёмно-синий вместо чёрного (как штрихи гор в логотипе),
// приглушённый текст и закатно-оранжевый акцент (как солнце в логотипе). Светлая — прежняя.
fn bg() -> u32 { pick(argb(26, 30, 41), argb(243, 243, 246)) }
fn sidebar() -> u32 { pick(argb(20, 23, 32), argb(232, 233, 238)) }
fn card() -> u32 { pick(argb(34, 39, 54), argb(255, 255, 255)) }
fn card_hover() -> u32 { pick(argb(42, 48, 66), argb(240, 242, 248)) }
fn border() -> u32 { pick(argb(52, 60, 82), argb(216, 218, 226)) }
fn accent() -> u32 { pick(argb(232, 118, 58), argb(48, 98, 226)) }
fn accent_hover() -> u32 { pick(argb(242, 140, 84), argb(72, 118, 238)) }
fn danger() -> u32 { pick(argb(222, 88, 96), argb(206, 58, 58)) }
fn danger_hover() -> u32 { pick(argb(238, 108, 116), argb(226, 76, 76)) }
fn t_main() -> u32 { pick(rgb(233, 234, 240), rgb(26, 26, 32)) }
fn t_sec() -> u32 { pick(rgb(152, 160, 182), rgb(98, 98, 110)) }
fn t_on_accent() -> u32 { rgb(255, 255, 255) }
fn t_error() -> u32 { pick(rgb(246, 132, 128), rgb(196, 46, 46)) }
fn switch_off() -> u32 { pick(argb(116, 125, 150), argb(128, 128, 140)) } // рамка выключенного переключателя
fn knob_off() -> u32 { pick(argb(158, 165, 188), argb(110, 110, 122)) }
fn swatch_ring() -> u32 { pick(argb(78, 87, 112), argb(190, 190, 200)) }
fn hover_ring() -> u32 { pick(argb(124, 133, 160), argb(150, 150, 162)) }
fn grid_cell() -> u32 { pick(argb(42, 48, 66), argb(236, 237, 242)) }
fn grid_dot() -> u32 { pick(argb(124, 133, 160), argb(150, 150, 162)) }
fn mon_frame() -> u32 { pick(argb(96, 105, 132), argb(170, 172, 182)) }
fn scrollbar() -> u32 { pick(argb(62, 71, 96), argb(196, 198, 206)) }
fn nav_active() -> u32 { pick(argb(38, 44, 62), argb(255, 255, 255)) }
fn nav_hover() -> u32 { pick(argb(29, 34, 47), argb(244, 245, 249)) }

thread_local! {
    static LIGHT_CACHE: Cell<Option<(std::time::Instant, bool)>> = const { Cell::new(None) };
}

/// Тема по настройке: auto — как у приложений Windows (светлый/тёмный режим).
pub fn is_light(setting: &str) -> bool {
    match setting {
        "light" => true,
        "dark" => false,
        _ => LIGHT_CACHE.with(|c| match c.get() {
            Some((t, v)) if t.elapsed().as_millis() < 1000 => v,
            _ => {
                let v = system_light();
                c.set(Some((std::time::Instant::now(), v)));
                v
            }
        }),
    }
}

/// Светлая ли тема приложений в Windows (из реестра).
fn system_light() -> bool {
    unsafe {
        use windows_sys::Win32::System::Registry::*;
        let mut v: u32 = 0;
        let mut size: u32 = 4;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut v as *mut u32 as *mut c_void,
            &mut size,
        );
        r == 0 && v == 1
    }
}

/// Перекрашивает заголовок окна под тему.
unsafe fn apply_titlebar(hwnd: HWND) {
    let setting = with_app(|a| a.cfg.ui_theme.clone()).unwrap_or_default();
    let dark: i32 = if is_light(&setting) { 0 } else { 1 };
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, 20, &dark as *const i32 as *const c_void, 4);
    // Windows 11: заголовок окна — в цвет бокового меню тёмной темы (без серой полосы сверху);
    // у светлой — обычный системный (0xFFFFFFFF = «по умолчанию»)
    let (caption, text): (u32, u32) = if dark == 1 { (rgb(20, 23, 32), rgb(233, 234, 240)) } else { (0xFFFF_FFFF, 0xFFFF_FFFF) };
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, 35, &caption as *const u32 as *const c_void, 4);
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, 36, &text as *const u32 as *const c_void, 4);
    // заставляем Windows перерисовать рамку окна
    SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
}

// ---------------- GDI+ (сглаженные фигуры и картинки), загружается только пока окно открыто ----------------
type G = *mut c_void;
struct Gp {
    lib: windows_sys::Win32::Foundation::HMODULE,
    token: usize,
    shutdown: unsafe extern "system" fn(usize),
    from_hdc: unsafe extern "system" fn(HDC, *mut G) -> i32,
    del_graphics: unsafe extern "system" fn(G) -> i32,
    smoothing: unsafe extern "system" fn(G, i32) -> i32,
    interp: unsafe extern "system" fn(G, i32) -> i32,
    solid: unsafe extern "system" fn(u32, *mut G) -> i32,
    del_brush: unsafe extern "system" fn(G) -> i32,
    pen: unsafe extern "system" fn(u32, f32, i32, *mut G) -> i32,
    del_pen: unsafe extern "system" fn(G) -> i32,
    path: unsafe extern "system" fn(i32, *mut G) -> i32,
    del_path: unsafe extern "system" fn(G) -> i32,
    arc: unsafe extern "system" fn(G, f32, f32, f32, f32, f32, f32) -> i32,
    close: unsafe extern "system" fn(G) -> i32,
    ellipse: unsafe extern "system" fn(G, f32, f32, f32, f32) -> i32,
    fill_path: unsafe extern "system" fn(G, G, G) -> i32,
    draw_path: unsafe extern "system" fn(G, G, G) -> i32,
    bitmap: unsafe extern "system" fn(i32, i32, i32, i32, *mut u8, *mut G) -> i32,
    del_image: unsafe extern "system" fn(G) -> i32,
    draw_image: unsafe extern "system" fn(G, G, f32, f32, f32, f32) -> i32,
    clip_path: unsafe extern "system" fn(G, G, i32) -> i32,
    reset_clip: unsafe extern "system" fn(G) -> i32,
    add_line: unsafe extern "system" fn(G, f32, f32, f32, f32) -> i32,
    start_figure: unsafe extern "system" fn(G) -> i32,
    pen_start_cap: unsafe extern "system" fn(G, i32) -> i32,
    pen_end_cap: unsafe extern "system" fn(G, i32) -> i32,
    pen_join: unsafe extern "system" fn(G, i32) -> i32,
}

impl Gp {
    fn load() -> Option<Gp> {
        unsafe {
            let lib = util::load_system_dll("gdiplus.dll");
            if lib.is_null() {
                return None;
            }
            macro_rules! f {
                ($n:literal) => {
                    std::mem::transmute(GetProcAddress(lib, concat!($n, "\0").as_ptr())?)
                };
            }
            #[repr(C)]
            struct Input {
                version: u32,
                cb: usize,
                no_thread: i32,
                no_codecs: i32,
            }
            let _ = std::mem::size_of::<Input>();
            // GDI+ запускается один раз на всю программу (см. util::gdiplus)
            if !util::gdiplus() {
                return None;
            }
            let token = 0usize;
            Some(Gp {
                lib,
                token,
                shutdown: f!("GdiplusShutdown"),
                from_hdc: f!("GdipCreateFromHDC"),
                del_graphics: f!("GdipDeleteGraphics"),
                smoothing: f!("GdipSetSmoothingMode"),
                interp: f!("GdipSetInterpolationMode"),
                solid: f!("GdipCreateSolidFill"),
                del_brush: f!("GdipDeleteBrush"),
                pen: f!("GdipCreatePen1"),
                del_pen: f!("GdipDeletePen"),
                path: f!("GdipCreatePath"),
                del_path: f!("GdipDeletePath"),
                arc: f!("GdipAddPathArc"),
                close: f!("GdipClosePathFigure"),
                ellipse: f!("GdipAddPathEllipse"),
                fill_path: f!("GdipFillPath"),
                draw_path: f!("GdipDrawPath"),
                bitmap: f!("GdipCreateBitmapFromScan0"),
                del_image: f!("GdipDisposeImage"),
                draw_image: f!("GdipDrawImageRect"),
                clip_path: f!("GdipSetClipPath"),
                reset_clip: f!("GdipResetClip"),
                add_line: f!("GdipAddPathLine"),
                start_figure: f!("GdipStartPathFigure"),
                pen_start_cap: f!("GdipSetPenStartCap"),
                pen_end_cap: f!("GdipSetPenEndCap"),
                pen_join: f!("GdipSetPenLineJoin"),
            })
        }
    }
}

impl Drop for Gp {
    fn drop(&mut self) {
        // GDI+ не выгружаем: он общий для всей программы; только уменьшаем счётчик библиотеки
        let _ = (self.shutdown, self.token);
        unsafe { windows_sys::Win32::Foundation::FreeLibrary(self.lib) };
    }
}

// ---------------- шрифты интерфейса ----------------
#[derive(Clone, Copy)]
enum F {
    Title,
    H2,
    Body,
    Small,
    Logo,
}

struct Fonts {
    h: [HFONT; 5],
}

impl Fonts {
    fn new(s: f32) -> Fonts {
        let mk = |px: f32, face: &str| unsafe {
            CreateFontW(-(px * s).round() as i32, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET as _, OUT_TT_PRECIS as _, CLIP_DEFAULT_PRECIS as _, CLEARTYPE_QUALITY as _, (DEFAULT_PITCH | FF_DONTCARE) as _, wide(face).as_ptr())
        };
        Fonts { h: [mk(26.0, "Segoe UI Semibold"), mk(15.0, "Segoe UI Semibold"), mk(14.0, "Segoe UI"), mk(12.5, "Segoe UI"), mk(20.0, "Segoe UI Semibold")] }
    }
    fn get(&self, f: F) -> HFONT {
        self.h[f as usize]
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for f in self.h {
            unsafe { DeleteObject(f as _) };
        }
    }
}

// ---------------- картинки (миниатюры обоев, предпросмотр часов) ----------------
struct Img {
    w: i32,
    h: i32,
    px: Vec<u32>,
}

// ---------------- состояние окна ----------------
struct Snap {
    videos: Vec<(String, bool, bool)>, // найденные видео: (название, выбрано, из Wallpaper Engine)
    videos_total: usize,               // сколько всего найдено
    live_running: bool,                // живые обои сейчас работают
    fav_count: usize,  // картинок в избранном
    ban_count: usize,  // в чёрном списке
    cur_fav: bool,     // текущие обои в избранном
    can_undo: bool,    // есть что «вернуть» из последних скрытых
    weather: Option<weather::Weather>,
    weather_status: String,
    weather_busy: bool,
    cfg: Config,
    status: String,
    current: Vec<PathBuf>,
    last_change: u64,
    cache: PathBuf,
    autostart: bool,
}

struct State {
    hwnd: HWND,
    page: usize,
    scroll: f32,
    content_h: f32,
    hover: usize,
    hits: Vec<(RECT, usize)>,
    s: f32,
    fonts: Fonts,
    gp: Gp,
    buf: Option<(HDC, HBITMAP, HGDIOBJ, i32, i32)>,
    thumbs: Vec<(usize, i32, i32, Img)>,
    preview: Option<(Config, u16, i32, i32, String, Img)>,
    edit: HWND,                // поле ввода города (обычное поле Windows)
    edit_rect: Option<RECT>,   // где его показать в этом кадре (None — спрятать)
    search_busy: bool,
    search_status: String,
    results: Vec<weather::City>,
    clock_fonts: clock::FontStore,
    tracking: bool,
    // плавность: прокрутка едет к цели, подсветка плавно появляется/гаснет, раздел «въезжает»
    scroll_target: f32,
    hover_prev: usize,
    hover_t: f32,
    prev_t: f32,
    page_t: f32,
    switch_anim: Option<(usize, f32, bool)>, // переключатель: (id, 0..1, во что переключаемся)
    anim_on: bool,
    anim_last: std::time::Instant,
    frame_prev: Option<std::time::Instant>,
    gaps: (u32, u64, u32), // кадров анимации, сумма промежутков мкс, сколько дольше 25 мс
    slider_track: Option<(f32, f32)>, // дорожка ползунка в последнем кадре: (x начала, ширина)
    dragging: bool,                   // сейчас тянем ручку ползунка
}

thread_local! {
    // кисть фона поля ввода: (кисть, её цвет) — отдельно от UI, чтобы не было двойного заёма
    static EDIT_BRUSH: Cell<(HBRUSH, u32)> = const { Cell::new((std::ptr::null_mut(), u32::MAX)) };
    static UI: RefCell<Option<State>> = const { RefCell::new(None) };
    static UI_HWND: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
}

/// Текущий час по местному времени.
fn clock_hour() -> u32 {
    let mut st: clock::SysTime = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    st.wHour as u32
}

/// Открыто ли окно настроек.
pub fn is_open() -> bool {
    !UI_HWND.with(|c| c.get()).is_null()
}

/// Перерисовать окно настроек, если оно открыто (данные программы поменялись).
pub fn refresh() {
    let h = UI_HWND.with(|c| c.get());
    if !h.is_null() {
        unsafe { InvalidateRect(h, std::ptr::null(), 0) };
    }
}

/// Обои/фон поменялись — миниатюры надо построить заново.
pub fn bg_changed() {
    UI.with(|u| {
        if let Ok(mut u) = u.try_borrow_mut() {
            if let Some(st) = u.as_mut() {
                st.thumbs.clear();
                st.preview = None;
            }
        }
    });
    refresh();
}

pub fn close() {
    let h = UI_HWND.with(|c| c.get());
    if !h.is_null() {
        unsafe { DestroyWindow(h) };
    }
}

/// Открывает окно настроек (или выводит на передний план, если уже открыто).
pub fn open() {
    with_app(|a| {
        a.refresh_lists();
        a.refresh_videos(false);
    });
    unsafe {
        let existing = UI_HWND.with(|c| c.get());
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = wide("WallClockSettings");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(proc_),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: LoadIconW(hinst, 1 as _),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: LoadIconW(hinst, 1 as _),
        };
        RegisterClassExW(&wc);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let hwnd = CreateWindowExW(0, cls.as_ptr(), wide("Hora").as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT, 960, 660, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        if hwnd.is_null() {
            return;
        }
        // заголовок окна — под тему; углы окна — скруглённые
        apply_titlebar(hwnd);
        let round: i32 = 2; // DWMWCP_ROUND
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, 33, &round as *const i32 as *const c_void, 4);
        let Some(gp) = Gp::load() else {
            DestroyWindow(hwnd);
            return;
        };
        let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);
        let s = dpi as f32 / 96.0;
        UI.with(|u| {
            *u.borrow_mut() = Some(State {
                hwnd,
                page: 0,
                scroll: 0.0,
                content_h: 0.0,
                hover: 0,
                hits: Vec::new(),
                s,
                fonts: Fonts::new(s),
                gp,
                buf: None,
                thumbs: Vec::new(),
                preview: None,
                clock_fonts: clock::FontStore::default(),
                tracking: false,
                scroll_target: 0.0,
                hover_prev: 0,
                hover_t: 1.0,
                prev_t: 0.0,
                page_t: 0.0,
                switch_anim: None,
                anim_on: false,
                anim_last: std::time::Instant::now(),
                frame_prev: None,
                gaps: (0, 0, 0),
                slider_track: None,
                dragging: false,
                edit: std::ptr::null_mut(),
                edit_rect: None,
                search_busy: false,
                search_status: String::new(),
                results: Vec::new(),
            })
        });
        UI_HWND.with(|c| c.set(hwnd));
        create_edit(hwnd);
        resize_for_dpi(hwnd, dpi);
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, T_REFRESH, 2000, None);
    }
}

// Старая оконная процедура поля ввода (мы перехватываем Enter, остальное отдаём ей).
static EDIT_OLD_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CHAR && (wp == 13 || wp == 10) {
        PostMessageW(GetParent(hwnd), WM_SEARCH, 0, 0);
        return 0; // без системного «бип»
    }
    let old: WNDPROC = std::mem::transmute(EDIT_OLD_PROC.load(std::sync::atomic::Ordering::Relaxed));
    CallWindowProcW(old, hwnd, msg, wp, lp)
}

unsafe fn create_edit(parent: HWND) {
    let edit = CreateWindowExW(
        0,
        wide("EDIT").as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_TABSTOP | (ES_AUTOHSCROLL as u32),
        0,
        0,
        10,
        10,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null(),
    );
    if edit.is_null() {
        return;
    }
    let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as usize as isize);
    EDIT_OLD_PROC.store(old, std::sync::atomic::Ordering::Relaxed);
    SendMessageW(edit, 0x00C5, 80, 0); // EM_LIMITTEXT — не больше 80 символов
    let font = UI.with(|u| {
        u.borrow_mut().as_mut().map(|st| {
            st.edit = edit;
            st.fonts.get(F::Body)
        })
    });
    if let Some(font) = font {
        SendMessageW(edit, WM_SETFONT, font as usize, 1);
    }
}

/// Ищет город по тексту из поля ввода (в фоновом потоке).
unsafe fn start_search(hwnd: HWND) {
    let (edit, busy) = UI.with(|u| u.borrow().as_ref().map(|s| (s.edit, s.search_busy)).unwrap_or((std::ptr::null_mut(), true)));
    if edit.is_null() || busy {
        return;
    }
    let mut buf = [0u16; 100];
    let n = GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32);
    let query = String::from_utf16_lossy(&buf[..n.max(0) as usize]).trim().to_string();
    if query.is_empty() {
        return;
    }
    let lang = with_app(|a| a.cfg.lang.clone()).unwrap_or_default();
    UI.with(|u| {
        if let Some(st) = u.borrow_mut().as_mut() {
            st.search_busy = true;
            st.search_status = tr("Ищу…").into();
            st.results.clear();
        }
    });
    let h = hwnd as usize;
    let spawned = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        let r = util::parcel_new(weather::search(&query, &lang));
        if PostMessageW(h as HWND, WM_SEARCH_DONE, 0, r) == 0 {
            drop(util::parcel_take::<Result<Vec<weather::City>, String>>(r)); // окно настроек уже закрыто
        }
    });
    if spawned.is_err() {
        UI.with(|u| {
            if let Some(st) = u.borrow_mut().as_mut() {
                st.search_busy = false;
                st.search_status = tr("Не удалось начать поиск").into();
            }
        });
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

/// Выбран город из найденных: запоминаем его и сразу загружаем погоду.
fn pick_city(c: &weather::City) {
    let label = c.label();
    let (lat, lon) = (c.lat, c.lon);
    with_app(|a| {
        let old = a.cfg.clone();
        a.cfg.weather_city = label;
        a.cfg.weather_lat = lat;
        a.cfg.weather_lon = lon;
        a.cfg.weather = true;
        a.save_cfg();
        a.apply_cfg(&old);
    });
}

/// Куда поставить поле ввода (копия данных из состояния окна).
struct EditPos {
    edit: HWND,
    edit_rect: Option<RECT>,
}

/// Ставит размер окна под DPI и центрирует его на основном мониторе.
unsafe fn resize_for_dpi(hwnd: HWND, dpi: u32) {
    let s = dpi as f32 / 96.0;
    let mut r = RECT { left: 0, top: 0, right: (960.0 * s) as i32, bottom: (660.0 * s) as i32 };
    windows_sys::Win32::UI::HiDpi::AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 0, 0, dpi);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let (_, work) = clock::primary_monitor();
    let x = work.left + ((work.right - work.left) - w) / 2;
    let y = work.top + ((work.bottom - work.top) - h) / 2;
    SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
}

thread_local! {
    static AUTOSTART_CACHE: Cell<Option<(std::time::Instant, bool)>> = const { Cell::new(None) };
}
/// Автозапуск включён? Реестр читаем не чаще раза в секунду (кадры анимации идут 60 раз в секунду).
fn autostart_cached() -> bool {
    AUTOSTART_CACHE.with(|c| match c.get() {
        Some((t, v)) if t.elapsed().as_millis() < 1000 => v,
        _ => {
            let v = autostart_enabled();
            c.set(Some((std::time::Instant::now(), v)));
            v
        }
    })
}

fn hit_at(st: &State, x: i32, y: i32) -> usize {
    st.hits.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|h| h.1).unwrap_or(0)
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let lx = (lp & 0xFFFF) as i16 as i32;
    let ly = ((lp >> 16) & 0xFFFF) as i16 as i32;
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            // данные программы берём заранее, чтобы не держать два «замка» сразу
            let snap = with_app(|a| Snap {
                cfg: a.cfg.clone(),
                status: a.status.clone(),
                current: a.current.clone(),
                last_change: a.last_change,
                cache: a.cache.clone(),
                autostart: autostart_cached(),
                weather: a.weather.clone(),
                weather_status: a.weather_status.clone(),
                weather_busy: a.weather_busy,
                videos: a.videos.iter().take(VPICK_MAX).map(|v| (v.title.clone(), v.path.to_string_lossy() == a.cfg.video.trim(), v.we)).collect(),
                videos_total: a.videos.len(),
                live_running: a.live.running(),
                fav_count: a.fav_names.len(),
                ban_count: a.banned.len(),
                cur_fav: a.current_is_fav(),
                can_undo: !a.last_banned.is_empty(),
            });
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    paint(st, hdc, snap.as_ref());
                }
            });
            EndPaint(hwnd, &ps);
            // поле ввода города двигаем уже после рисования (и уже не держа UI)
            let edit_state = UI.with(|u| u.borrow().as_ref().map(|st| (st.edit, st.edit_rect)));
            if let Some((edit, edit_rect)) = edit_state {
                let st = EditPos { edit, edit_rect };
                {
                    if st.edit.is_null() {
                        return 0;
                    }
                    match st.edit_rect {
                        Some(r) => {
                            let mut cur = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                            GetWindowRect(st.edit, &mut cur);
                            let mut pt = POINT { x: cur.left, y: cur.top };
                            ScreenToClient(hwnd, &mut pt);
                            if pt.x != r.left || pt.y != r.top || cur.right - cur.left != r.right - r.left {
                                SetWindowPos(st.edit, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                            }
                            if IsWindowVisible(st.edit) == 0 {
                                ShowWindow(st.edit, SW_SHOWNOACTIVATE);
                            }
                        }
                        None => {
                            if IsWindowVisible(st.edit) != 0 {
                                ShowWindow(st.edit, SW_HIDE);
                            }
                        }
                    }
                }
            }
            if snap.is_none() {
                SetTimer(hwnd, T_REFRESH, 200, None);
            }
            0
        }
        WM_ERASEBKGND => 1,
        // цвета поля ввода — под тему окна
        WM_CTLCOLOREDIT => {
            // поле лежит на «таблетке» цвета фона окна
            let hdc = wp as HDC;
            let c = bg() & 0xFF_FFFF;
            let cref = ((c >> 16) & 0xFF) | (c & 0xFF00) | ((c & 0xFF) << 16);
            let brush = EDIT_BRUSH.with(|b| {
                let (old, col) = b.get();
                if old.is_null() || col != cref {
                    if !old.is_null() {
                        DeleteObject(old as _);
                    }
                    let nb = CreateSolidBrush(cref);
                    b.set((nb, cref));
                    nb
                } else {
                    old
                }
            });
            SetTextColor(hdc, t_main());
            SetBkColor(hdc, cref);
            brush as LRESULT
        }
        WM_SEARCH => {
            start_search(hwnd);
            0
        }
        WM_SEARCH_DONE => {
            let Some(r) = util::parcel_take::<Result<Vec<weather::City>, String>>(lp) else {
                return 0; // не наша посылка (поддельное сообщение)
            };
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    st.search_busy = false;
                    match r {
                        Ok(list) if list.is_empty() => {
                            st.search_status = tr("Ничего не нашлось — попробуйте написать иначе").into();
                            st.results.clear();
                        }
                        Ok(list) => {
                            st.search_status = tr("Выберите город:").into();
                            st.results = list;
                        }
                        Err(e) => {
                            st.search_status = if crate::i18n::en() { format!("Search failed: {e}") } else { format!("Не удалось найти: {e}") };
                            st.results.clear();
                        }
                    }
                }
            });
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        // Windows переключила светлый/тёмный режим — для темы «как в Windows» перекрашиваемся
        WM_SETTINGCHANGE => {
            LIGHT_CACHE.with(|c| c.set(None)); // системная тема могла смениться — перечитать
            if with_app(|_| ()).is_some() {
                apply_titlebar(hwnd);
            }
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_MOUSEMOVE if UI.with(|u| u.borrow().as_ref().map(|s| s.dragging).unwrap_or(false)) => {
            if wp & 0x0001 == 0 {
                // кнопку отпустили, а мы этого не увидели — заканчиваем перетаскивание
                end_drag();
                windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            } else {
                slider_to(hwnd, lx, false);
            }
            0
        }
        // захват мыши отобрали (Alt+Tab, чужое окно, блокировка) — ползунок «отпускаем» и сохраняем
        WM_CAPTURECHANGED => {
            end_drag();
            0
        }
        WM_MOUSEMOVE => {
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    if !st.tracking {
                        let mut t = windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>() as u32,
                            dwFlags: windows_sys::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut t);
                        st.tracking = true;
                    }
                    let h = hit_at(st, lx, ly);
                    set_hover(hwnd, st, h);
                }
            });
            0
        }
        WM_MOUSELEAVE_ => {
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    st.tracking = false;
                    set_hover(hwnd, st, 0);
                }
            });
            0
        }
        WM_SETCURSOR => {
            let over = UI.with(|u| u.borrow().as_ref().map(|s| s.hover != 0).unwrap_or(false));
            if (lp & 0xFFFF) as u32 == HTCLIENT {
                SetCursor(LoadCursorW(std::ptr::null_mut(), if over { IDC_HAND } else { IDC_ARROW }));
                return 1;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) & 0xFFFF) as i16 as f32 / 120.0;
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    GetClientRect(hwnd, &mut rc);
                    let max = (st.content_h - rc.bottom as f32).max(0.0);
                    // колесо задаёт цель, а прокрутка плавно к ней едет
                    st.scroll_target = (st.scroll_target - delta * 110.0 * st.s).clamp(0.0, max);
                    set_hover(hwnd, st, 0);
                    kick(hwnd, st);
                }
            });
            0
        }
        WM_LBUTTONDOWN => {
            let on_slider = UI.with(|u| {
                let mut u = u.borrow_mut();
                let Some(st) = u.as_mut() else { return false };
                if hit_at(st, lx, ly) == SLIDER_PLATE {
                    st.dragging = true;
                    true
                } else {
                    false
                }
            });
            if on_slider {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                slider_to(hwnd, lx, false);
            }
            0
        }
        WM_LBUTTONUP => {
            let was_drag = UI.with(|u| {
                let mut u = u.borrow_mut();
                match u.as_mut() {
                    Some(st) if st.dragging => {
                        st.dragging = false;
                        true
                    }
                    _ => false,
                }
            });
            if was_drag {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                slider_to(hwnd, lx, true);
                return 0;
            }
            let id = UI.with(|u| u.borrow().as_ref().map(|s| hit_at(s, lx, ly)).unwrap_or(0));
            click(hwnd, id);
            0
        }
        WM_FRAME => {
            let now = std::time::Instant::now();
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    if st.anim_on {
                        // замер ровности: промежутки между кадрами анимации
                        if let Some(prev) = st.frame_prev {
                            let gap = (now - prev).as_micros() as u64;
                            st.gaps.0 += 1;
                            st.gaps.1 += gap;
                            if gap > 25_000 {
                                st.gaps.2 += 1;
                            }
                        }
                        st.frame_prev = Some(now);
                        anim_step(hwnd, st);
                        if !st.anim_on {
                            st.frame_prev = None;
                        }
                    }
                }
            });
            // рисуем сразу, не дожидаясь очереди сообщений (WM_PAINT самый «ленивый»)
            UpdateWindow(hwnd);
            FRAME_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
            0
        }
        WM_TIMER => {
            if wp == T_REFRESH {
                SetTimer(hwnd, T_REFRESH, 2000, None);
                // раз в 2 с — обновить цифру памяти и время в предпросмотре
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_DPICHANGED if lp == 0 => 0, // настоящее сообщение всегда несёт прямоугольник окна
        WM_DPICHANGED => {
            let dpi = (wp & 0xFFFF) as u32;
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().as_mut() {
                    st.s = dpi as f32 / 96.0;
                    st.fonts = Fonts::new(st.s);
                    st.thumbs.clear();
                    st.preview = None;
                }
            });
            // новый шрифт полю ввода — уже не держа состояние окна
            if let Some((edit, font)) = UI.with(|u| u.borrow().as_ref().map(|st| (st.edit, st.fonts.get(F::Body)))) {
                if !edit.is_null() {
                    SendMessageW(edit, WM_SETFONT, font as usize, 1);
                }
            }
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, T_REFRESH);
            end_drag(); // закрыли посреди перетаскивания ползунка — значение не теряем
            ANIM_RUN.store(false, std::sync::atomic::Ordering::SeqCst);
            FRAME_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
            if let Some((n, sum, slow)) = UI.with(|u| u.borrow().as_ref().map(|s| s.gaps)) {
                if n > 0 {
                    util::log(&format!("анимация: {} кадров, промежуток в среднем {:.1} мс, рывков (>25 мс): {}", n, sum as f64 / n as f64 / 1000.0, slow));
                }
            }
            PAINT_STATS.with(|s| {
                let (n, sum, max) = s.get();
                if n > 0 {
                    util::log(&format!("окно настроек: {} кадров, в среднем {:.1} мс, худший {:.1} мс", n, sum as f64 / n as f64 / 1000.0, max as f64 / 1000.0));
                }
                s.set((0, 0, 0));
            });
            UI.with(|u| {
                if let Some(st) = u.borrow_mut().take() {
                    if let Some((dc, bmp, old, _, _)) = st.buf {
                        SelectObject(dc, old);
                        DeleteObject(bmp as _);
                        DeleteDC(dc);
                    }
                }
            });
            UI_HWND.with(|c| c.set(std::ptr::null_mut()));
            // память вернём системе чуть позже, когда окно полностью уйдёт
            match with_app(|a| a.hwnd) {
                Some(main) => {
                    SetTimer(main, T_TRIM, 1000, None);
                }
                None => util::log("окно настроек: не удалось запланировать очистку памяти"),
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Запустить кадры анимации (если ещё не идут).
unsafe fn kick(hwnd: HWND, st: &mut State) {
    use std::sync::atomic::Ordering::SeqCst;
    if !st.anim_on {
        st.anim_on = true;
        st.anim_last = std::time::Instant::now();
        // прошлый кадр мог пропасть вместе с закрытым окном — иначе поток больше не пошлёт ни одного
        FRAME_PENDING.store(false, SeqCst);
    }
    ANIM_HWND.store(hwnd as isize, SeqCst);
    ANIM_RUN.store(true, SeqCst);
    if !ANIM_THREAD.swap(true, SeqCst) {
        let ok = std::thread::Builder::new().stack_size(64 * 1024).spawn(frame_thread).is_ok();
        if !ok {
            // поток не создался — просто сразу показываем конечное состояние, без анимации
            ANIM_THREAD.store(false, SeqCst);
            ANIM_RUN.store(false, SeqCst);
            st.scroll = st.scroll_target;
            st.hover_t = 1.0;
            st.prev_t = 0.0;
            st.page_t = 1.0;
            st.switch_anim = None;
            st.anim_on = false;
        }
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

/// Период обновления основного монитора (60, 120, 144 Гц — сколько есть).
fn refresh_period() -> std::time::Duration {
    let hz = unsafe {
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut dm) != 0 { dm.dmDisplayFrequency } else { 60 }
    };
    // 0 и 1 — «по умолчанию у видеокарты»; странные значения тоже заменяем на 60
    let hz = if (30..=360).contains(&hz) { hz } else { 60 };
    std::time::Duration::from_nanos(1_000_000_000 / hz as u64)
}

/// Поток кадров: один кадр на одно обновление монитора.
fn frame_thread() {
    use std::sync::atomic::Ordering::SeqCst;
    use windows_sys::Win32::System::Threading::*;
    // ожидающий таймер высокой точности (Windows 10 1803+); если не создался — обычный sleep
    let timer = unsafe { CreateWaitableTimerExW(std::ptr::null(), std::ptr::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS) };
    let pause = |d: std::time::Duration| unsafe {
        if timer.is_null() {
            std::thread::sleep(d);
        } else {
            let due: i64 = -((d.as_nanos() / 100) as i64); // минус — «через столько», шаг 100 нс
            if SetWaitableTimer(timer, &due, 0, None, std::ptr::null(), 0) != 0 {
                WaitForSingleObject(timer, 100);
            } else {
                std::thread::sleep(d);
            }
        }
    };
    loop {
        let period = refresh_period();
        let mut next = std::time::Instant::now() + period;
        while ANIM_RUN.load(SeqCst) {
            // главный путь: ждём ближайшего обновления экрана — кадры идут ровно в такт монитору
            let t = std::time::Instant::now();
            let hr = unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() };
            if hr == 0 && t.elapsed() >= std::time::Duration::from_millis(2) {
                next = std::time::Instant::now() + period;
            } else {
                // DwmFlush не ждёт (экран заблокирован, удалённый сеанс) — точное расписание с частотой монитора
                let now = std::time::Instant::now();
                if next > now {
                    pause(next - now);
                } else if now - next > period {
                    next = now; // сильно отстали — не догоняем пачкой кадров
                }
                next += period;
            }
            // новый кадр шлём, только когда окно нарисовало прошлый, — очередь не копится
            if !FRAME_PENDING.swap(true, SeqCst) {
                let h = ANIM_HWND.load(SeqCst) as HWND;
                if unsafe { PostMessageW(h, WM_FRAME, 0, 0) } == 0 {
                    FRAME_PENDING.store(false, SeqCst);
                }
            }
        }
        ANIM_THREAD.store(false, SeqCst);
        // пока выходили, анимацию могли запустить снова — тогда продолжаем сами
        if ANIM_RUN.load(SeqCst) && !ANIM_THREAD.swap(true, SeqCst) {
            continue;
        }
        break;
    }
    if !timer.is_null() {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(timer) };
    }
}

/// Один шаг анимаций; когда всё доехало — таймер выключается (в покое процессор не тратится).
unsafe fn anim_step(hwnd: HWND, st: &mut State) {
    let now = std::time::Instant::now();
    let dt = (now - st.anim_last).as_secs_f32().min(0.05) * 1000.0;
    st.anim_last = now;
    // прокрутка: экспоненциально догоняем цель (мягкая остановка)
    let diff = st.scroll_target - st.scroll;
    if diff.abs() < 0.5 {
        st.scroll = st.scroll_target;
    } else {
        st.scroll += diff * (1.0 - (-dt / 75.0).exp());
    }
    st.hover_t = (st.hover_t + dt / 120.0).min(1.0);
    st.prev_t = (st.prev_t - dt / 180.0).max(0.0);
    st.page_t = (st.page_t + dt / 220.0).min(1.0);
    if let Some((id, t, to)) = st.switch_anim {
        let t = t + dt / 170.0;
        st.switch_anim = if t >= 1.0 { None } else { Some((id, t, to)) };
    }
    let done = st.scroll == st.scroll_target && st.hover_t >= 1.0 && st.prev_t <= 0.0 && st.page_t >= 1.0 && st.switch_anim.is_none();
    if done {
        ANIM_RUN.store(false, std::sync::atomic::Ordering::SeqCst);
        st.anim_on = false;
        // прокрутка остановилась — подсветить то, что теперь под курсором (мышь могла не двигаться)
        if st.hover == 0 && !st.dragging {
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) != 0 && WindowFromPoint(pt) == hwnd {
                ScreenToClient(hwnd, &mut pt);
                let h = hit_at(st, pt.x, pt.y);
                if h != 0 {
                    set_hover(hwnd, st, h);
                    return;
                }
            }
        }
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

/// Наведение сменилось: прежний элемент плавно гаснет, новый — загорается.
unsafe fn set_hover(hwnd: HWND, st: &mut State, h: usize) {
    if h == st.hover {
        return;
    }
    st.hover_prev = st.hover;
    st.prev_t = st.hover_t;
    st.hover = h;
    st.hover_t = 0.0;
    kick(hwnd, st);
}

/// Ползунок прозрачности подложки: значение по положению мыши. Пока тянем — меняется только
/// предпросмотр в окне; когда отпустили (commit) — сохраняем и перерисовываем часы на рабочем столе.
unsafe fn slider_to(hwnd: HWND, mx: i32, commit: bool) {
    let Some((tx, tw)) = UI.with(|u| u.borrow().as_ref().and_then(|s| s.slider_track)) else { return };
    let v = ((mx as f32 - tx) / tw.max(1.0)).clamp(0.0, 1.0);
    let transp = ((v * 90.0) / 5.0).round() as u32 * 5; // шаг 5%
    let changed = with_app(|a| {
        let new = 100 - transp;
        let changed = a.cfg.backdrop_opacity != new;
        a.cfg.backdrop_opacity = new;
        if commit {
            a.save_cfg();
            a.tick_clock();
        }
        changed
    })
    .unwrap_or(false);
    // значение идёт шагами по 5% — пока шаг тот же, окно не перерисовываем
    if changed || commit {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

/// Закончить перетаскивание ползунка (если оно шло): сохранить значение и перерисовать часы.
unsafe fn end_drag() {
    let was = UI.with(|u| u.borrow_mut().as_mut().map(|s| std::mem::replace(&mut s.dragging, false)).unwrap_or(false));
    if was {
        with_app(|a| {
            a.save_cfg();
            a.tick_clock();
        });
    }
}

/// Нажатие на элемент: переход по разделам или та же команда, что в меню трея.
unsafe fn click(hwnd: HWND, id: usize) {
    if id == 0 {
        return;
    }
    if id == CITY_SEARCH {
        start_search(hwnd);
        return;
    }
    if id >= CITY_PICK && id < CITY_PICK + 10 {
        let city = UI.with(|u| u.borrow().as_ref().and_then(|s| s.results.get(id - CITY_PICK).cloned()));
        if let Some(c) = city {
            let edit = UI.with(|u| {
                u.borrow_mut().as_mut().map(|st| {
                    st.results.clear();
                    st.search_status.clear();
                    st.edit
                })
            });
            if let Some(edit) = edit.filter(|e| !e.is_null()) {
                SetWindowTextW(edit, wide("").as_ptr());
            }
            pick_city(&c);
        }
        InvalidateRect(hwnd, std::ptr::null(), 0);
        return;
    }
    if (NAV..NAV + PAGES.len()).contains(&id) {
        UI.with(|u| {
            if let Some(st) = u.borrow_mut().as_mut() {
                st.page = id - NAV;
                st.scroll = 0.0;
                st.scroll_target = 0.0;
                st.page_t = 0.0;
                kick(hwnd, st);
            }
        });
        InvalidateRect(hwnd, std::ptr::null(), 0);
        return;
    }
    // переключатели (вкл/выкл): запоминаем прежнее состояние, чтобы ручка плавно переехала
    let switch_was = with_app(|a| match id {
        M_PAUSE => Some(a.cfg.paused),
        M_CLOCK_ON => Some(a.cfg.clock),
        M_SECONDS => Some(a.cfg.seconds),
        M_H24 => Some(a.cfg.is_24h()),
        M_SHADOW => Some(a.cfg.shadow),
        M_WEATHER_ON => Some(a.cfg.weather),
        M_SCHED_ON => Some(a.cfg.schedule),
        M_LIVE_BAT => Some(a.cfg.live_battery),
        M_VROT => Some(a.cfg.video_rotate),
        M_AUTOSTART => Some(autostart_enabled()),
        _ => None,
    })
    .flatten();
    if let Some(was) = switch_was {
        UI.with(|u| {
            if let Some(st) = u.borrow_mut().as_mut() {
                st.switch_anim = Some((id, 0.0, !was));
                kick(hwnd, st);
            }
        });
    }
    let theme_click = (M_UI_THEME..M_UI_THEME + UI_THEMES.len()).contains(&id);
    let after = with_app(|a| on_menu(a, id)).unwrap_or(After::Nothing);
    AUTOSTART_CACHE.with(|c| c.set(None)); // автозапуск мог переключиться — перечитать
    if theme_click {
        apply_titlebar(hwnd);
    }
    match after {
        After::Nothing | After::Settings => {}
        After::Open(p) => {
            windows_sys::Win32::UI::Shell::ShellExecuteW(hwnd, wide("open").as_ptr(), util::wide_os(p.as_os_str()).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        }
        After::Quit => {
            if let Some(main) = with_app(|a| a.hwnd) {
                DestroyWindow(main);
            }
            return;
        }
    }
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

// ---------------- рисование ----------------

struct TextCmd {
    r: RECT,
    s: Vec<u16>,
    f: F,
    color: u32,
    flags: u32,
}

/// Кисть для рисования одного кадра.
struct Ctx<'a> {
    gp: &'a Gp,
    g: G,
    hdc: HDC,
    s: f32,
    fonts: &'a Fonts,
    hover: usize,
    texts: Vec<TextCmd>,
    hits: Vec<(RECT, usize)>,
    view_h: f32,
    dry: bool, // true — ничего не рисуем, только считаем размеры (для карточек)
    slider_track: Option<(f32, f32)>,
    hover_prev: usize,
    hover_t: f32,
    prev_t: f32,
    switch_anim: Option<(usize, f32, bool)>,
}

/// Плавное смешивание двух цветов (ARGB), t от 0 до 1.
fn mix_argb(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |sh: u32| -> u32 {
        let (x, y) = (((a >> sh) & 0xFF) as f32, ((b >> sh) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Плавный выход в конце: быстро начинает, мягко останавливается.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl<'a> Ctx<'a> {
    /// Насколько элемент сейчас подсвечен (0..1): наведённый плавно «загорается», прежний — гаснет.
    fn hv(&self, id: usize) -> f32 {
        if id == 0 {
            0.0
        } else if id == self.hover {
            ease_out(self.hover_t)
        } else if id == self.hover_prev {
            ease_out(self.prev_t)
        } else {
            0.0
        }
    }
    fn p(&self, v: f32) -> f32 {
        v * self.s
    }
    fn visible(&self, y: f32, h: f32) -> bool {
        y + h >= 0.0 && y <= self.view_h
    }
    unsafe fn rr_path(&self, x: f32, y: f32, w: f32, h: f32, r: f32) -> G {
        let mut path: G = std::ptr::null_mut();
        (self.gp.path)(0, &mut path);
        let d = (r * 2.0).min(w).min(h).max(0.1);
        (self.gp.arc)(path, x, y, d, d, 180.0, 90.0);
        (self.gp.arc)(path, x + w - d, y, d, d, 270.0, 90.0);
        (self.gp.arc)(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        (self.gp.arc)(path, x, y + h - d, d, d, 90.0, 90.0);
        (self.gp.close)(path);
        path
    }
    fn fill_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        if self.dry || !self.visible(y, h) {
            return;
        }
        unsafe {
            let path = self.rr_path(x, y, w, h, r);
            let mut b: G = std::ptr::null_mut();
            (self.gp.solid)(color, &mut b);
            (self.gp.fill_path)(self.g, b, path);
            (self.gp.del_brush)(b);
            (self.gp.del_path)(path);
        }
    }
    fn stroke_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32, width: f32) {
        if self.dry || !self.visible(y, h) {
            return;
        }
        unsafe {
            let path = self.rr_path(x, y, w, h, r);
            let mut pen: G = std::ptr::null_mut();
            (self.gp.pen)(color, width, 2, &mut pen);
            (self.gp.draw_path)(self.g, pen, path);
            (self.gp.del_pen)(pen);
            (self.gp.del_path)(path);
        }
    }
    fn circle(&self, cx: f32, cy: f32, r: f32, color: u32) {
        if self.dry {
            return;
        }
        unsafe {
            let mut path: G = std::ptr::null_mut();
            (self.gp.path)(0, &mut path);
            (self.gp.ellipse)(path, cx - r, cy - r, r * 2.0, r * 2.0);
            let mut b: G = std::ptr::null_mut();
            (self.gp.solid)(color, &mut b);
            (self.gp.fill_path)(self.g, b, path);
            (self.gp.del_brush)(b);
            (self.gp.del_path)(path);
        }
    }
    /// Левая половина круга (для образца цвета «авто»).
    fn half_left(&self, cx: f32, cy: f32, r: f32, color: u32) {
        if self.dry {
            return;
        }
        unsafe {
            let mut path: G = std::ptr::null_mut();
            (self.gp.path)(0, &mut path);
            (self.gp.arc)(path, cx - r, cy - r, r * 2.0, r * 2.0, 90.0, 180.0);
            (self.gp.close)(path);
            let mut b: G = std::ptr::null_mut();
            (self.gp.solid)(color, &mut b);
            (self.gp.fill_path)(self.g, b, path);
            (self.gp.del_brush)(b);
            (self.gp.del_path)(path);
        }
    }
    fn ring(&self, cx: f32, cy: f32, r: f32, color: u32, width: f32) {
        if self.dry {
            return;
        }
        unsafe {
            let mut path: G = std::ptr::null_mut();
            (self.gp.path)(0, &mut path);
            (self.gp.ellipse)(path, cx - r, cy - r, r * 2.0, r * 2.0);
            let mut pen: G = std::ptr::null_mut();
            (self.gp.pen)(color, width, 2, &mut pen);
            (self.gp.draw_path)(self.g, pen, path);
            (self.gp.del_pen)(pen);
            (self.gp.del_path)(path);
        }
    }
    /// Картинка со скруглёнными углами.
    fn image(&self, img: &Img, x: f32, y: f32, w: f32, h: f32, r: f32) {
        if self.dry || !self.visible(y, h) || img.px.is_empty() {
            return;
        }
        unsafe {
            let mut bmp: G = std::ptr::null_mut();
            (self.gp.bitmap)(img.w, img.h, img.w * 4, 0x0002_2009, img.px.as_ptr() as *mut u8, &mut bmp);
            if bmp.is_null() {
                return;
            }
            let (x, y) = (x.round(), y.round());
            let exact = (w - img.w as f32).abs() < 1.5 && (h - img.h as f32).abs() < 1.5;
            let (w, h) = if exact { (img.w as f32, img.h as f32) } else { (w, h) };
            let path = self.rr_path(x, y, w, h, r);
            (self.gp.clip_path)(self.g, path, 0);
            if exact {
                // размер уже точный: переносим точки как есть — в разы быстрее бикубического сглаживания
                (self.gp.interp)(self.g, 5);
                (self.gp.draw_image)(self.g, bmp, x, y, w, h);
                (self.gp.interp)(self.g, 7);
            } else {
                (self.gp.draw_image)(self.g, bmp, x, y, w, h);
            }
            (self.gp.reset_clip)(self.g);
            (self.gp.del_path)(path);
            (self.gp.del_image)(bmp);
        }
    }
    fn text(&mut self, x: f32, y: f32, w: f32, h: f32, s: &str, f: F, color: u32, flags: u32) {
        if self.dry || !self.visible(y, h) {
            return;
        }
        let r = RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 };
        self.texts.push(TextCmd { r, s: s.encode_utf16().collect(), f, color, flags });
    }
    fn text_w(&self, s: &str, f: F) -> f32 {
        unsafe {
            let old = SelectObject(self.hdc, self.fonts.get(f) as _);
            let w: Vec<u16> = s.encode_utf16().collect();
            let mut sz = SIZE { cx: 0, cy: 0 };
            GetTextExtentPoint32W(self.hdc, w.as_ptr(), w.len() as i32, &mut sz);
            SelectObject(self.hdc, old);
            sz.cx as f32
        }
    }
    fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, id: usize) {
        if !self.dry && self.visible(y, h) {
            self.hits.push((RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 }, id));
        }
    }

    // ---- значки ----

    /// Рисует значок размером size×size (сетка 24×24). bg — цвет фона под значком (для «ручек» ползунков).
    fn icon(&self, ic: Ic, x: f32, y: f32, size: f32, color: u32, bg: u32) {
        if self.dry || !self.visible(y, size) {
            return;
        }
        let u = size / 24.0;
        let gp = self.gp;
        unsafe {
            let mut path: G = std::ptr::null_mut();
            (gp.path)(0, &mut path);
            let tx = |v: f32| x + v * u;
            let ty = |v: f32| y + v * u;
            let line = |a: f32, b: f32, c: f32, d: f32| {
                (gp.start_figure)(path);
                (gp.add_line)(path, tx(a), ty(b), tx(c), ty(d));
            };
            let poly = |pts: &[(f32, f32)], close: bool| {
                (gp.start_figure)(path);
                for w in pts.windows(2) {
                    (gp.add_line)(path, tx(w[0].0), ty(w[0].1), tx(w[1].0), ty(w[1].1));
                }
                if close {
                    (gp.close)(path);
                }
            };
            let arc = |cx: f32, cy: f32, r: f32, s: f32, sw: f32| {
                (gp.start_figure)(path);
                (gp.arc)(path, tx(cx - r), ty(cy - r), 2.0 * r * u, 2.0 * r * u, s, sw);
            };
            let circ = |cx: f32, cy: f32, r: f32| {
                (gp.start_figure)(path);
                (gp.ellipse)(path, tx(cx - r), ty(cy - r), 2.0 * r * u, 2.0 * r * u);
            };
            let rrect = |rx: f32, ry: f32, rw: f32, rh: f32, rr: f32| {
                let d = 2.0 * rr * u;
                (gp.start_figure)(path);
                (gp.arc)(path, tx(rx), ty(ry), d, d, 180.0, 90.0);
                (gp.arc)(path, tx(rx + rw) - d, ty(ry), d, d, 270.0, 90.0);
                (gp.arc)(path, tx(rx + rw) - d, ty(ry + rh) - d, d, d, 0.0, 90.0);
                (gp.arc)(path, tx(rx), ty(ry + rh) - d, d, d, 90.0, 90.0);
                (gp.close)(path);
            };
            let mut dots: Vec<(f32, f32, f32)> = Vec::new();
            let mut knobs: Vec<(f32, f32, f32)> = Vec::new();
            match ic {
                Ic::Image => {
                    rrect(3.0, 5.0, 18.0, 14.0, 3.0);
                    poly(&[(6.5, 16.0), (10.5, 11.5), (13.5, 14.5), (15.5, 12.5), (18.0, 15.0)], false);
                    dots.push((15.5, 9.0, 1.6));
                }
                Ic::Grid => {
                    rrect(4.0, 4.0, 7.0, 7.0, 2.0);
                    rrect(13.0, 4.0, 7.0, 7.0, 2.0);
                    rrect(4.0, 13.0, 7.0, 7.0, 2.0);
                    rrect(13.0, 13.0, 7.0, 7.0, 2.0);
                }
                Ic::Clock => {
                    circ(12.0, 12.0, 8.5);
                    poly(&[(12.0, 7.5), (12.0, 12.0), (15.0, 14.0)], false);
                }
                Ic::Palette => {
                    circ(12.0, 12.0, 8.5);
                    circ(14.6, 15.6, 1.9);
                    dots.push((8.3, 10.6, 1.35));
                    dots.push((12.0, 7.9, 1.35));
                    dots.push((15.7, 10.6, 1.35));
                }
                Ic::Cloud => {
                    (gp.start_figure)(path);
                    (gp.arc)(path, tx(4.5), ty(11.0), 7.0 * u, 7.0 * u, 90.0, 180.0);
                    (gp.arc)(path, tx(8.0), ty(6.5), 9.0 * u, 9.0 * u, 180.0, 180.0);
                    (gp.arc)(path, tx(13.5), ty(11.0), 7.0 * u, 7.0 * u, 270.0, 180.0);
                    (gp.close)(path);
                }
                Ic::Monitor => {
                    rrect(3.0, 4.0, 18.0, 12.5, 2.5);
                    line(12.0, 16.5, 12.0, 19.5);
                    line(8.0, 19.8, 16.0, 19.8);
                }
                Ic::Sliders => {
                    line(4.0, 7.0, 20.0, 7.0);
                    line(4.0, 12.0, 20.0, 12.0);
                    line(4.0, 17.0, 20.0, 17.0);
                    knobs.push((9.0, 7.0, 2.4));
                    knobs.push((15.0, 12.0, 2.4));
                    knobs.push((8.0, 17.0, 2.4));
                }
                Ic::Next => {
                    line(5.0, 12.0, 19.0, 12.0);
                    poly(&[(13.5, 6.5), (19.0, 12.0), (13.5, 17.5)], false);
                }
                Ic::Folder => {
                    poly(&[(3.0, 18.5), (3.0, 6.0), (9.5, 6.0), (11.5, 8.5), (21.0, 8.5), (21.0, 18.5)], true);
                }
                Ic::Search => {
                    circ(10.5, 10.5, 6.0);
                    line(15.0, 15.0, 20.0, 20.0);
                }
                Ic::Refresh => {
                    arc(12.0, 12.0, 7.5, -40.0, 300.0);
                    poly(&[(6.7, 2.3), (10.7, 4.6), (7.8, 8.2)], false);
                }
                Ic::File => {
                    rrect(5.5, 3.0, 13.0, 18.0, 2.5);
                    line(8.5, 9.0, 15.5, 9.0);
                    line(8.5, 12.5, 15.5, 12.5);
                    line(8.5, 16.0, 13.0, 16.0);
                }
                Ic::List => {
                    line(9.0, 7.0, 19.0, 7.0);
                    line(9.0, 12.0, 19.0, 12.0);
                    line(9.0, 17.0, 19.0, 17.0);
                    dots.push((5.0, 7.0, 1.3));
                    dots.push((5.0, 12.0, 1.3));
                    dots.push((5.0, 17.0, 1.3));
                }
                Ic::Power => {
                    arc(12.0, 13.0, 7.5, 300.0, 300.0);
                    line(12.0, 3.5, 12.0, 11.0);
                }
                Ic::Heart | Ic::HeartFull => {
                    // две дуги-«доли» сверху и острый низ
                    (gp.start_figure)(path);
                    (gp.arc)(path, tx(4.0), ty(5.0), 8.0 * u, 8.0 * u, 135.0, 225.0);
                    (gp.arc)(path, tx(12.0), ty(5.0), 8.0 * u, 8.0 * u, 180.0, 225.0);
                    (gp.add_line)(path, tx(18.83), ty(11.83), tx(12.0), ty(19.5));
                    (gp.close)(path);
                }
                Ic::Ban => {
                    circ(12.0, 12.0, 8.5);
                    line(6.0, 6.0, 18.0, 18.0);
                }
                Ic::Sparkle => {
                    // четырёхлучевая звёздочка и маленькая рядом
                    poly(&[(10.0, 3.5), (11.6, 9.4), (17.5, 11.0), (11.6, 12.6), (10.0, 18.5), (8.4, 12.6), (2.5, 11.0), (8.4, 9.4)], true);
                    poly(&[(18.0, 15.0), (18.7, 17.3), (21.0, 18.0), (18.7, 18.7), (18.0, 21.0), (17.3, 18.7), (15.0, 18.0), (17.3, 17.3)], true);
                }
            }
            if matches!(ic, Ic::HeartFull) {
                let mut b: G = std::ptr::null_mut();
                (gp.solid)(color, &mut b);
                (gp.fill_path)(self.g, b, path);
                (gp.del_brush)(b);
            }
            let mut pen: G = std::ptr::null_mut();
            (gp.pen)(color, (1.8 * u).max(1.2), 2, &mut pen);
            (gp.pen_start_cap)(pen, 2); // скруглённые концы
            (gp.pen_end_cap)(pen, 2);
            (gp.pen_join)(pen, 2); // скруглённые изломы
            (gp.draw_path)(self.g, pen, path);
            (gp.del_pen)(pen);
            (gp.del_path)(path);
            for (cx, cy, r) in dots {
                self.circle(tx(cx), ty(cy), r * u, color);
            }
            for (cx, cy, r) in knobs {
                self.circle(tx(cx), ty(cy), r * u + 0.5, bg);
                self.ring(tx(cx), ty(cy), r * u, color, (1.8 * u).max(1.2));
            }
        }
    }

    /// Логотип Hora размером size×size (тот же рисунок, что у значка программы, сетка 120×120).
    fn logo(&self, x: f32, y: f32, size: f32) {
        if self.dry {
            return;
        }
        let u = size / 120.0;
        self.fill_rr(x, y, size, size, 28.0 * u, argb(244, 237, 228));
        let gp = self.gp;
        unsafe {
            let mut path: G = std::ptr::null_mut();
            (gp.path)(0, &mut path);
            (gp.start_figure)(path);
            let pts = [(28.0, 100.0), (47.0, 40.0), (60.0, 74.0), (73.0, 40.0), (92.0, 100.0)];
            for w in pts.windows(2) {
                (gp.add_line)(path, x + w[0].0 * u, y + w[0].1 * u, x + w[1].0 * u, y + w[1].1 * u);
            }
            let mut pen: G = std::ptr::null_mut();
            (gp.pen)(argb(31, 36, 51), (10.0 * u).max(2.0), 2, &mut pen);
            (gp.pen_start_cap)(pen, 2);
            (gp.pen_end_cap)(pen, 2);
            (gp.pen_join)(pen, 2);
            (gp.draw_path)(self.g, pen, path);
            (gp.del_pen)(pen);
            (gp.del_path)(path);
        }
        self.circle(x + 86.0 * u, y + 26.0 * u, (8.0 * u).max(2.0), argb(232, 118, 58));
    }

    // ---- готовые элементы ----

    /// Карточка со скруглёнными углами вокруг содержимого. Сначала «сухой прогон» узнаёт высоту,
    /// потом рисуется фон и само содержимое. Возвращает y под карточкой.
    fn card(&mut self, x: f32, y: f32, w: f32, mut f: impl FnMut(&mut Self, f32, f32, f32) -> f32) -> f32 {
        let pad = self.p(18.0);
        let was = self.dry;
        self.dry = true;
        let end = f(self, x + pad, y + pad, w - pad * 2.0);
        self.dry = was;
        let h = end - y + pad;
        self.fill_rr(x, y, w, h, self.p(16.0), card());
        f(self, x + pad, y + pad, w - pad * 2.0);
        y + h + self.p(12.0)
    }

    /// Заголовок внутри карточки; возвращает y под ним.
    fn head(&mut self, x: f32, y: f32, w: f32, title: &str, hint: &str) -> f32 {
        self.text(x, y, w, self.p(22.0), title, F::H2, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        let mut y = y + self.p(24.0);
        if !hint.is_empty() {
            self.text(x, y, w, self.p(20.0), hint, F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            y += self.p(22.0);
        }
        y + self.p(8.0)
    }

    /// Карточка «заголовок + таблетки выбора».
    fn group(&mut self, x: f32, y: f32, w: f32, title: &str, hint: &str, items: &[(String, usize, bool)]) -> f32 {
        self.card(x, y, w, |c, x, y, w| {
            let y = c.head(x, y, w, title, hint);
            c.chips(x, y, w, items)
        })
    }

    /// Ползунок: дорожка-«таблетка», заполненная часть и круглая ручка; value — от 0 до 1.
    fn slider(&mut self, x: f32, y: f32, w: f32, value: f32, id: usize) {
        let h = self.p(28.0);
        let th = self.p(6.0);
        let r = self.p(10.0);
        let (tx, tw) = (x + r, w - r * 2.0);
        let cy = y + h / 2.0;
        let v = value.clamp(0.0, 1.0);
        self.fill_rr(tx, cy - th / 2.0, tw, th, th / 2.0, grid_cell());
        self.fill_rr(tx, cy - th / 2.0, (tw * v).max(th), th, th / 2.0, accent());
        let kx = tx + tw * v;
        let rr = r + self.p(2.0) * self.hv(id);
        self.circle(kx, cy, rr, argb(255, 255, 255));
        self.ring(kx, cy, rr, accent(), self.p(2.0));
        self.hit(x, y, w, h, id);
        if !self.dry {
            self.slider_track = Some((tx, tw));
        }
    }

    /// Кнопка-«таблетка» со значком; возвращает её ширину. kind: 0 обычная, 1 главная, 2 опасная.
    fn btn(&mut self, x: f32, y: f32, label: &str, id: usize, kind: u8, ic: Option<Ic>) -> f32 {
        let h = self.p(38.0);
        let isz = self.p(18.0);
        let w = self.text_w(label, F::Body) + self.p(40.0) + if ic.is_some() { isz + self.p(8.0) } else { 0.0 };
        let hv = self.hv(id);
        let (fill, tc) = match kind {
            1 => (mix_argb(accent(), accent_hover(), hv), t_on_accent()),
            2 => (mix_argb(danger(), danger_hover(), hv), t_on_accent()),
            _ => (mix_argb(bg(), card_hover(), hv), t_main()),
        };
        self.fill_rr(x, y, w, h, h / 2.0, fill);
        if kind == 0 {
            self.stroke_rr(x + 0.5, y + 0.5, w - 1.0, h - 1.0, h / 2.0, border(), 1.0);
        }
        let mut tx = x + self.p(20.0);
        if let Some(ic) = ic {
            let col = 0xFF00_0000 | ((tc & 0xFF) << 16) | (tc & 0xFF00) | ((tc >> 16) & 0xFF);
            self.icon(ic, tx, y + (h - isz) / 2.0, isz, col, fill);
            tx += isz + self.p(8.0);
        }
        self.text(tx, y, w - (tx - x) - self.p(16.0), h, label, F::Body, tc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        self.hit(x, y, w, h, id);
        w
    }

    /// Ряд «таблеток» с переносом строк; возвращает y под ними.
    fn chips(&mut self, x: f32, y: f32, w: f32, items: &[(String, usize, bool)]) -> f32 {
        let h = self.p(32.0);
        let gap = self.p(8.0);
        let pad = self.p(15.0);
        let (mut cx, mut cy) = (x, y);
        for (label, id, on) in items {
            let cw = self.text_w(label, F::Body) + pad * 2.0;
            if cx > x && cx + cw > x + w {
                cx = x;
                cy += h + gap;
            }
            let hv = self.hv(*id);
            let fill = if *on { mix_argb(accent(), accent_hover(), hv) } else { mix_argb(card(), card_hover(), hv) };
            self.fill_rr(cx, cy, cw, h, h / 2.0, fill);
            if !*on {
                self.stroke_rr(cx + 0.5, cy + 0.5, cw - 1.0, h - 1.0, h / 2.0, border(), 1.0);
            }
            self.text(cx, cy, cw, h, label, F::Body, if *on { t_on_accent() } else { t_main() }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            self.hit(cx, cy, cw, h, *id);
            cx += cw + gap;
        }
        cy + h
    }

    /// Строка-карточка с переключателем справа; возвращает y под ней.
    fn toggle(&mut self, x: f32, y: f32, w: f32, label: &str, hint: &str, on: bool, id: usize) -> f32 {
        let h = self.p(if hint.is_empty() { 50.0 } else { 62.0 });
        self.fill_rr(x, y, w, h, self.p(16.0), mix_argb(card(), card_hover(), self.hv(id)));
        let tx = x + self.p(18.0);
        if hint.is_empty() {
            self.text(tx, y, w - self.p(90.0), h, label, F::Body, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        } else {
            self.text(tx, y + self.p(10.0), w - self.p(90.0), self.p(22.0), label, F::Body, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            self.text(tx, y + self.p(32.0), w - self.p(90.0), self.p(20.0), hint, F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        // переключатель
        let (sw, sh) = (self.p(44.0), self.p(24.0));
        let sx = x + w - sw - self.p(18.0);
        let sy = y + (h - sh) / 2.0;
        // насколько переключатель «включён» (0..1) — во время щелчка ручка плавно переезжает
        let k = match self.switch_anim {
            Some((sid, t, to_on)) if sid == id => {
                let e = ease_out(t);
                if to_on { e } else { 1.0 - e }
            }
            _ => {
                if on { 1.0 } else { 0.0 }
            }
        };
        if k < 1.0 {
            self.stroke_rr(sx + 0.5, sy + 0.5, sw - 1.0, sh - 1.0, sh / 2.0, switch_off(), self.p(1.5));
        }
        if k > 0.0 {
            self.fill_rr(sx, sy, sw, sh, sh / 2.0, mix_argb(accent() & 0x00FF_FFFF, accent(), k));
        }
        let kx = sx + sh / 2.0 + (sw - sh) * k;
        let kr = sh / 2.0 - self.p(6.0) + self.p(2.0) * k;
        self.circle(kx, sy + sh / 2.0, kr, mix_argb(knob_off(), argb(255, 255, 255), k));
        self.hit(x, y, w, h, id);
        y + h + self.p(12.0)
    }

    /// Сетка 3×3 для выбора положения на экране; возвращает y под ней.
    fn pos_grid(&mut self, x: f32, y: f32, w: f32, current: &str, base_id: usize) -> f32 {
        let grid = ["top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right"];
        let (gw, gh) = (self.p(54.0), self.p(32.0));
        self.fill_rr(x, y, gw * 3.0 + self.p(24.0), gh * 3.0 + self.p(24.0), self.p(14.0), bg());
        for (k, name) in grid.iter().enumerate() {
            let idx = POSITIONS.iter().position(|(v, _)| v == name).unwrap_or(0);
            let id = base_id + idx;
            let bx = x + self.p(8.0) + (k % 3) as f32 * (gw + self.p(4.0));
            let by = y + self.p(8.0) + (k / 3) as f32 * (gh + self.p(4.0));
            let on = current == *name;
            self.fill_rr(bx, by, gw, gh, self.p(9.0), if on { accent() } else { mix_argb(grid_cell(), card_hover(), self.hv(id)) });
            self.circle(bx + gw / 2.0, by + gh / 2.0, self.p(3.5), if on { argb(255, 255, 255) } else { grid_dot() });
            self.hit(bx, by, gw, gh, id);
        }
        let pname = POSITIONS.iter().find(|(v, _)| *v == current).map(|(_, t)| tr(t)).unwrap_or("");
        self.text(x + gw * 3.0 + self.p(44.0), y, w, gh * 3.0 + self.p(24.0), pname, F::Body, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        y + gh * 3.0 + self.p(24.0)
    }
}

fn get_thumb<'a>(thumbs: &'a mut Vec<(usize, i32, i32, Img)>, cache: &std::path::Path, mon: usize, w: i32, h: i32) -> Option<&'a Img> {
    if !thumbs.iter().any(|t| t.0 == mon && t.1 == w && t.2 == h) {
        let px = bg::thumbnail(&bg::file_for(cache, mon), w, h).unwrap_or_default();
        thumbs.push((mon, w, h, Img { w, h, px }));
    }
    thumbs.iter().find(|t| t.0 == mon && t.1 == w && t.2 == h).map(|t| &t.3)
}

/// Предпросмотр: уменьшенные обои основного монитора и часы поверх них, как на рабочем столе.
fn build_preview(st: &mut State, cfg: &Config, cache: &std::path::Path, w: i32, h: i32, wx: Option<&weather::Weather>) {
    let mut now: clock::SysTime = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut now) };
    let minute = now.wHour * 60 + now.wMinute;
    let wkey = format!("{:?}", wx);
    if let Some((c, m, pw, ph, k, _)) = &st.preview {
        if c == cfg && *m == minute && *pw == w && *ph == h && *k == wkey {
            return;
        }
    }
    let mut img = get_thumb(&mut st.thumbs, cache, 0, w, h).map(|t| Img { w: t.w, h: t.h, px: t.px.clone() }).unwrap_or(Img { w, h, px: vec![0xFF20_2228; (w * h) as usize] });
    if img.px.is_empty() {
        img.px = vec![0xFF20_2228; (w * h) as usize];
    }
    let mons = clock::monitors();
    let m = mons[0];
    let (mw, mh) = ((m.rect.right - m.rect.left).max(1), (m.rect.bottom - m.rect.top).max(1));
    let sx = |v: i32| ((v - m.rect.left) as i64 * w as i64 / mw as i64) as i32;
    let sy = |v: i32| ((v - m.rect.top) as i64 * h as i64 / mh as i64) as i32;
    let work = RECT { left: sx(m.work.left), top: sy(m.work.top), right: sx(m.work.right), bottom: sy(m.work.bottom) };
    let mut c2 = cfg.clone();
    if c2.color == config::COLOR_AUTO {
        c2.color = 0xFFFFFF;
    }
    // слои как на рабочем столе основного монитора: часы и (если включена) погода
    let mut frames = Vec::new();
    if cfg.clock && clock::clock_targets(cfg, &mons).contains(&0) {
        if let Some(fr) = clock::compose(&c2, &now, (w, h), &mut st.clock_fonts) {
            let pos = clock::place(&c2, &fr, (w, h), work);
            frames.push((fr, pos));
        }
    }
    if let Some(wd) = wx.filter(|_| cfg.weather && clock::weather_targets(cfg, &mons).contains(&0)) {
        if let Some(fr) = clock::compose_weather(&c2, &now, (w, h), &mut st.clock_fonts, wd) {
            let pos = clock::place_at(&fr, (w, h), work, &cfg.weather_position, 0, 0);
            frames.push((fr, pos));
        }
    }
    // фон для подложки берём из чистой миниатюры (до наложения часов)
    let base = img.px.clone();
    let reader = |rx: i32, ry: i32, rw: i32, rh: i32, out: &mut [u32]| -> bool {
        for yy in 0..rh {
            for xx in 0..rw {
                // за краем — крайние точки миниатюры (как на рабочем столе: без тёмной каймы у подложки)
                let (sx, sy) = ((rx + xx).clamp(0, w - 1), (ry + yy).clamp(0, h - 1));
                out[(yy * rw + xx) as usize] = base.get((sy * w + sx) as usize).copied().unwrap_or(0);
            }
        }
        true
    };
    for (fr, (x, y)) in frames {
        // подложка — по тем же правилам, что на рабочем столе
        let (tw, th) = (fr.text.right - fr.text.left, fr.text.bottom - fr.text.top);
        let mut under = vec![0u32; (tw.max(1) * th.max(1)) as usize];
        reader(x + (fr.text.left - fr.all.left), y + (fr.text.top - fr.all.top), tw.max(1), th.max(1), &mut under);
        let (light, dark) = clock::split_of(&under);
        let plate_on = cfg.backdrop == "on" || (cfg.backdrop == "auto" && light > 0.15 && dark > 0.15);
        let text_dark = cfg.color != config::COLOR_AUTO && ((cfg.color >> 16 & 0xFF) * 299 + (cfg.color >> 8 & 0xFF) * 587 + (cfg.color & 0xFF) * 114) / 1000 < 110;
        let plate = if plate_on { Some(clock::Plate::for_height(h, !text_dark, cfg.backdrop_opacity)) } else { None };
        let (lpx, lr) = clock::make_layer(&fr, x, y, plate.as_ref(), &reader, None);
        let lw = lr.right - lr.left;
        {
            let op = cfg.opacity.clamp(10, 100);
            for row in 0..(lr.bottom - lr.top) {
                for col in 0..lw {
                    let (dx, dy) = (lr.left + col, lr.top + row);
                    if dx < 0 || dy < 0 || dx >= w || dy >= h {
                        continue;
                    }
                    let s = lpx[(row * lw + col) as usize];
                    let a = (s >> 24) * op / 100;
                    if a == 0 {
                        continue;
                    }
                    let d = &mut img.px[(dy * w + dx) as usize];
                    let mix = |sh: u32| -> u32 { (((s >> sh) & 0xFF) * op / 100 + ((*d >> sh) & 0xFF) * (255 - a) / 255).min(255) };
                    *d = 0xFF00_0000 | (mix(16) << 16) | (mix(8) << 8) | mix(0);
                }
            }
        }
    }
    st.preview = Some((cfg.clone(), minute, w, h, wkey, img));
}

fn memory_mb() -> f32 {
    use windows_sys::Win32::System::ProcessStatus::*;
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        K32GetProcessMemoryInfo(windows_sys::Win32::System::Threading::GetCurrentProcess(), &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, pmc.cb);
        pmc.PrivateUsage as f32 / 1024.0 / 1024.0
    }
}

// замер скорости отрисовки окна: (кадров, суммарно мкс, худший кадр мкс)
thread_local! {
    static PAINT_STATS: Cell<(u32, u64, u64)> = const { Cell::new((0, 0, 0)) };
}

unsafe fn paint(st: &mut State, hdc: HDC, snap: Option<&Snap>) {
    let t0 = std::time::Instant::now();
    paint_inner(st, hdc, snap);
    let us = t0.elapsed().as_micros() as u64;
    PAINT_STATS.with(|s| {
        let (n, sum, max) = s.get();
        s.set((n + 1, sum + us, max.max(us)));
    });
}

unsafe fn paint_inner(st: &mut State, hdc: HDC, snap: Option<&Snap>) {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetClientRect(st.hwnd, &mut rc);
    let (cw, ch) = (rc.right.max(1), rc.bottom.max(1));
    // буфер кадра (рисуем в память, потом одним махом на экран — без мерцания)
    if st.buf.map(|b| b.3 != cw || b.4 != ch).unwrap_or(true) {
        if let Some((dc, bmp, old, _, _)) = st.buf.take() {
            SelectObject(dc, old);
            DeleteObject(bmp as _);
            DeleteDC(dc);
        }
        let dc = CreateCompatibleDC(hdc);
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = cw;
        bi.bmiHeader.biHeight = -ch;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old = SelectObject(dc, bmp as _);
        st.buf = Some((dc, bmp, old, cw, ch));
    }
    let (mdc, ..) = st.buf.unwrap();
    let s = st.s;
    if let Some(sn) = snap {
        LIGHT.with(|l| l.set(is_light(&sn.cfg.ui_theme)));
    }
    let p = |v: f32| v * s;

    // предпросмотр и миниатюры готовим до начала рисования (им нужен &mut st)
    let side = p(220.0);
    let cx0 = side + p(32.0);
    let cwid = cw as f32 - cx0 - p(32.0);
    let prev_w = cwid.min(p(640.0));
    let prev_h = prev_w * 9.0 / 16.0;
    if let Some(sn) = snap {
        if st.page == 2 || st.page == 3 || st.page == P_WEATHER {
            build_preview(st, &sn.cfg, &sn.cache, prev_w as i32, prev_h as i32, sn.weather.as_ref());
        }
    }

    let mut g: G = std::ptr::null_mut();
    (st.gp.from_hdc)(mdc, &mut g);
    (st.gp.smoothing)(g, 4);
    (st.gp.interp)(g, 7);
    let mut c = Ctx { gp: &st.gp, g, hdc: mdc, s, fonts: &st.fonts, hover: st.hover, texts: Vec::new(), hits: Vec::new(), view_h: ch as f32, dry: false, slider_track: None, hover_prev: st.hover_prev, hover_t: st.hover_t, prev_t: st.prev_t, switch_anim: st.switch_anim };

    // фон
    c.fill_rr(-10.0, -10.0, cw as f32 + 20.0, ch as f32 + 20.0, 0.0, bg());

    let mut content_h = 0.0;
    let mut st_edit_rect: Option<RECT> = None;
    let search_status = st.search_status.clone();
    let results = st.results.clone();
    if let Some(sn) = snap {
        let x = cx0;
        let w = cwid;
        let shift = (st.scroll - (1.0 - ease_out(st.page_t)) * p(18.0)).round();
        let mut y = p(28.0) - shift;
        let cfg = &sn.cfg;
        // заголовок раздела и подсказка под ним
        c.text(x, y, w, p(36.0), tr(PAGES[st.page]), F::Title, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        y += p(36.0);
        c.text(x, y, w, p(20.0), tr(PAGE_HINTS[st.page]), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        y += p(34.0);
        // предпросмотр — для «Часов», «Оформления» и «Погоды»
        let preview = |c: &mut Ctx, y: f32| -> f32 {
            if let Some((.., img)) = &st.preview {
                c.image(img, x, y, prev_w, prev_h, p(16.0));
            }
            y + prev_h + p(14.0)
        };
        match st.page {
            // ---------------- Обои ----------------
            0 => {
                // главная карточка: миниатюры текущих обоев, подписи и кнопки
                let tw = p(288.0);
                let th = tw * 9.0 / 16.0;
                let card_h = p(18.0) + th + p(16.0) + p(38.0) + p(18.0);
                c.fill_rr(x, y, w, card_h, p(16.0), card());
                let mons = clock::monitors();
                let per = cfg.wall_mode != "same" && mons.len() > 1;
                let n = if per { mons.len().min(2) } else { 1 };
                let info_x = x + tw + p(36.0);
                let info_w = w - tw - p(54.0);
                c.text(info_x, y + p(22.0), info_w, p(22.0), tr("Сейчас на рабочем столе"), F::H2, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                let names: Vec<String> = sn.current.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                let names = if names.is_empty() { "—".to_string() } else { names.join(", ") };
                c.text(info_x, y + p(48.0), info_w, p(20.0), &names, F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                let next = if cfg.paused {
                    tr("Автосмена на паузе").to_string()
                } else if cfg.interval_min == 0 {
                    tr("Автосмена выключена — только вручную").to_string()
                } else {
                    let left = (cfg.interval_min as u64 * 60).saturating_sub(util::now_unix().saturating_sub(sn.last_change));
                    if crate::i18n::en() { format!("Next wallpaper in {} min", (left + 59) / 60) } else { format!("Следующие обои через {} мин", (left + 59) / 60) }
                };
                c.icon(Ic::Clock, info_x, y + p(78.0), p(18.0), argb_of(t_sec()), card());
                c.text(info_x + p(26.0), y + p(77.0), info_w, p(20.0), &next, F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                if sn.status.starts_with(tr("ошибка")) {
                    c.text(info_x, y + p(104.0), info_w, p(20.0), &sn.status, F::Small, t_error(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                }
                let by = y + card_h - p(56.0);
                let bx = x + p(18.0);
                let bw = c.btn(bx, by, tr("Следующие обои"), M_NEXT, 1, Some(Ic::Next));
                let (fl, fi) = if sn.cur_fav { (tr("В избранном"), Ic::HeartFull) } else { (tr("В избранное"), Ic::Heart) };
                let bw2 = c.btn(bx + bw + p(8.0), by, fl, M_FAV, 0, Some(fi));
                c.btn(bx + bw + bw2 + p(16.0), by, tr("Не показывать"), M_BAN, 0, Some(Ic::Ban));
                let thumbs_y = y + p(18.0);
                y += card_h + p(12.0);
                y = c.toggle(x, y, w, tr("Пауза автосмены"), tr("Обои не будут меняться сами, пока пауза включена"), cfg.paused, M_PAUSE);

                let mixes: Vec<_> = FAV_MIXES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_FAV_MIX + i, cfg.favorites_mix == *v)).collect();
                let fav_hint = if crate::i18n::en() { format!("In favorites: {} · hidden: {}", sn.fav_count, sn.ban_count) } else { format!("В избранном: {} · скрыто: {}", sn.fav_count, sn.ban_count) };
                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Избранное и скрытые"), &fav_hint);
                    let y = c.head(x, y + p(4.0), w, tr("Показывать избранное среди новых обоев"), "");
                    let y = c.chips(x, y, w, &mixes) + p(14.0);
                    let b1 = c.btn(x, y, tr("Открыть избранное"), M_FAV_DIR, 0, Some(Ic::Heart));
                    let b2 = c.btn(x + b1 + p(8.0), y, tr("Моя папка"), M_FOLDER, 0, Some(Ic::Folder));
                    c.btn(x + b1 + b2 + p(16.0), y, tr("Список скрытых"), M_BAN_OPEN, 0, Some(Ic::List));
                    let mut y = y + p(38.0);
                    if sn.can_undo {
                        y += p(8.0);
                        c.btn(x, y, tr("Вернуть последние скрытые"), M_BAN_UNDO, 0, Some(Ic::Refresh));
                        y += p(38.0);
                    }
                    y
                });

                let items: Vec<_> = SOURCES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_SOURCE + i, cfg.source == *v)).collect();
                y = c.group(x, y, w, tr("Откуда брать обои"), "", &items);
                let items: Vec<_> = INTERVALS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_INTERVAL + i, cfg.interval_min == *v)).collect();
                y = c.group(x, y, w, tr("Менять каждые"), "", &items);
                let anims: Vec<_> = ANIMS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_ANIM + i, cfg.anim == *v)).collect();
                let speeds: Vec<_> = ANIM_SPEEDS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_ANIM_SPEED + i, cfg.anim_ms == *v)).collect();
                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Анимация смены"), tr("Эффекты делает сама Windows — процессор почти не тратится"));
                    let y = c.chips(x, y, w, &anims) + p(14.0);
                    let y = c.head(x, y, w, tr("Скорость"), "");
                    c.chips(x, y, w, &speeds)
                });

                // миниатюры обоев (по одной на монитор в режиме «разные» или «единые»)
                content_h = y + st.scroll + p(20.0);
                let mut list: Vec<(f32, f32, f32, f32, usize)> = Vec::new();
                let mut ix = x + p(18.0);
                for mi in 0..n {
                    let ew = if n == 2 { (tw - p(8.0)) / 2.0 } else { tw };
                    let eh = ew * 9.0 / 16.0;
                    list.push((ix, thumbs_y + (th - eh) / 2.0, ew, eh, mi));
                    ix += ew + p(8.0);
                }
                for (ix, iy, ew, eh, mi) in list {
                    if let Some(img) = get_thumb(&mut st.thumbs, &sn.cache, mi, ew as i32, eh as i32) {
                        c.image(img, ix, iy, ew, eh, p(12.0));
                    }
                }
            }
            // ---------------- Категории ----------------
            1 => {
                let chosen: Vec<&str> = cfg.topics.split(',').filter(|s| !s.is_empty()).collect();
                let mut items = vec![(tr("Любые (популярное)").to_string(), M_TOPIC_ANY, chosen.is_empty() && cfg.query.trim().is_empty())];
                items.extend(wallpaper::TOPICS.iter().enumerate().map(|(i, t)| (tr(t.1).to_string(), M_TOPIC + i, chosen.contains(&t.0))));
                y = c.group(x, y, w, tr("Темы"), tr("Каждый раз берётся случайная из отмеченных. Картинки — wallhaven.cc"), &items);
                if chosen.is_empty() && !cfg.query.trim().is_empty() {
                    c.text(x + p(4.0), y, w, p(20.0), &(if crate::i18n::en() { format!("A custom query from config.ini is active: \"{}\"", cfg.query.trim()) } else { format!("Сейчас действует свой запрос из config.ini: «{}»", cfg.query.trim()) }), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    y += p(26.0);
                }
                if cfg.source == "folder" {
                    c.text(x + p(4.0), y, w, p(20.0), tr("Сейчас обои берутся из вашей папки — выбор темы переключит источник на Wallhaven."), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    y += p(26.0);
                }

                // ---- расписание: свои категории на утро, день, вечер и ночь ----
                y = c.toggle(x, y, w, tr("Менять темы по времени суток"), tr("Утром природа, вечером закаты, ночью космос — обои меняются сами"), cfg.schedule, M_SCHED_ON);
                if cfg.schedule {
                    let hours = cfg.slot_hours();
                    let now = cfg.slot_at(clock_hour());
                    for slot in 0..4 {
                        let chosen: Vec<&str> = cfg.topics_slot[slot].split(',').filter(|s| !s.is_empty()).collect();
                        let base = M_SCHED + slot * 20;
                        let mut items = vec![(tr("Как основные").to_string(), base + SCHED_SAME, chosen.is_empty())];
                        items.extend(wallpaper::TOPICS.iter().enumerate().map(|(i, t)| (tr(t.1).to_string(), base + i, chosen.contains(&t.0))));
                        let (from, to) = (hours[slot], if slot == 3 { hours[0] } else { hours[slot + 1] });
                        let title = format!("{} · {}:00–{}:00{}", tr(config::SLOTS[slot]), from % 24, to % 24, if slot == now { tr(" — сейчас") } else { "" });
                        let hint = if chosen.is_empty() { tr("Берутся основные темы, отмеченные выше") } else { "" };
                        y = c.group(x, y, w, &title, hint, &items);
                    }
                    c.text(x + p(4.0), y, w, p(20.0), tr("Часы начала утра, дня, вечера и ночи меняются в config.ini (schedule_hours)."), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    y += p(26.0);
                }
                content_h = y + st.scroll + p(20.0);
            }
            // ---------------- Часы ----------------
            2 => {
                y = preview(&mut c, y);
                y = c.toggle(x, y, w, tr("Показывать часы"), tr("Большие часы на рабочем столе, под иконками"), cfg.clock, M_CLOCK_ON);
                let items: Vec<_> = STYLES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_STYLE + i, cfg.style == *v)).collect();
                y = c.group(x, y, w, tr("Стиль"), "", &items);
                y = c.toggle(x, y, w, tr("Секунды"), tr("Часы обновляются раз в секунду вместо раза в минуту"), cfg.seconds, M_SECONDS);
                y = c.toggle(x, y, w, tr("24-часовой формат"), "", cfg.is_24h(), M_H24);
                y = c.toggle(x, y, w, tr("Тень под текстом"), tr("Помогает читать часы на светлых обоях"), cfg.shadow, M_SHADOW);
                let items: Vec<_> = LANGS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_LANG + i, cfg.lang == *v)).collect();
                y = c.group(x, y, w, tr("Язык"), "", &items);
                content_h = y + st.scroll + p(20.0);
            }
            // ---------------- Оформление ----------------
            3 => {
                y = preview(&mut c, y);
                let cname = COLORS.iter().find(|(v, _)| *v == cfg.color).map(|(_, t)| tr(t)).unwrap_or(tr("свой цвет из config.ini"));
                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Цвет"), cname);
                    let r = p(15.0);
                    let step = r * 2.0 + p(12.0);
                    let per_row = ((w + p(12.0)) / step).floor().max(1.0) as usize;
                    for (i, (v, _)) in COLORS.iter().enumerate() {
                        let id = M_COLOR + i;
                        let sx = x + r + p(4.0) + (i % per_row) as f32 * step;
                        let cy = y + r + p(4.0) + (i / per_row) as f32 * step;
                        if *v == config::COLOR_AUTO {
                            // «авто»: половина светлая, половина тёмная
                            c.circle(sx, cy, r, argb(28, 30, 34));
                            c.half_left(sx, cy, r, argb(255, 255, 255));
                        } else {
                            c.circle(sx, cy, r, 0xFF00_0000 | v);
                        }
                        c.ring(sx, cy, r, swatch_ring(), 1.0);
                        if cfg.color == *v {
                            c.ring(sx, cy, r + p(4.5), accent(), p(2.5));
                        } else if c.hv(id) > 0.02 {
                            c.ring(sx, cy, r + p(4.0), mix_argb(hover_ring() & 0x00FF_FFFF, hover_ring(), c.hv(id)), p(1.5));
                        }
                        c.hit(sx - r - p(4.0), cy - r - p(4.0), r * 2.0 + p(8.0), r * 2.0 + p(8.0), id);
                    }
                    let rows = (COLORS.len() + per_row - 1) / per_row;
                    y + rows as f32 * step + p(2.0)
                });
                let items: Vec<_> = BACKDROPS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_BACKDROP + i, cfg.backdrop == *v)).collect();
                let transp = 100 - cfg.backdrop_opacity.clamp(10, 100);
                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Подложка под часами"), tr("Матовое стекло: размытые обои под текстом — читается на любом фоне"));
                    let mut y = c.chips(x, y, w, &items);
                    if cfg.backdrop != "off" {
                        // ползунок прозрачности: 0% — плотное стекло, 90% — почти невидимое
                        y += p(16.0);
                        c.text(x, y, w, p(22.0), tr("Прозрачность подложки"), F::Body, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                        c.text(x, y, w, p(22.0), &format!("{transp}%"), F::Body, t_sec(), DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
                        y += p(28.0);
                        c.slider(x, y, w, transp as f32 / 90.0, SLIDER_PLATE);
                        y += p(28.0);
                    }
                    y
                });
                let items: Vec<_> = OPACITIES.iter().enumerate().map(|(i, v)| (format!("{v}%"), M_OPACITY + i, cfg.opacity == *v)).collect();
                y = c.group(x, y, w, tr("Непрозрачность"), "", &items);
                let items: Vec<_> = SCALES.iter().enumerate().map(|(i, v)| (format!("{v}%"), M_SCALE + i, cfg.scale == *v)).collect();
                y = c.group(x, y, w, tr("Размер"), "", &items);
                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Положение на экране"), "");
                    c.pos_grid(x, y, w, &cfg.position, M_POS)
                });
                let cur = if cfg.font.is_empty() { "auto" } else { cfg.font.as_str() };
                let items: Vec<_> = FONTS.iter().enumerate().filter(|(_, (v, _))| !v.is_empty()).map(|(i, (v, t))| (tr(t).split(" — ").next().unwrap_or(t).to_string(), M_FONT + i, cur.eq_ignore_ascii_case(v))).collect();
                y = c.group(x, y, w, tr("Шрифт времени"), tr("Michroma — только латиница; для русского выбирайте любой другой"), &items);
                let cur2 = if cfg.font2.is_empty() { "auto" } else { cfg.font2.as_str() };
                let items: Vec<_> = FONTS2.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_FONT2 + i, cur2.eq_ignore_ascii_case(v))).collect();
                y = c.group(x, y, w, tr("Шрифт подписей"), tr("Дата и день недели"), &items);
                content_h = y + st.scroll + p(20.0);
            }
            // ---------------- Погода ----------------
            P_WEATHER => {
                y = preview(&mut c, y);
                y = c.toggle(x, y, w, tr("Показывать погоду"), tr("Данные open-meteo.com, обновление раз в 30 минут"), cfg.weather, M_WEATHER_ON);

                // город: поле ввода + «Найти» + найденные варианты
                let cur = if cfg.weather_city.is_empty() { tr("Город не выбран — найдите его ниже").to_string() } else { if crate::i18n::en() { format!("Now: {}", cfg.weather_city) } else { format!("Сейчас: {}", cfg.weather_city) } };
                let mut edit_at: Option<RECT> = None;
                y = c.card(x, y, w, |c, x, y, w| {
                    let mut y = c.head(x, y, w, tr("Город"), &cur);
                    let ih = p(40.0);
                    let bw_ = c.text_w(tr("Найти"), F::Body) + p(40.0) + p(26.0);
                    let iw = (w - bw_ - p(8.0)).min(p(420.0));
                    c.fill_rr(x, y, iw, ih, ih / 2.0, bg());
                    c.stroke_rr(x + 0.5, y + 0.5, iw - 1.0, ih - 1.0, ih / 2.0, border(), 1.0);
                    c.icon(Ic::Search, x + p(14.0), y + (ih - p(18.0)) / 2.0, p(18.0), argb_of(t_sec()), bg());
                    // само поле Windows ставим внутрь нарисованной рамки (после кадра)
                    let fh = p(22.0);
                    if !c.dry && c.visible(y, ih) {
                        edit_at = Some(RECT { left: (x + p(40.0)) as i32, top: (y + (ih - fh) / 2.0) as i32, right: (x + iw - p(16.0)) as i32, bottom: (y + (ih + fh) / 2.0) as i32 });
                    }
                    c.btn(x + iw + p(8.0), y + (ih - p(38.0)) / 2.0, tr("Найти"), CITY_SEARCH, 1, Some(Ic::Search));
                    y += ih + p(8.0);
                    c.text(x + p(4.0), y, w, p(20.0), tr("Напишите город на русском или английском и нажмите Enter"), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    y += p(22.0);
                    if !search_status.is_empty() {
                        y += p(6.0);
                        c.text(x, y, w, p(22.0), &search_status, F::Body, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                        y += p(28.0);
                    }
                    if !results.is_empty() {
                        let items: Vec<_> = results.iter().enumerate().map(|(i, r)| (r.label(), CITY_PICK + i, false)).collect();
                        y = c.chips(x, y, w, &items);
                    }
                    y
                });
                st_edit_rect = edit_at;

                // текущая погода
                let now_text = match &sn.weather {
                    Some(wd) => format!("{}  ·  {}", weather::deg(wd.temp), weather::describe(wd.code, crate::i18n::en())),
                    None if sn.weather_busy => tr("Загружаю…").to_string(),
                    None if cfg.weather_city.is_empty() => tr("Сначала выберите город").to_string(),
                    None => tr("Нет данных").to_string(),
                };
                let upd = match &sn.weather {
                    Some(wd) => if crate::i18n::en() { format!("updated at {:02}:{:02}", wd.fetched.0, wd.fetched.1) } else { format!("обновлено в {:02}:{:02}", wd.fetched.0, wd.fetched.1) },
                    None => String::new(),
                };
                y = c.card(x, y, w, |c, x, y, w| {
                    let rb = c.text_w(tr("Обновить"), F::Body) + p(40.0) + p(26.0);
                    c.icon(Ic::Cloud, x, y + p(4.0), p(28.0), argb_of(t_main()), card());
                    c.text(x + p(40.0), y, w - rb - p(48.0), p(22.0), &now_text, F::H2, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    c.text(x + p(40.0), y + p(22.0), w - rb - p(48.0), p(18.0), &upd, F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    if !cfg.weather_city.is_empty() {
                        c.btn(x + w - rb, y, tr("Обновить"), M_WEATHER_REFRESH, 0, Some(Ic::Refresh));
                    }
                    let mut y = y + p(40.0);
                    if !sn.weather_status.is_empty() {
                        c.text(x, y + p(6.0), w, p(20.0), &(if crate::i18n::en() { format!("Error: {}", sn.weather_status) } else { format!("Ошибка: {}", sn.weather_status) }), F::Small, t_error(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                        y += p(28.0);
                    }
                    y
                });

                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Положение на экране"), tr("Можно поставить в другой угол, чем часы"));
                    c.pos_grid(x, y, w, &cfg.weather_position, M_WPOS)
                });
                let mons = clock::monitors();
                if mons.len() > 1 {
                    let items: Vec<_> = CLOCK_MONS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_WMON + i, cfg.weather_monitor == *v)).collect();
                    y = c.group(x, y, w, tr("На мониторе"), "", &items);
                }
                let items: Vec<_> = SCALES.iter().enumerate().map(|(i, v)| (format!("{v}%"), M_WSCALE + i, cfg.weather_scale == *v)).collect();
                y = c.group(x, y, w, tr("Размер"), "", &items);
                content_h = y + st.scroll + p(20.0);
            }
            // ---------------- Мониторы ----------------
            P_LIVE => {
                let modes: Vec<_> = LIVE_MODES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_LIVE + i, cfg.live == *v)).collect();
                let status = match (cfg.live.as_str(), sn.live_running) {
                    ("off", _) => tr("Выключены — программа занимает минимум памяти").to_string(),
                    (_, true) => tr("Работают. Замирают, когда рабочий стол закрыт окнами или экран заблокирован").to_string(),
                    _ => tr("Запускаются…").to_string(),
                };
                y = c.group(x, y, w, tr("Живые обои"), &status, &modes);
                if cfg.live == "effects" {
                    let fx: Vec<_> = LIVE_EFFECTS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_LIVE_FX + i, cfg.live_effect == *v)).collect();
                    let hint = if cfg.live_effect == "weather" { tr("По погоде: в снегопад — снег, в дождь — дождь, ясной ночью — звёзды") } else { tr("Рисуется поверх обычных обоев и часов") };
                    y = c.group(x, y, w, tr("Эффект"), hint, &fx);
                    let dens: Vec<_> = LIVE_DENSITY.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_LIVE_DENS + i, cfg.live_density == *v)).collect();
                    y = c.group(x, y, w, tr("Сколько"), "", &dens);
                }
                if cfg.live == "video" {
                    let srcs: Vec<_> = VIDEO_SOURCES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_VSRC + i, cfg.video_source == *v)).collect();
                    y = c.group(x, y, w, tr("Откуда брать видео"), tr("Из Wallpaper Engine берутся только видеообои — сцены и веб-обои он показывает сам"), &srcs);
                    let list: Vec<_> = sn
                        .videos
                        .iter()
                        .enumerate()
                        .map(|(i, (t, on, we))| {
                            let mut s: String = t.chars().take(34).collect();
                            if t.chars().count() > 34 {
                                s.push('…');
                            }
                            if *we {
                                s = format!("{s} · WE");
                            }
                            (s, M_VPICK + i, *on)
                        })
                        .collect();
                    let found = if list.is_empty() {
                        tr("Пока ничего: положите mp4 или webm в папку с видео либо подпишитесь на видеообои в Wallpaper Engine").to_string()
                    } else if sn.videos_total > list.len() {
                        if crate::i18n::en() { format!("Found: {}, showing the first {}. Use \"Next video\" for the rest", sn.videos_total, list.len()) } else { format!("Найдено: {}, показаны первые {}. Остальные — кнопкой «Следующее видео»", sn.videos_total, list.len()) }
                    } else {
                        if crate::i18n::en() { format!("Found: {}. Click one to set it", list.len()) } else { format!("Найдено: {}. Нажмите, чтобы поставить", list.len()) }
                    };
                    y = c.card(x, y, w, |c, x, y, w| {
                        let y = c.head(x, y, w, tr("Видео"), &found);
                        let y = if list.is_empty() { y } else { c.chips(x, y, w, &list) + p(14.0) };
                        let b1 = c.btn(x, y, tr("Следующее видео"), M_VNEXT, 1, Some(Ic::Next));
                        c.btn(x + b1 + p(8.0), y, tr("Папка с видео"), M_VDIR, 0, Some(Ic::Folder));
                        y + p(38.0)
                    });
                    y = c.toggle(x, y, w, tr("Менять видео вместе с обоями"), tr("С тем же интервалом, что и обычные обои (раздел «Обои»)"), cfg.video_rotate, M_VROT);
                }
                if cfg.live != "off" {
                    y = c.toggle(x, y, w, tr("Замирать от батареи"), tr("На ноутбуке без зарядки живые обои не тратят заряд"), cfg.live_battery, M_LIVE_BAT);
                }
                content_h = y + st.scroll + p(20.0);
            }
            P_MONITORS => {
                let mons = clock::monitors();
                // схема расположения мониторов
                let vs = mons.iter().fold(mons[0].rect, |a, m| RECT { left: a.left.min(m.rect.left), top: a.top.min(m.rect.top), right: a.right.max(m.rect.right), bottom: a.bottom.max(m.rect.bottom) });
                let (vw, vh) = (((vs.right - vs.left) as f32).max(1.0), ((vs.bottom - vs.top) as f32).max(1.0));
                let box_h = p(240.0);
                let k = ((w - p(48.0)) / vw).min((box_h - p(48.0)) / vh);
                let ox = x + (w - vw * k) / 2.0;
                let oy = y + (box_h - vh * k) / 2.0;
                c.fill_rr(x, y, w, box_h, p(16.0), card());
                let mut tiles = Vec::new();
                for (i, m) in mons.iter().enumerate() {
                    let mx = ox + (m.rect.left - vs.left) as f32 * k + p(4.0);
                    let my = oy + (m.rect.top - vs.top) as f32 * k + p(4.0);
                    let mw = (m.rect.right - m.rect.left) as f32 * k - p(8.0);
                    let mh = (m.rect.bottom - m.rect.top) as f32 * k - p(8.0);
                    tiles.push((i, mx, my, mw, mh, *m));
                }
                for (i, mx, my, mw, mh, m) in &tiles {
                    if let Some(img) = get_thumb(&mut st.thumbs, &sn.cache, *i, *mw as i32, *mh as i32) {
                        c.image(img, *mx, *my, *mw, *mh, p(10.0));
                    }
                    let clock_here = clock::clock_targets(cfg, &mons).contains(i) && cfg.clock;
                    c.stroke_rr(*mx, *my, *mw, *mh, p(10.0), if clock_here { accent() } else { mon_frame() }, p(2.0));
                    let label = format!("{}{}  ·  {}×{}", i + 1, if m.primary { tr(" (основной)") } else { "" }, m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
                    let lw = c.text_w(&label, F::Small) + p(18.0);
                    c.fill_rr(*mx + p(8.0), *my + p(8.0), lw, p(24.0), p(12.0), argb(16, 16, 20));
                    c.text(*mx + p(8.0), *my + p(8.0), lw, p(24.0), &label, F::Small, rgb(236, 236, 241), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                y += box_h + p(10.0);
                c.text(x + p(4.0), y, w, p(20.0), tr("Цветная рамка — мониторы, на которых сейчас показываются часы"), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                y += p(30.0);
                if mons.len() > 1 {
                    let items: Vec<_> = CLOCK_MONS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_CLOCK_MON + i, cfg.clock_monitor == *v)).collect();
                    y = c.group(x, y, w, tr("Часы на мониторе"), "", &items);
                    let items: Vec<_> = WALL_MODES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_WALL_MODE + i, cfg.wall_mode == *v)).collect();
                    y = c.group(x, y, w, tr("Обои на мониторах"), tr("Единые — одна широкая картинка продолжается с экрана на экран"), &items);
                } else {
                    c.text(x + p(4.0), y, w, p(20.0), tr("Подключён один монитор — дополнительные настройки появятся, когда подключите второй."), F::Body, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    y += p(24.0);
                }
                content_h = y + st.scroll + p(20.0);
            }
            // ---------------- Прочее ----------------
            _ => {
                y = c.toggle(x, y, w, tr("Запускать вместе с Windows"), tr("Hora будет стартовать сама при входе в систему"), sn.autostart, M_AUTOSTART);
                let items: Vec<_> = UI_THEMES.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_UI_THEME + i, sn.cfg.ui_theme == *v)).collect();
                y = c.group(x, y, w, tr("Тема окна"), tr("«Как в Windows» — следует за светлым или тёмным режимом системы"), &items);
                let langs: Vec<_> = UI_LANGS.iter().enumerate().map(|(i, (v, t))| (tr(t).to_string(), M_UI_LANG + i, sn.cfg.ui_lang == *v)).collect();
                y = c.group(x, y, w, "Язык интерфейса · Language", tr("Язык окна и меню (часы — в разделе «Часы»)"), &langs);
                y = c.card(x, y, w, |c, x, y, w| {
                    let y = c.head(x, y, w, tr("Файлы"), "");
                    let b1 = c.btn(x, y, tr("Все настройки"), M_CONFIG, 0, Some(Ic::File));
                    let b2 = c.btn(x + b1 + p(8.0), y, tr("Журнал"), M_OPEN_LOG, 0, Some(Ic::List));
                    c.btn(x + b1 + b2 + p(16.0), y, tr("Папка программы"), M_OPEN_DIR, 0, Some(Ic::Folder));
                    y + p(38.0)
                });
                let mem = memory_mb();
                y = c.card(x, y, w, |c, x, y, w| {
                    c.text(x, y, w, p(22.0), &format!("Hora {}", env!("CARGO_PKG_VERSION")), F::H2, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    c.text(x, y + p(26.0), w, p(20.0), &(if crate::i18n::en() { format!("Memory now: {:.1} MB (with this window open; less after closing it)", mem) } else { format!("Память сейчас: {:.1} МБ (с открытым окном; после закрытия — меньше)", mem) }), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    c.text(x, y + p(48.0), w, p(20.0), tr("Обои: wallhaven.cc · Погода: open-meteo.com · Шрифты: Google Fonts (OFL)"), F::Small, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    y + p(70.0)
                });
                c.btn(x, y + p(4.0), tr("Выйти из Hora"), M_EXIT, 2, Some(Ic::Power));
                y += p(46.0);
                content_h = y + st.scroll + p(20.0);
            }
        }
    } else {
        c.text(cx0, p(28.0), p(400.0), p(36.0), tr("Загрузка…"), F::Title, t_sec(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    }

    // полоса прокрутки
    let view = ch as f32;
    if content_h > view {
        let track_h = view - p(16.0);
        let bar_h = (track_h * view / content_h).max(p(40.0));
        let bar_y = p(8.0) + (track_h - bar_h) * (st.scroll / (content_h - view)).clamp(0.0, 1.0);
        c.fill_rr(cw as f32 - p(10.0), bar_y, p(4.0), bar_h, p(2.0), scrollbar());
    }

    // боковое меню (рисуем последним — поверх прокручиваемого содержимого)
    c.fill_rr(-10.0, -10.0, side + 10.0, ch as f32 + 20.0, 0.0, sidebar());
    // логотип Hora: горы и солнце на светлой плитке (как значок программы)
    let (lx, ly) = (p(26.0), p(36.0));
    c.logo(lx, ly - p(16.0), p(32.0));
    c.text(lx + p(42.0), ly - p(16.0), p(150.0), p(32.0), "Hora", F::Logo, t_main(), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    let mut ny = p(84.0);
    for (i, name) in PAGES.iter().enumerate() {
        let id = NAV + i;
        let (nx, nw, nh) = (p(12.0), side - p(24.0), p(42.0));
        let active = st.page == i;
        let fill = if active { nav_active() } else { mix_argb(sidebar(), nav_hover(), c.hv(id)) };
        if active || c.hv(id) > 0.01 {
            c.fill_rr(nx, ny, nw, nh, nh / 2.0, fill);
        }
        if active {
            // метка выбранного раздела — короткая полоска цвета акцента слева
            c.fill_rr(nx + p(4.0), ny + nh * 0.3, p(3.0), nh * 0.4, p(1.5), accent());
        }
        let icol = if active { accent() } else { argb_of(t_sec()) };
        c.icon(PAGE_ICONS[i], nx + p(16.0), ny + (nh - p(20.0)) / 2.0, p(20.0), icol, fill);
        c.text(nx + p(48.0), ny, nw - p(52.0), nh, tr(name), F::Body, if active { t_main() } else { t_sec() }, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        c.hit(nx, ny, nw, nh, id);
        ny += nh + p(4.0);
    }
    // внизу — сколько памяти занимает программа (маленькая «таблетка»)
    let mem = if crate::i18n::en() { format!("Memory: {:.1} MB", memory_mb()) } else { format!("Память: {:.1} МБ", memory_mb()) };
    let mw = c.text_w(&mem, F::Small) + p(24.0);
    c.fill_rr(p(20.0), ch as f32 - p(48.0), mw, p(28.0), p(14.0), nav_hover());
    c.text(p(20.0), ch as f32 - p(48.0), mw, p(28.0), &mem, F::Small, t_sec(), DT_CENTER | DT_VCENTER | DT_SINGLELINE);

    // фигуры готовы — завершаем GDI+ и выводим текст обычным GDI (ClearType)
    (st.gp.del_graphics)(g);
    SetBkMode(mdc, TRANSPARENT as _);
    for t in &mut c.texts {
        SelectObject(mdc, c.fonts.get(t.f) as _);
        SetTextColor(mdc, t.color);
        DrawTextW(mdc, t.s.as_mut_ptr(), t.s.len() as i32, &mut t.r, t.flags | DT_NOPREFIX);
    }
    let slider_track = c.slider_track;
    let hits = std::mem::take(&mut c.hits);
    drop(c);
    st.hits = hits;
    st.edit_rect = st_edit_rect;
    if snap.is_some() {
        // без данных программы (она на миг занята) содержимое не рисовалось — прокрутку не трогаем
        st.content_h = content_h;
        st.slider_track = slider_track;
        let max = (content_h - ch as f32).max(0.0);
        if st.scroll > max || st.scroll_target > max {
            st.scroll = st.scroll.min(max);
            st.scroll_target = st.scroll_target.min(max);
            InvalidateRect(st.hwnd, std::ptr::null(), 0); // содержимое укоротилось — сразу без пустоты
        }
    }
    BitBlt(hdc, 0, 0, cw, ch, mdc, 0, 0, SRCCOPY);
}
