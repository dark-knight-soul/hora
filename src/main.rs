// Hora — лёгкая программа для Windows: меняет обои по таймеру и показывает большие часы
// прямо на рабочем столе. Живёт в трее, в простое почти не потребляет память и процессор.
#![windows_subsystem = "windows"]

mod bg;
mod clock;
mod config;
mod crash;
mod http;
mod i18n;
mod live;
mod setup;
mod ui;
mod update;
mod util;
mod wallpaper;
mod weather;

use i18n::tr;
use config::Config;
use std::cell::RefCell;
use std::path::PathBuf;
use util::{log, wide, wide_os};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::{CreateMutexW, INFINITE};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
const WM_WP_DONE: u32 = WM_APP + 2;
const WM_BG_DONE: u32 = WM_APP + 3;
const WM_OPEN_UI: u32 = WM_APP + 4;
const WM_UPDATE_CHECKED: u32 = WM_APP + 6; // проверка обновлений закончилась
const WM_UPDATE_READY: u32 = WM_APP + 7; // обновление скачано и проверено (или ошибка)
const WM_WEATHER_DONE: u32 = WM_APP + 5; // погода загружена (или ошибка) // открыть окно настроек (в том числе по просьбе второй копии программы)

const T_CLOCK: usize = 1;
const T_WALL: usize = 2;
const T_RELOAD: usize = 3;
const T_TRIM: usize = 4;
const T_BG: usize = 5;
const T_BG_RETRY: usize = 6;
const T_WEATHER: usize = 7;
const T_WEATHER_RETRY: usize = 8;

// загруженная погода ждёт здесь, если в момент прихода состояние программы было занято
static WEATHER_PENDING: std::sync::Mutex<Option<(f64, f64, Result<weather::Weather, String>)>> = std::sync::Mutex::new(None);
// результат смены обоев ждёт здесь, если в момент прихода состояние программы было занято
static WP_PENDING: std::sync::Mutex<Option<wallpaper::Outcome>> = std::sync::Mutex::new(None);
const T_WP_RETRY: usize = 9;
const T_SYS_RETRY: usize = 10;
const T_ZFIX: usize = 11; // вернуть часы под иконки, если Explorer положил поверх свой слой
const T_AFTER_WP: usize = 12; // повторная проверка часов после смены обоев
const T_UPDATE: usize = 14; // проверка обновлений (через полминуты после запуска, потом раз в сутки)
const T_LIVE_RETRY: usize = 13; // новости живых обоев не разобраны (состояние было занято) — ещё раз

// окно рабочего стола и наше главное окно — для обработчика системных событий
static HOST_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static MAIN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Windows сообщает: у какого-то окна Explorer поменялся порядок дочерних окон.
/// Если это рабочий стол — сразу проверяем, не закрыл ли слой обоев наши часы.
unsafe extern "system" fn on_reorder(_hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK, _event: u32, hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _time: u32) {
    if hwnd as isize != HOST_HWND.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if with_app(|a| a.clock.fix_z()).is_none() {
        // программа занята — проверим через мгновение
        SetTimer(MAIN_HWND.load(std::sync::atomic::Ordering::Relaxed) as HWND, T_ZFIX, 50, None);
    }
}

/// Подписка на перестановки окон в процессе Explorer (без опроса, система сама сообщает).
fn watch_desktop(host: HWND) {
    use windows_sys::Win32::UI::Accessibility::SetWinEventHook;
    static HOOK: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    static HOOK_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if host.is_null() {
        return;
    }
    HOST_HWND.store(host as isize, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(host, &mut pid);
        if pid == 0 || pid == HOOK_PID.load(std::sync::atomic::Ordering::Relaxed) {
            return; // уже следим за этим процессом Explorer
        }
        let old = HOOK.load(std::sync::atomic::Ordering::Relaxed);
        if old != 0 {
            windows_sys::Win32::UI::Accessibility::UnhookWinEvent(old as _);
        }
        // 0x8004 = EVENT_OBJECT_REORDER; 0 = WINEVENT_OUTOFCONTEXT (события приходят в наш цикл сообщений)
        let h = SetWinEventHook(0x8004, 0x8004, std::ptr::null_mut(), Some(on_reorder), pid, 0, 0);
        HOOK.store(h as isize, std::sync::atomic::Ordering::Relaxed);
        HOOK_PID.store(pid, std::sync::atomic::Ordering::Relaxed);
    }
}

// мониторы, чей фон обновился, пока состояние программы было занято (передаём при повторной попытке)
static BG_PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_NAME: &str = "Hora";
const OLD_APP_NAME: &str = "WallClock"; // прежнее название (запись автозапуска переносим)

// Пункты меню
const M_NEXT: usize = 100;
const M_PAUSE: usize = 101;
const M_SAVE: usize = 102;
const M_FAV: usize = 103; // в избранное / убрать из избранного
const M_BAN: usize = 104; // больше не показывать
const M_FAV_DIR: usize = 105; // открыть папку избранного
const M_BAN_OPEN: usize = 106; // открыть чёрный список
const M_BAN_UNDO: usize = 107; // вернуть последние скрытые
const M_FAV_MIX: usize = 800; // +индекс
const M_SCHED_ON: usize = 820; // расписание тем вкл/выкл
const M_SCHED: usize = 830; // + время суток·20 + номер категории (19 — «как основные»)
const SCHED_SAME: usize = 19;
// живые обои
const M_LIVE: usize = 920; // +индекс в LIVE_MODES
const LIVE_MODES: [(&str, &str); 3] = [("off", "Выключены"), ("effects", "Эффекты поверх обоев"), ("video", "Видео")];
const M_LIVE_FX: usize = 925; // +индекс в LIVE_EFFECTS
const LIVE_EFFECTS: [(&str, &str); 6] = [("snow", "Снег"), ("rain", "Дождь"), ("stars", "Звёзды"), ("leaves", "Листопад"), ("fireflies", "Светлячки"), ("weather", "По погоде")];
const M_LIVE_DENS: usize = 932; // +индекс в LIVE_DENSITY
const LIVE_DENSITY: [(u32, &str); 3] = [(1, "Мало"), (2, "Средне"), (3, "Много")];
const M_LIVE_BAT: usize = 936;
const M_VSRC: usize = 937; // +индекс в VIDEO_SOURCES
const VIDEO_SOURCES: [(&str, &str); 3] = [("both", "Всё"), ("folder", "Моя папка"), ("we", "Wallpaper Engine")];
const M_VROT: usize = 940;
const M_VDIR: usize = 941;
const M_VNEXT: usize = 942;
const M_VPICK: usize = 1200; // +номер видео в списке
const VPICK_MAX: usize = 200;
const FAV_MIXES: [(u32, &str); 3] = [(0, "Нет"), (20, "Иногда"), (50, "Часто")];
const M_INTERVAL: usize = 110; // +индекс
const M_SOURCE: usize = 130; // +индекс
const M_CLOCK_ON: usize = 200;
const M_STYLE: usize = 210; // +индекс
// после 11 стилей (210..=220) — отдельные номера, чтобы диапазоны не пересекались
const M_SECONDS: usize = 225;
const M_H24: usize = 226;
const M_SHADOW: usize = 227;
const M_LANG: usize = 230; // +индекс
const M_POS: usize = 240; // +индекс
const M_SCALE: usize = 260; // +индекс
const M_COLOR: usize = 400; // +индекс
const M_FONT: usize = 440; // +индекс
const M_FONT2: usize = 480; // +индекс
const M_OPACITY: usize = 520; // +индекс
const M_TOPIC_ANY: usize = 559;
const M_WALL_MODE: usize = 600; // +индекс
const WALL_MODES: [(&str, &str); 3] = [("same", "Одинаковые"), ("per_monitor", "Разные на каждом"), ("span", "Единые (одна картинка на все экраны)")];
const M_OPEN_LOG: usize = 700;
const M_OPEN_DIR: usize = 701;
const M_SETTINGS: usize = 702;
const M_REPORT: usize = 703; // сообщить о проблеме на GitHub
const M_UI_THEME: usize = 720; // +индекс
const M_UI_LANG: usize = 960; // +индекс в UI_LANGS
const M_UPDATE_NOW: usize = 970; // обновить до найденной версии
const M_UPDATE_CHECK: usize = 971; // проверить обновления сейчас
const M_UPDATE_AUTO: usize = 972; // проверять автоматически вкл/выкл
const M_UPDATE_PAGE: usize = 973; // страница выпуска («что нового»)
const UI_LANGS: [(&str, &str); 3] = [("auto", "Как в Windows"), ("ru", "Русский"), ("en", "English")];
const M_WEATHER_ON: usize = 740;
const M_WEATHER_REFRESH: usize = 741;
const M_WPOS: usize = 750; // +индекс в POSITIONS
const M_WMON: usize = 760; // +индекс в CLOCK_MONS
const M_WSCALE: usize = 770; // +индекс в SCALES
const M_BACKDROP: usize = 790; // +индекс
const BACKDROPS: [(&str, &str); 3] = [("off", "Нет"), ("on", "Всегда"), ("auto", "Авто (только на пёстром фоне)")];
const UI_THEMES: [(&str, &str); 3] = [("auto", "Как в Windows"), ("light", "Светлая"), ("dark", "Тёмная")];
const M_ANIM: usize = 630; // +индекс
const M_ANIM_SPEED: usize = 650; // +индекс
const ANIMS: [(&str, &str); 7] = [
    ("none", "Без анимации"),
    ("fade", "Растворение"),
    ("slide", "Сдвиг"),
    ("wipe", "Шторка"),
    ("circle", "Круг из центра"),
    ("blinds", "Жалюзи"),
    ("random", "Случайная"),
];
const ANIM_SPEEDS: [(u32, &str); 3] = [(450, "Быстро"), (900, "Обычно"), (1600, "Медленно")];
const M_CLOCK_MON: usize = 610; // +индекс
const CLOCK_MONS: [(&str, &str); 3] = [("primary", "Основной"), ("second", "Второй"), ("all", "На всех")];
const M_TOPIC: usize = 560; // +индекс
const M_CONFIG: usize = 300;
const M_FOLDER: usize = 301;
const M_AUTOSTART: usize = 302;
const M_EXIT: usize = 303;

const INTERVALS: [(u32, &str); 8] = [
    (0, "Только вручную"),
    (5, "5 минут"),
    (15, "15 минут"),
    (30, "30 минут"),
    (60, "1 час"),
    (180, "3 часа"),
    (720, "12 часов"),
    (1440, "1 день"),
];
const SOURCES: [(&str, &str); 4] = [("wallhaven", "Wallhaven (интернет)"), ("folder", "Моя папка"), ("both", "Wallhaven + моя папка"), ("favorites", "Только избранное")];
const STYLES: [(&str, &str); 12] = [
    ("hero", "Как на фото (день недели)"),
    ("photo", "Точно как на фото (буквы с разрывами)"),
    ("minimal", "Минимализм (большое время)"),
    ("split", "Время | дата"),
    ("vertical", "Вертикально"),
    ("words", "Словами («без четверти два»)"),
    ("calendar", "Календарь (большое число)"),
    ("outline", "Контур"),
    ("poster", "Постер (узкие жирные цифры)"),
    ("line", "Одной строкой"),
    ("techno", "Техно (цифры вразрядку)"),
    ("analog", "Стрелочные"),
];
const LANGS: [(&str, &str); 2] = [("ru", "Русский"), ("en", "English")];
const POSITIONS: [(&str, &str); 9] = [
    ("center", "По центру"),
    ("top", "Сверху"),
    ("bottom", "Снизу"),
    ("left", "Слева"),
    ("right", "Справа"),
    ("top-left", "Слева сверху"),
    ("top-right", "Справа сверху"),
    ("bottom-left", "Слева снизу"),
    ("bottom-right", "Справа снизу"),
];
const SCALES: [u32; 6] = [50, 75, 100, 125, 150, 200];
const COLORS: [(u32, &str); 14] = [
    (config::COLOR_AUTO, "Авто (по яркости обоев)"),
    (0xFFFFFF, "Белый"),
    (0xFFF1DC, "Тёплый белый"),
    (0xC8CCD2, "Светло-серый"),
    (0x2B2E33, "Графит"),
    (0x0A0A0A, "Чёрный"),
    (0x8CC8FF, "Голубой"),
    (0x4F8BFF, "Синий"),
    (0x8EF0C4, "Мятный"),
    (0xC4B2FF, "Лавандовый"),
    (0xFFB0C8, "Розовый"),
    (0xF5C96A, "Золотой"),
    (0xFF9F5A, "Оранжевый"),
    (0xFF5F5F, "Красный"),
];
// Шрифты крупного текста: (имя для config.ini, подпись в меню). Пустая строка — разделитель.
const FONTS: [(&str, &str); 24] = [
    ("auto", "Авто (под стиль)"),
    ("", ""),
    ("Michroma", "Michroma — как на фото (только латиница)"),
    ("WallClock Split", "WallClock Split — свой, с разрывами (рус/англ)"),
    ("Megrim", "Megrim — буквы с разрывами (только латиница)"),
    ("Unbounded", "Unbounded — широкий"),
    ("Jura", "Jura — тонкий техно"),
    ("Russo One", "Russo One — жирный спортивный"),
    ("Oswald", "Oswald — узкий"),
    ("Comfortaa", "Comfortaa — округлый"),
    ("Tektur", "Tektur — угловатый"),
    ("Exo 2", "Exo 2 — футуристичный"),
    ("Montserrat Alternates Light", "Montserrat Alternates — лёгкий"),
    ("Cormorant Garamond", "Cormorant — с засечками, изящный"),
    ("Press Start 2P", "Press Start 2P — пиксельный"),
    ("Hora Pixel", "Hora Pixel — точки, как на табло (рус/англ)"),
    ("Hora Block", "Hora Block — блочные пиксели, как в играх-песочницах (рус/англ)"),
    ("", ""),
    ("Bahnschrift", "Bahnschrift (Windows)"),
    ("Bahnschrift Condensed", "Bahnschrift Condensed (Windows)"),
    ("Segoe UI Light", "Segoe UI Light (Windows)"),
    ("Segoe UI Black", "Segoe UI Black (Windows)"),
    ("Georgia", "Georgia (Windows)"),
    ("Consolas", "Consolas (Windows)"),
];
// Шрифты подписей (дата, день недели)
const FONTS2: [(&str, &str); 11] = [
    ("auto", "Авто (Bahnschrift)"),
    ("Segoe UI", "Segoe UI"),
    ("Segoe UI Light", "Segoe UI Light"),
    ("Jura", "Jura"),
    ("Comfortaa", "Comfortaa"),
    ("Oswald", "Oswald"),
    ("Exo 2", "Exo 2"),
    ("Montserrat Alternates Light", "Montserrat Alternates"),
    ("Tektur", "Tektur"),
    ("Georgia", "Georgia"),
    ("Consolas", "Consolas"),
];
const OPACITIES: [u32; 5] = [100, 85, 70, 55, 40];

struct App {
    hwnd: HWND,
    cfg: Config,
    cfg_path: PathBuf,
    cfg_mtime: Option<std::time::SystemTime>,
    state_path: PathBuf,
    cache: PathBuf,
    clock: clock::Clock,
    busy: bool,
    bg_busy: bool,
    bg_again: bool,
    current: Vec<PathBuf>, // обои, стоящие сейчас (одни или по одним на монитор)
    last_change: u64,
    status: String,
    taskbar_created: u32,
    weather: Option<weather::Weather>,
    weather_busy: bool,
    weather_status: String, // последняя ошибка загрузки погоды
    after_wp_checks: u32,   // сколько проверок часов после смены обоев уже сделано
    fav_names: Vec<String>,    // что лежит в избранном (имена без расширения)
    banned: Vec<String>,       // чёрный список
    last_banned: Vec<String>,  // скрытые последним нажатием — чтобы можно было вернуть
    slot_hour: u32,            // час, для которого последний раз проверяли расписание тем
    live: live::Live,              // живые обои (эффекты и видео)
    videos: Vec<live::VideoItem>,  // найденные видео (своя папка и Wallpaper Engine)
    videos_at: Option<std::time::Instant>, // когда последний раз искали видео
    live_mode: live::Mode,         // что живые обои показывают на самом деле (с учётом погоды и наличия видео)
    banned_favs: Vec<PathBuf>,     // файлы, убранные из избранного последним «Не показывать» (для «Вернуть»)
    upd: UpdState,                 // обновления: что сейчас происходит
    upd_notified: String,          // о какой версии уже сообщили всплывающим уведомлением
}

/// Обновления: что сейчас происходит.
#[derive(Clone, PartialEq)]
enum UpdState {
    Idle,
    Checking,
    Latest,
    Available(update::Release),
    Downloading(String),
    Failed(String),
}

/// Текущий час (0..23) по местному времени.
fn local_hour() -> u32 {
    let mut st: clock::SysTime = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    st.wHour as u32
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Выполнить действие с состоянием программы (если оно сейчас не занято).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn mtime(p: &PathBuf) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

impl App {
    /// Перечитать избранное и чёрный список (их могли поправить руками в проводнике или блокноте).
    fn refresh_lists(&mut self) {
        self.fav_names = wallpaper::list_keys(&favorites_dir());
        match wallpaper::load_blacklist(&blacklist_path()) {
            Ok(b) => self.banned = b,
            Err(e) => log(&format!("blacklist.txt не прочитался ({e}) — оставляю прежний список")),
        }
    }

    fn is_fav(&self, p: &std::path::Path) -> bool {
        self.fav_names.contains(&wallpaper::key_of(p))
    }

    /// Все текущие обои в избранном?
    fn current_is_fav(&self) -> bool {
        !self.current.is_empty() && self.current.iter().all(|p| self.is_fav(p))
    }

    /// Убрать картинку из избранного. Если обои стоят прямо из папки избранного — файл
    /// переносим в кэш (а не удаляем), чтобы у Windows и у часов он остался на месте.
    fn drop_favorite(&mut self, cur: &std::path::Path) {
        for f in wallpaper::find_by_key(&favorites_dir(), &wallpaper::key_of(cur)) {
            if f == cur {
                let Some(name) = f.file_name() else { continue };
                let dst = self.cache.join(name);
                match std::fs::rename(&f, &dst) {
                    Ok(()) => {
                        for p in self.current.iter_mut() {
                            if *p == f {
                                *p = dst.clone();
                            }
                        }
                        self.save_state();
                    }
                    Err(e) => log(&format!("не удалось убрать из избранного {}: {e}", f.display())),
                }
            } else if let Err(e) = std::fs::remove_file(&f) {
                log(&format!("не удалось убрать из избранного {}: {e}", f.display()));
            }
        }
    }

    /// Раз в минуту (по тику часов): если началось другое время суток и у него другие категории —
    /// сразу ставим подходящие обои.
    fn check_schedule(&mut self) {
        let hour = local_hour();
        if hour == self.slot_hour {
            return;
        }
        // обои сейчас подбираются (или ставится видео) — проверим на следующем тике, смену не теряем
        if self.busy {
            return;
        }
        let before = self.cfg.topics_at(self.slot_hour).to_string();
        let was = self.cfg.slot_at(self.slot_hour);
        self.slot_hour = hour;
        let now = self.cfg.slot_at(hour);
        if !self.cfg.schedule || self.cfg.paused || now == was || self.cfg.topics_at(hour) == before {
            return;
        }
        // темы — это картинки с Wallhaven; видео и «своя папка»/«избранное» расписание не трогает
        if matches!(self.live_mode, live::Mode::Video(_)) || matches!(self.cfg.source.as_str(), "folder" | "favorites") {
            return;
        }
        log(&format!("расписание тем: {} — ставлю подходящие обои", config::SLOTS[now]));
        self.start_change();
    }

    /// Настройки для часов. Поверх видео нельзя заранее узнать яркость фона и размыть его под
    /// подложкой, поэтому там «авто»-цвет — белый с тёмным ореолом, а подложка выключена.
    fn clock_cfg(&self) -> Config {
        let mut c = self.cfg.clone();
        if matches!(self.live_mode, live::Mode::Video(_)) {
            if c.color == config::COLOR_AUTO {
                c.color = 0xFFFFFF;
                c.halo = true;
                c.shadow = true;
            }
            c.backdrop = "off".into();
        }
        c
    }

    fn video_folder(&self) -> PathBuf {
        if self.cfg.video_folder.trim().is_empty() {
            util::app_dir().join("videos")
        } else {
            PathBuf::from(self.cfg.video_folder.trim())
        }
    }

    /// Перечитать список видео (своя папка и обои Wallpaper Engine).
    fn refresh_videos(&mut self, force: bool) {
        if !force && self.videos_at.map(|t| t.elapsed() < std::time::Duration::from_secs(30)).unwrap_or(false) {
            return;
        }
        let src = self.cfg.video_source.as_str();
        self.videos = live::scan(&self.video_folder(), src != "we", src != "folder");
        self.videos_at = Some(std::time::Instant::now());
    }

    /// Эффект с учётом «по погоде»: снегопад — снег, дождь и гроза — дождь, ясная ночь — звёзды.
    fn live_effect(&self) -> live::Effect {
        match self.cfg.live_effect.as_str() {
            "rain" => live::Effect::Rain,
            "stars" => live::Effect::Stars,
            "leaves" => live::Effect::Leaves,
            "fireflies" => live::Effect::Fireflies,
            "weather" => match &self.weather {
                Some(w) => match w.code {
                    71..=77 | 85 | 86 => live::Effect::Snow,
                    51..=67 | 80..=82 | 95..=99 => live::Effect::Rain,
                    0..=2 if !w.is_day => live::Effect::Stars,
                    _ => live::Effect::None,
                },
                None => live::Effect::None,
            },
            _ => live::Effect::Snow,
        }
    }

    /// Что живые обои будут показывать на самом деле: «по погоде» в ясный день — ничего (процесс не нужен),
    /// видео — только если файл есть (иначе обычные обои).
    fn resolve_live(&mut self) -> live::Mode {
        match self.cfg.live.as_str() {
            "effects" => match self.live_effect() {
                live::Effect::None => live::Mode::Off,
                e => live::Mode::Effects(e),
            },
            "video" => {
                // видео ещё не выбрано или файл пропал — берём первое существующее из найденных
                if !PathBuf::from(self.cfg.video.trim()).is_file() {
                    self.refresh_videos(false);
                    if let Some(v) = self.videos.iter().find(|v| v.path.is_file()) {
                        let p = v.path.to_string_lossy().into_owned();
                        if p != self.cfg.video {
                            self.cfg.video = p;
                            self.save_cfg();
                        }
                    }
                }
                let p = PathBuf::from(self.cfg.video.trim());
                if p.is_file() {
                    live::Mode::Video(p)
                } else {
                    live::Mode::Off
                }
            }
            _ => live::Mode::Off,
        }
    }

    /// Передать живым обоям настройки и окна мониторов (вызывается после каждой перерисовки часов).
    fn sync_live(&mut self) {
        let mode = self.live_mode.clone();
        let video = matches!(mode, live::Mode::Video(_));
        let faces = if mode == live::Mode::Off { Vec::new() } else { self.clock.live_faces(video) };
        let settings = live::Settings { mode, density: self.cfg.live_density, span: self.cfg.wall_mode == "span", pause_battery: self.cfg.live_battery };
        self.live.update(settings, faces);
    }

    /// Следующее видео (случайное из найденных, но не то же самое).
    fn next_video(&mut self) {
        self.refresh_videos(true);
        if self.videos.is_empty() {
            self.status = tr("нет видео: положите mp4 в папку videos или подпишитесь на видеообои в Wallpaper Engine").into();
            log(&self.status);
            // следующий круг всё равно назначаем — вдруг видео появятся
            self.last_change = util::now_unix();
            self.schedule_wallpaper();
            ui::refresh();
            return;
        }
        let cur = PathBuf::from(self.cfg.video.trim());
        let others: Vec<usize> = (0..self.videos.len()).filter(|&i| self.videos[i].path != cur).collect();
        let i = if others.is_empty() { 0 } else { others[util::Rng::new().below(others.len())] };
        let v = self.videos[i].clone();
        self.cfg.video = v.path.to_string_lossy().into_owned();
        self.save_cfg();
        self.status = if crate::i18n::en() { format!("video: {}", v.title) } else { format!("видео: {}", v.title) };
        self.last_change = util::now_unix();
        self.save_state();
        self.schedule_wallpaper();
        self.tick_clock(); // режим, часы поверх видео и сами живые обои — заново
    }

    fn folder(&self) -> PathBuf {
        if self.cfg.folder.trim().is_empty() {
            util::app_dir().join("wallpapers")
        } else {
            PathBuf::from(self.cfg.folder.trim())
        }
    }

    fn save_cfg(&mut self) {
        if let Err(e) = self.cfg.save(&self.cfg_path) {
            log(&format!("не удалось сохранить настройки: {e}"));
        }
        self.cfg_mtime = mtime(&self.cfg_path);
    }

    fn save_state(&self) {
        let mut s = format!("last={}\n", self.last_change);
        for p in &self.current {
            s += &format!("current={}\n", p.display());
        }
        let _ = std::fs::write(&self.state_path, s);
    }

    fn load_state(&mut self) {
        if let Ok(t) = std::fs::read_to_string(&self.state_path) {
            for l in t.lines() {
                if let Some(v) = l.strip_prefix("last=") {
                    self.last_change = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = l.strip_prefix("current=") {
                    let p = PathBuf::from(v.trim());
                    if !v.trim().is_empty() && p.exists() {
                        self.current.push(p);
                    }
                }
            }
        }
    }

    // ---- часы ----
    fn tick_clock(&mut self) {
        unsafe { KillTimer(self.hwnd, T_CLOCK) };
        self.live_mode = self.resolve_live();
        self.clock.all_monitors = self.live_mode != live::Mode::Off;
        let cfg = self.clock_cfg();
        self.clock.render(&cfg, self.weather.as_ref());
        self.sync_live();
        watch_desktop(self.clock.host_window());
        unsafe { SetTimer(self.hwnd, T_CLOCK, clock::ms_to_next_tick(&self.cfg), None) };
        ui::refresh();
        if !self.cfg.seconds && !ui::is_open() {
            util::trim_memory();
        }
    }

    // ---- погода ----
    fn weather_ready(&self) -> bool {
        self.cfg.weather && !self.cfg.weather_city.is_empty()
    }

    fn start_weather(&mut self) {
        unsafe { KillTimer(self.hwnd, T_WEATHER) };
        if !self.weather_ready() || self.weather_busy {
            return;
        }
        self.weather_busy = true;
        let (lat, lon) = (self.cfg.weather_lat, self.cfg.weather_lon);
        let hwnd = self.hwnd as usize;
        let spawned = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let r = util::parcel_new((lat, lon, weather::fetch(lat, lon)));
            unsafe {
                if PostMessageW(hwnd as HWND, WM_WEATHER_DONE, 0, r) == 0 {
                    drop(util::parcel_take::<(f64, f64, Result<weather::Weather, String>)>(r)); // окно уже закрыто
                }
            }
        });
        if spawned.is_err() {
            self.weather_busy = false;
        }
    }

    fn weather_done(&mut self, lat: f64, lon: f64, r: Result<weather::Weather, String>) {
        self.weather_busy = false;
        // пока грузили, город сменили — этот ответ уже не нужен, грузим заново
        if lat != self.cfg.weather_lat || lon != self.cfg.weather_lon {
            self.start_weather();
            return;
        }
        let next_ms = match r {
            Ok(w) => {
                self.weather = Some(w);
                self.weather_status.clear();
                30 * 60 * 1000 // обновляем раз в полчаса
            }
            Err(e) => {
                log(&format!("погода не загрузилась: {e}"));
                self.weather_status = e;
                10 * 60 * 1000 // при ошибке — повтор через 10 минут
            }
        };
        if self.weather_ready() {
            unsafe { SetTimer(self.hwnd, T_WEATHER, next_ms, None) };
        }
        self.tick_clock();
        unsafe { SetTimer(self.hwnd, T_TRIM, 2000, None) };
    }

    // ---- обои ----
    fn schedule_wallpaper(&mut self) {
        unsafe { KillTimer(self.hwnd, T_WALL) };
        if self.cfg.paused || self.cfg.interval_min == 0 {
            return;
        }
        let period = self.cfg.interval_min as u64 * 60;
        let elapsed = util::now_unix().saturating_sub(self.last_change);
        let wait = if elapsed >= period { 2 } else { period - elapsed };
        unsafe { SetTimer(self.hwnd, T_WALL, (wait.min(24 * 3600) * 1000) as u32, None) };
    }

    fn start_change(&mut self) {
        if self.busy {
            return;
        }
        // в режиме видео картинки не меняем (их всё равно не видно) — меняем само видео
        if self.cfg.live == "video" {
            self.next_video();
            return;
        }
        self.busy = true;
        self.set_tip(tr("Hora — подбираю обои…"));
        let mut cfg = self.cfg.clone();
        // расписание тем: вместо основных категорий — категории этого времени суток
        cfg.topics = self.cfg.topics_at(local_hour()).to_string();
        self.refresh_lists();
        let dirs = wallpaper::Dirs { cache: self.cache.clone(), folder: self.folder(), favorites: favorites_dir(), banned: self.banned.clone() };
        let current = self.current.clone();
        let (mon, _) = clock::primary_monitor();
        let screen = (mon.right - mon.left, mon.bottom - mon.top);
        let hwnd = self.hwnd as usize;
        let spawned = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let out = wallpaper::change(&cfg, &dirs, &current, screen);
            let ptr = util::parcel_new(out);
            unsafe {
                if PostMessageW(hwnd as HWND, WM_WP_DONE, 0, ptr) == 0 {
                    drop(util::parcel_take::<wallpaper::Outcome>(ptr));
                }
            }
        });
        if spawned.is_err() {
            self.busy = false;
        }
    }

    fn change_done(&mut self, out: wallpaper::Outcome) {
        self.busy = false;
        unsafe { SetTimer(self.hwnd, T_AFTER_WP, 1500, None) };
        match out.paths.first() {
            Some(_) => {
                self.current = out.paths.clone();
                self.last_change = util::now_unix();
                let name = out.paths.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
                self.status = name.clone();
                self.set_tip(&(if crate::i18n::en() { format!("Hora\nWallpaper: {name}") } else { format!("Hora\nОбои: {name}") }));
                if let Some(e) = &out.error {
                    log(&format!("обои поставлены из запасного источника: {e}"));
                }
                self.save_state();
                self.schedule_wallpaper();
            }
            None => {
                let e = out.error.unwrap_or_else(|| tr("неизвестная ошибка").into());
                log(&format!("не удалось сменить обои: {e}"));
                self.status = if crate::i18n::en() { format!("error: {e}") } else { format!("ошибка: {e}") };
                self.set_tip(&(if crate::i18n::en() { format!("Hora\nCould not change the wallpaper: {e}") } else { format!("Hora\nНе удалось сменить обои: {e}") }));
                // повторим через 5 минут (или раньше, если интервал короче)
                if !self.cfg.paused && self.cfg.interval_min > 0 {
                    let retry = (self.cfg.interval_min.min(5) * 60 * 1000).max(60_000);
                    unsafe { SetTimer(self.hwnd, T_WALL, retry, None) };
                }
            }
        }
        // фон под часами — под новые обои; часы могли «потеряться», если Explorer пересоздал слой обоев
        self.start_bg();
        self.tick_clock();
        unsafe { SetTimer(self.hwnd, T_TRIM, 3000, None) };
    }

    // ---- фон под часами (копия обоев, масштабированная под экран) ----
    fn request_bg(&mut self) {
        unsafe { SetTimer(self.hwnd, T_BG, 700, None) };
    }

    fn start_bg(&mut self) {
        if self.bg_busy {
            self.bg_again = true;
            return;
        }
        self.bg_busy = true;
        let cache = self.cache.clone();
        let rects: Vec<RECT> = clock::monitors().iter().map(|m| m.rect).collect();
        let hwnd = self.hwnd as usize;
        let spawned = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let changed = bg::build(&cache, &rects).unwrap_or(0);
            unsafe { PostMessageW(hwnd as HWND, WM_BG_DONE, changed, 0) };
        });
        if spawned.is_err() {
            self.bg_busy = false;
        }
    }

    fn bg_done(&mut self, mask: usize) {
        self.bg_busy = false;
        if mask != 0 {
            // поверх видео накладка со старой картинкой только мелькнула бы — без анимации
            let anim = if matches!(self.live_mode, live::Mode::Video(_)) { None } else { clock::Anim::from_cfg(&self.cfg.anim).map(|k| (k, self.cfg.anim_ms)) };
            self.clock.bg_updated(mask, anim);
            ui::bg_changed();
        }
        self.tick_clock();
        if self.bg_again {
            self.bg_again = false;
            self.request_bg();
        }
        unsafe { SetTimer(self.hwnd, T_TRIM, 2000, None) };
    }

    // ---- трей ----
    fn tray(&self, msg: NOTIFY_ICON_MESSAGE, tip: &str) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = LoadIconW(GetModuleHandleW(std::ptr::null()), 1 as _);
            util::copy_to_buf(&mut nid.szTip, tip);
            Shell_NotifyIconW(msg, &nid);
        }
    }

    /// Всплывающее уведомление у значка в трее.
    fn balloon(&self, title: &str, text: &str) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_INFO;
            nid.dwInfoFlags = NIIF_INFO;
            util::copy_to_buf(&mut nid.szInfoTitle, title);
            util::copy_to_buf(&mut nid.szInfo, text);
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    // ---- обновления ----
    /// Спросить GitHub о новой версии (в фоне). manual — нажали кнопку (ошибку тогда показываем).
    fn start_update_check(&mut self, manual: bool) {
        if matches!(self.upd, UpdState::Checking | UpdState::Downloading(_)) {
            return;
        }
        self.upd = UpdState::Checking;
        let hwnd = self.hwnd as usize;
        let ok = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let r = util::parcel_new(update::check());
            unsafe {
                if PostMessageW(hwnd as HWND, WM_UPDATE_CHECKED, manual as usize, r) == 0 {
                    drop(util::parcel_take::<Result<Option<update::Release>, String>>(r));
                }
            }
        });
        if ok.is_err() {
            self.upd = UpdState::Idle;
        }
        ui::refresh();
    }

    fn update_checked(&mut self, manual: bool, r: Result<Option<update::Release>, String>) {
        self.upd = match r {
            Ok(Some(rel)) => {
                log(&format!("обновления: доступна версия {}", rel.version));
                // о новой версии сообщаем один раз (при ручной проверке её и так видно в окне)
                if !manual && self.upd_notified != rel.version {
                    self.upd_notified = rel.version.clone();
                    let text = if i18n::en() { format!("Version {}. Click to open the settings and update.", rel.version) } else { format!("Версия {}. Нажмите, чтобы открыть настройки и обновить.", rel.version) };
                    CRASH_BALLOON.store(false, std::sync::atomic::Ordering::Relaxed);
                    self.balloon(tr("Доступна новая версия Hora"), &text);
                }
                UpdState::Available(rel)
            }
            Ok(None) => UpdState::Latest,
            Err(e) => {
                log(&format!("обновления: не удалось проверить: {e}"));
                if manual {
                    UpdState::Failed(e)
                } else {
                    UpdState::Idle
                }
            }
        };
        ui::refresh();
    }

    /// Скачать найденную версию, проверить подпись и поставить её.
    fn start_update_install(&mut self) {
        let UpdState::Available(rel) = self.upd.clone() else { return };
        self.upd = UpdState::Downloading(rel.version.clone());
        let hwnd = self.hwnd as usize;
        let ok = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let rel = update::freshest(rel);
            let r = util::parcel_new(update::download(&rel));
            unsafe {
                if PostMessageW(hwnd as HWND, WM_UPDATE_READY, 0, r) == 0 {
                    drop(util::parcel_take::<Result<PathBuf, String>>(r));
                }
            }
        });
        if ok.is_err() {
            self.upd = UpdState::Failed(tr("неизвестная ошибка").into());
        }
        ui::refresh();
    }

    /// Обновление скачано и проверено: запускаем замену и закрываемся (новая версия запустится сама).
    fn update_ready(&mut self, r: Result<PathBuf, String>) -> bool {
        match r.and_then(|exe| update::start(&exe)) {
            Ok(()) => {
                log("обновления: новая версия проверена, устанавливаю и перезапускаюсь");
                true
            }
            Err(e) => {
                log(&format!("обновления: не удалось обновить: {e}"));
                self.upd = UpdState::Failed(e);
                ui::refresh();
                false
            }
        }
    }

    /// Строка состояния обновлений для окна: (текст, найденная версия, идёт работа).
    fn update_view(&self) -> (String, Option<String>, bool) {
        let en = i18n::en();
        match &self.upd {
            UpdState::Idle => (tr("Обновления ещё не проверялись").into(), None, false),
            UpdState::Checking => (tr("Проверяю обновления…").into(), None, true),
            UpdState::Latest => (tr("У вас последняя версия").into(), None, false),
            UpdState::Available(r) => (if en { format!("Version {} is available", r.version) } else { format!("Доступна версия {}", r.version) }, Some(r.version.clone()), false),
            UpdState::Downloading(v) => (if en { format!("Downloading and checking version {v}…") } else { format!("Скачиваю и проверяю версию {v}…") }, None, true),
            UpdState::Failed(e) => (if en { format!("Update error: {e}") } else { format!("Ошибка обновления: {e}") }, None, false),
        }
    }

    fn set_tip(&self, tip: &str) {
        self.tray(NIM_MODIFY, tip);
    }

    // ---- применение изменённых настроек ----
    fn apply_cfg(&mut self, old: &Config) {
        let c = self.cfg.clone();
        if c.ui_lang != old.ui_lang {
            i18n::set_lang(&c.ui_lang); // окно и меню сразу на новом языке (окно перерисуется ниже)
        }
        if c.update_check != old.update_check {
            unsafe {
                if c.update_check {
                    SetTimer(self.hwnd, T_UPDATE, 5_000, None);
                } else {
                    KillTimer(self.hwnd, T_UPDATE);
                }
            }
        }
        if c.interval_min != old.interval_min || c.paused != old.paused {
            self.schedule_wallpaper();
        }
        if c.fit != old.fit {
            let fit = c.fit.clone();
            std::thread::spawn(move || wallpaper::set_fit(&fit));
        }
        let wall_keys_changed = c.source != old.source
            || c.query != old.query
            || c.topics != old.topics
            || c.wall_mode != old.wall_mode
            || c.sorting != old.sorting
            || c.top_range != old.top_range
            || c.categories != old.categories
            || c.folder != old.folder;
        if c.live != old.live || c.video_source != old.video_source || c.video_folder != old.video_folder {
            self.refresh_videos(true);
        }
        // [13] расписание включили или поменяли темы текущего времени суток — сразу подходящие обои
        let hour = local_hour();
        let topics_now_changed = c.topics_at(hour) != old.topics_at(hour) && c.topics == old.topics;
        if (wall_keys_changed || topics_now_changed) && c.live != "video" {
            let _ = std::fs::create_dir_all(self.folder());
            self.start_change();
        }
        let weather_changed = c.weather != old.weather || c.weather_lat != old.weather_lat || c.weather_lon != old.weather_lon || c.weather_city != old.weather_city;
        if weather_changed {
            if c.weather_lat != old.weather_lat || c.weather_lon != old.weather_lon {
                self.weather = None; // старый город больше не показываем
            }
            self.start_weather();
        }
        self.tick_clock();
    }

    fn reload_if_changed(&mut self) {
        let m = mtime(&self.cfg_path);
        if m.is_none() || m == self.cfg_mtime {
            return;
        }
        static FAILS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let Some(new) = Config::try_load(&self.cfg_path) else {
            let n = FAILS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n == 0 {
                log("config.ini сейчас не читается (занят или пуст) — оставляю прежние настройки");
            }
            if n < 10 {
                unsafe { SetTimer(self.hwnd, T_RELOAD, 1000, None) };
            }
            return;
        };
        FAILS.store(0, std::sync::atomic::Ordering::Relaxed);
        self.cfg_mtime = m;
        let old = self.cfg.clone();
        self.cfg = new;
        if self.cfg != old {
            log("настройки перечитаны из файла");
            self.apply_cfg(&old);
        }
    }
}

// ---------------- избранное и чёрный список ----------------

fn favorites_dir() -> PathBuf {
    util::app_dir().join("favorites")
}

fn blacklist_path() -> PathBuf {
    util::app_dir().join("blacklist.txt")
}

// ---------------- автозапуск ----------------

fn autostart_enabled() -> bool {
    unsafe {
        let mut size: u32 = 0;
        RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0
    }
}

/// Программа раньше называлась WallClock: запись автозапуска под старым именем переносим
/// на новое, а путь к программе обновляем (её могли переложить в другую папку).
fn migrate_autostart() {
    unsafe {
        let mut size: u32 = 0;
        let old = RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(OLD_APP_NAME).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0;
        if old {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(OLD_APP_NAME).as_ptr());
            log("автозапуск перенесён: WallClock → Hora");
        }
        let missing = autostart_target().map(|p| !p.is_file()).unwrap_or(false);
        if old || missing {
            set_autostart(true);
        }
    }
}

/// Куда указывает запись автозапуска (путь к exe без кавычек).
fn autostart_target() -> Option<PathBuf> {
    unsafe {
        let mut size: u32 = 0;
        if RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size) != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..n]);
        Some(PathBuf::from(s.trim().trim_matches('"')))
    }
}

fn set_autostart(on: bool) {
    unsafe {
        if on {
            let exe = std::env::current_exe().unwrap_or_default();
            let val = wide(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr(), REG_SZ, val.as_ptr() as _, (val.len() * 2) as u32);
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(APP_NAME).as_ptr());
        }
    }
}

// ---------------- меню в трее ----------------

fn add(menu: HMENU, id: usize, text: &str, checked: bool) {
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe { AppendMenuW(menu, flags, id, wide(text).as_ptr()) };
}
fn sep(menu: HMENU) {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
}
fn sub(menu: HMENU, child: HMENU, text: &str) {
    unsafe { AppendMenuW(menu, MF_POPUP, child as usize, wide(text).as_ptr()) };
}

/// Квадратик-образец цвета для пункта меню. Возвращает картинку, её нужно удалить после закрытия меню.
fn swatch(menu: HMENU, id: usize, color: u32) -> windows_sys::Win32::Graphics::Gdi::HBITMAP {
    use windows_sys::Win32::Graphics::Gdi::*;
    unsafe {
        let n = 16i32;
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = n;
        bi.bmiHeader.biHeight = -n;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(std::ptr::null_mut(), &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() {
            return bmp;
        }
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (n * n) as usize);
        for y in 0..n {
            for x in 0..n {
                let edge = x == 1 || y == 1 || x == n - 2 || y == n - 2;
                let inside = x >= 1 && y >= 1 && x <= n - 2 && y <= n - 2;
                let c = if color == config::COLOR_AUTO {
                    // «авто»: половина белая, половина тёмная
                    if x + y < n { 0xFFFFFF } else { 0x1C1E22 }
                } else {
                    color
                };
                px[(y * n + x) as usize] = if !inside { 0 } else if edge { 0xFF80_8080 } else { 0xFF00_0000 | c };
            }
        }
        let mut mi: MENUITEMINFOW = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        mi.fMask = MIIM_BITMAP;
        mi.hbmpItem = bmp;
        SetMenuItemInfoW(menu, id as u32, 0, &mi);
        bmp
    }
}

fn build_menu(a: &App, bitmaps: &mut Vec<windows_sys::Win32::Graphics::Gdi::HBITMAP>) -> HMENU {
    let c = &a.cfg;
    unsafe {
        let m = CreatePopupMenu();
        add(m, M_SETTINGS, tr("Открыть Hora"), false);
        SetMenuDefaultItem(m, M_SETTINGS as u32, 0);
        sep(m);
        if let UpdState::Available(r) = &a.upd {
            let s = if i18n::en() { format!("Update to {}", r.version) } else { format!("Обновить до {}", r.version) };
            add(m, M_UPDATE_NOW, &s, false);
            sep(m);
        }
        add(m, M_NEXT, tr("Следующие обои"), false);
        let fav = a.current_is_fav();
        add(m, M_FAV, if fav { tr("В избранном (нажмите, чтобы убрать)") } else { tr("В избранное") }, fav);
        add(m, M_BAN, tr("Больше не показывать эти обои"), false);
        if !a.last_banned.is_empty() {
            add(m, M_BAN_UNDO, tr("Вернуть последние скрытые"), false);
        }
        add(m, M_PAUSE, tr("Пауза автосмены"), c.paused);

        let mi = CreatePopupMenu();
        for (i, (v, t)) in INTERVALS.iter().enumerate() {
            add(mi, M_INTERVAL + i, tr(t), c.interval_min == *v);
        }
        sub(m, mi, tr("Менять каждые"));

        let ms = CreatePopupMenu();
        for (i, (v, t)) in SOURCES.iter().enumerate() {
            add(ms, M_SOURCE + i, tr(t), c.source == *v);
        }
        sub(m, ms, tr("Откуда брать обои"));
        let mfx = CreatePopupMenu();
        for (i, (v, t)) in FAV_MIXES.iter().enumerate() {
            add(mfx, M_FAV_MIX + i, tr(t), c.favorites_mix == *v);
        }
        sep(mfx);
        add(mfx, M_FAV_DIR, tr("Открыть папку избранного"), false);
        add(mfx, M_BAN_OPEN, tr("Открыть список скрытых"), false);
        sub(m, mfx, tr("Показывать избранное"));
        let ml = CreatePopupMenu();
        for (i, (v, t)) in LIVE_MODES.iter().enumerate() {
            add(ml, M_LIVE + i, tr(t), c.live == *v);
        }
        sep(ml);
        for (i, (v, t)) in LIVE_EFFECTS.iter().enumerate() {
            add(ml, M_LIVE_FX + i, tr(t), c.live == "effects" && c.live_effect == *v);
        }
        sep(ml);
        add(ml, M_VNEXT, tr("Следующее видео"), false);
        add(ml, M_VDIR, tr("Открыть папку с видео"), false);
        sub(m, ml, tr("Живые обои"));

        // категории: можно отметить несколько — каждый раз берётся случайная из отмеченных
        let mt = CreatePopupMenu();
        let chosen: Vec<&str> = c.topics.split(',').filter(|s| !s.is_empty()).collect();
        add(mt, M_TOPIC_ANY, tr("Любые (популярное)"), chosen.is_empty() && c.query.trim().is_empty());
        if chosen.is_empty() && !c.query.trim().is_empty() {
            let s: String = (if crate::i18n::en() { format!("Custom query: {}", c.query.trim()) } else { format!("Свой запрос: {}", c.query.trim()) }).chars().take(50).collect();
            add(mt, 0, &s, true);
            windows_sys::Win32::UI::WindowsAndMessaging::EnableMenuItem(mt, 0, MF_BYCOMMAND | MF_GRAYED);
        }
        sep(mt);
        for (i, t) in wallpaper::TOPICS.iter().enumerate() {
            add(mt, M_TOPIC + i, tr(t.1), chosen.contains(&t.0));
        }
        sep(mt);
        add(mt, M_SCHED_ON, tr("По времени суток (расписание)"), c.schedule);
        let title = match chosen.len() {
            0 => tr("Категории обоев").to_string(),
            1 => if crate::i18n::en() { format!("Wallpaper categories: {}", wallpaper::selected_topics(&c.topics).first().map(|t| tr(t.1)).unwrap_or("")) } else { format!("Категории обоев: {}", wallpaper::selected_topics(&c.topics).first().map(|t| tr(t.1)).unwrap_or("")) },
            n => if crate::i18n::en() { format!("Wallpaper categories: {n} selected") } else { format!("Категории обоев: выбрано {n}") },
        };
        sub(m, mt, &title);
        let mon_count = clock::monitors().len();
        if mon_count > 1 {
            let mw = CreatePopupMenu();
            for (i, (v, t)) in WALL_MODES.iter().enumerate() {
                add(mw, M_WALL_MODE + i, tr(t), c.wall_mode == *v);
            }
            sub(m, mw, tr("Обои на мониторах"));
        }
        let ma = CreatePopupMenu();
        for (i, (v, t)) in ANIMS.iter().enumerate() {
            add(ma, M_ANIM + i, tr(t), c.anim == *v);
        }
        sep(ma);
        for (i, (v, t)) in ANIM_SPEEDS.iter().enumerate() {
            add(ma, M_ANIM_SPEED + i, tr(t), c.anim_ms == *v);
        }
        sub(m, ma, tr("Анимация смены"));
        add(m, M_SAVE, tr("Сохранить текущие в мою папку"), false);
        sep(m);

        let mc = CreatePopupMenu();
        add(mc, M_CLOCK_ON, tr("Показывать часы"), c.clock);
        if mon_count > 1 {
            let mm = CreatePopupMenu();
            for (i, (v, t)) in CLOCK_MONS.iter().enumerate() {
                add(mm, M_CLOCK_MON + i, tr(t), c.clock_monitor == *v);
            }
            sub(mc, mm, tr("На каком мониторе"));
        }
        sep(mc);
        for (i, (v, t)) in STYLES.iter().enumerate() {
            add(mc, M_STYLE + i, tr(t), c.style == *v);
        }
        sep(mc);
        add(mc, M_SECONDS, tr("Секунды"), c.seconds);
        add(mc, M_H24, tr("24-часовой формат"), c.is_24h());
        add(mc, M_SHADOW, tr("Тень под текстом"), c.shadow);
        let mb = CreatePopupMenu();
        for (i, (v, t)) in BACKDROPS.iter().enumerate() {
            add(mb, M_BACKDROP + i, tr(t), c.backdrop == *v);
        }
        sub(mc, mb, tr("Подложка (матовое стекло)"));
        sep(mc);
        for (i, (v, t)) in LANGS.iter().enumerate() {
            add(mc, M_LANG + i, tr(t), c.lang == *v);
        }
        sep(mc);
        let mp = CreatePopupMenu();
        for (i, (v, t)) in POSITIONS.iter().enumerate() {
            add(mp, M_POS + i, tr(t), c.position == *v && c.offset_x == 0 && (c.offset_y == 0 || (*v == "center" && c.offset_y == -4)));
        }
        sub(mc, mp, tr("Положение"));
        let mz = CreatePopupMenu();
        for (i, v) in SCALES.iter().enumerate() {
            add(mz, M_SCALE + i, &format!("{v}%"), c.scale == *v);
        }
        sub(mc, mz, tr("Размер"));
        sep(mc);

        let mcol = CreatePopupMenu();
        for (i, (v, t)) in COLORS.iter().enumerate() {
            add(mcol, M_COLOR + i, tr(t), c.color == *v);
            bitmaps.push(swatch(mcol, M_COLOR + i, *v));
            if i == 0 {
                sep(mcol);
            }
        }
        sub(mc, mcol, tr("Цвет"));
        let mo = CreatePopupMenu();
        for (i, v) in OPACITIES.iter().enumerate() {
            add(mo, M_OPACITY + i, &format!("{v}%"), c.opacity == *v);
        }
        sub(mc, mo, tr("Непрозрачность"));
        let mf = CreatePopupMenu();
        let cur = if c.font.is_empty() { "auto" } else { c.font.as_str() };
        for (i, (v, t)) in FONTS.iter().enumerate() {
            if v.is_empty() {
                sep(mf);
            } else {
                add(mf, M_FONT + i, tr(t), cur.eq_ignore_ascii_case(v));
            }
        }
        sub(mc, mf, tr("Шрифт времени"));
        let mf2 = CreatePopupMenu();
        let cur2 = if c.font2.is_empty() { "auto" } else { c.font2.as_str() };
        for (i, (v, t)) in FONTS2.iter().enumerate() {
            add(mf2, M_FONT2 + i, tr(t), cur2.eq_ignore_ascii_case(v));
        }
        sub(mc, mf2, tr("Шрифт подписей"));
        sub(m, mc, tr("Часы"));
        let mwx = CreatePopupMenu();
        add(mwx, M_WEATHER_ON, tr("Показывать погоду"), c.weather);
        add(mwx, M_WEATHER_REFRESH, tr("Обновить сейчас"), false);
        if c.weather_city.is_empty() {
            add(mwx, M_SETTINGS, tr("Выбрать город…"), false);
        }
        sub(m, mwx, if c.weather_city.is_empty() { tr("Погода").to_string() } else { if crate::i18n::en() { format!("Weather: {}", c.weather_city.split(',').next().unwrap_or("")) } else { format!("Погода: {}", c.weather_city.split(',').next().unwrap_or("")) } }.as_str());
        sep(m);

        let mlang = CreatePopupMenu();
        for (i, (v, t)) in UI_LANGS.iter().enumerate() {
            add(mlang, M_UI_LANG + i, tr(t), c.ui_lang == *v);
        }
        sub(m, mlang, "Язык / Language");
        add(m, M_UPDATE_CHECK, tr("Проверить обновления"), false);
        add(m, M_CONFIG, tr("Все настройки (config.ini)"), false);
        add(m, M_FOLDER, tr("Открыть мою папку с обоями"), false);
        add(m, M_AUTOSTART, tr("Запускать вместе с Windows"), autostart_enabled());
        if !a.status.is_empty() {
            sep(m);
            let s: String = a.status.chars().take(70).collect();
            AppendMenuW(m, MF_STRING | MF_GRAYED, 0, wide(&s).as_ptr());
        }
        sep(m);
        add(m, M_EXIT, tr("Выход"), false);
        m
    }
}

/// Что сделать после выбора пункта меню, когда состояние уже отпущено (открыть файл/папку).
enum After {
    Nothing,
    Open(PathBuf),
    Settings,
    Quit,
}

fn on_menu(a: &mut App, id: usize) -> After {
    let old = a.cfg.clone();
    match id {
        M_NEXT => {
            a.start_change();
            return After::Nothing;
        }
        M_PAUSE => a.cfg.paused = !a.cfg.paused,
        M_SAVE => {
            let dir = a.folder();
            let _ = std::fs::create_dir_all(&dir);
            for cur in &a.current {
                if let Some(name) = cur.file_name() {
                    let dst = dir.join(name);
                    if dst != *cur {
                        let _ = std::fs::copy(cur, dst);
                    }
                }
            }
            return After::Nothing;
        }
        M_FAV => {
            let dir = favorites_dir();
            if a.current_is_fav() {
                for p in a.current.clone() {
                    a.drop_favorite(&p);
                }
            } else {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    log(&format!("не удалось создать папку избранного: {e}"));
                }
                for p in a.current.clone() {
                    if a.is_fav(&p) {
                        continue;
                    }
                    let Some(name) = p.file_name() else { continue };
                    if let Err(e) = std::fs::copy(&p, dir.join(name)) {
                        log(&format!("не удалось добавить в избранное {}: {e}", p.display()));
                    }
                }
            }
            a.refresh_lists();
            ui::refresh();
            return After::Nothing;
        }
        M_BAN => {
            let keys: Vec<String> = a.current.iter().map(|p| wallpaper::key_of(p)).filter(|k| !k.is_empty()).collect();
            if keys.is_empty() {
                return After::Nothing;
            }
            let mut list = match wallpaper::load_blacklist(&blacklist_path()) {
                Ok(l) => l,
                Err(e) => {
                    log(&format!("blacklist.txt не прочитался ({e}) — не трогаю его"));
                    return After::Nothing;
                }
            };
            for k in &keys {
                if !list.contains(k) {
                    list.push(k.clone());
                }
            }
            if let Err(e) = wallpaper::save_blacklist(&blacklist_path(), &list) {
                log(&format!("не удалось записать чёрный список: {e}"));
                return After::Nothing;
            }
            // скрытое не должно всплывать и из избранного (запоминаем, что убрали, — для «Вернуть»)
            a.banned_favs.clear();
            for p in a.current.clone() {
                if a.is_fav(&p) {
                    a.drop_favorite(&p);
                    // после переноса в кэш путь в current уже новый
                    let key = wallpaper::key_of(&p);
                    if let Some(now) = a.current.iter().find(|c| wallpaper::key_of(c) == key) {
                        a.banned_favs.push(now.clone());
                    }
                }
            }
            log(&format!("больше не показывать: {}", keys.join(", ")));
            a.last_banned = keys;
            a.refresh_lists();
            if !matches!(a.live_mode, live::Mode::Video(_)) {
                a.start_change(); // сразу ставим другие обои (поверх видео их всё равно не видно)
            }
            ui::refresh();
            return After::Nothing;
        }
        M_BAN_UNDO => {
            let mut list = match wallpaper::load_blacklist(&blacklist_path()) {
                Ok(l) => l,
                Err(e) => {
                    log(&format!("blacklist.txt не прочитался ({e}) — не трогаю его"));
                    return After::Nothing;
                }
            };
            list.retain(|k| !a.last_banned.contains(k));
            if let Err(e) = wallpaper::save_blacklist(&blacklist_path(), &list) {
                log(&format!("не удалось записать чёрный список: {e}"));
                return After::Nothing;
            }
            a.last_banned.clear();
            // что было в избранном — возвращаем туда (если файл ещё в кэше)
            let fav = favorites_dir();
            for p in std::mem::take(&mut a.banned_favs) {
                if let Some(name) = p.file_name() {
                    if p.is_file() && std::fs::create_dir_all(&fav).is_ok() {
                        let _ = std::fs::copy(&p, fav.join(name));
                    }
                }
            }
            a.refresh_lists();
            ui::refresh();
            return After::Nothing;
        }
        M_FAV_DIR => {
            let dir = favorites_dir();
            let _ = std::fs::create_dir_all(&dir);
            return After::Open(dir);
        }
        M_BAN_OPEN => {
            let path = blacklist_path();
            if !path.exists() {
                let _ = wallpaper::save_blacklist(&path, &[]);
            }
            return After::Open(path);
        }
        x if (M_FAV_MIX..M_FAV_MIX + FAV_MIXES.len()).contains(&x) => a.cfg.favorites_mix = FAV_MIXES[x - M_FAV_MIX].0,
        M_SCHED_ON => a.cfg.schedule = !a.cfg.schedule,
        x if (M_LIVE..M_LIVE + LIVE_MODES.len()).contains(&x) => a.cfg.live = LIVE_MODES[x - M_LIVE].0.into(),
        x if (M_LIVE_FX..M_LIVE_FX + LIVE_EFFECTS.len()).contains(&x) => {
            // выбор эффекта из меню трея сразу включает эффекты
            a.cfg.live_effect = LIVE_EFFECTS[x - M_LIVE_FX].0.into();
            a.cfg.live = "effects".into();
        }
        x if (M_LIVE_DENS..M_LIVE_DENS + LIVE_DENSITY.len()).contains(&x) => a.cfg.live_density = LIVE_DENSITY[x - M_LIVE_DENS].0,
        M_LIVE_BAT => a.cfg.live_battery = !a.cfg.live_battery,
        x if (M_VSRC..M_VSRC + VIDEO_SOURCES.len()).contains(&x) => a.cfg.video_source = VIDEO_SOURCES[x - M_VSRC].0.into(),
        M_VROT => a.cfg.video_rotate = !a.cfg.video_rotate,
        M_VDIR => {
            let dir = a.video_folder();
            let _ = std::fs::create_dir_all(&dir);
            return After::Open(dir);
        }
        M_VNEXT => {
            if a.cfg.live != "video" {
                a.cfg.live = "video".into();
            }
            a.next_video();
            a.tick_clock();
            return After::Nothing;
        }
        x if (M_VPICK..M_VPICK + VPICK_MAX).contains(&x) => match a.videos.get(x - M_VPICK) {
            Some(v) => {
                a.cfg.video = v.path.to_string_lossy().into_owned();
                a.cfg.live = "video".into();
                a.status = if crate::i18n::en() { format!("video: {}", v.title) } else { format!("видео: {}", v.title) };
            }
            None => return After::Nothing,
        },
        x if (M_SCHED..M_SCHED + 4 * 20).contains(&x) => {
            let (slot, ti) = ((x - M_SCHED) / 20, (x - M_SCHED) % 20);
            if ti == SCHED_SAME {
                a.cfg.topics_slot[slot].clear();
            } else if let Some(t) = wallpaper::TOPICS.get(ti) {
                // отметить / снять категорию у этого времени суток
                let mut list: Vec<String> = a.cfg.topics_slot[slot].split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
                if let Some(i) = list.iter().position(|s| s == t.0) {
                    list.remove(i);
                } else {
                    list.push(t.0.to_string());
                }
                a.cfg.topics_slot[slot] = list.join(",");
            }
        }
        x if (M_WALL_MODE..M_WALL_MODE + WALL_MODES.len()).contains(&x) => a.cfg.wall_mode = WALL_MODES[x - M_WALL_MODE].0.into(),
        x if (M_ANIM..M_ANIM + ANIMS.len()).contains(&x) => a.cfg.anim = ANIMS[x - M_ANIM].0.into(),
        x if (M_ANIM_SPEED..M_ANIM_SPEED + ANIM_SPEEDS.len()).contains(&x) => a.cfg.anim_ms = ANIM_SPEEDS[x - M_ANIM_SPEED].0,
        x if (M_CLOCK_MON..M_CLOCK_MON + CLOCK_MONS.len()).contains(&x) => a.cfg.clock_monitor = CLOCK_MONS[x - M_CLOCK_MON].0.into(),
        x if (M_INTERVAL..M_INTERVAL + INTERVALS.len()).contains(&x) => {
            a.cfg.interval_min = INTERVALS[x - M_INTERVAL].0;
            // отсчёт нового интервала — с текущего момента
            a.last_change = util::now_unix();
            a.save_state();
        }
        x if (M_SOURCE..M_SOURCE + SOURCES.len()).contains(&x) => a.cfg.source = SOURCES[x - M_SOURCE].0.into(),
        M_TOPIC_ANY => {
            a.cfg.topics.clear();
            a.cfg.query.clear();
        }
        x if (M_TOPIC..M_TOPIC + wallpaper::TOPICS.len()).contains(&x) => {
            // отметить / снять категорию
            let id = wallpaper::TOPICS[x - M_TOPIC].0;
            let mut list: Vec<String> = a.cfg.topics.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
            if let Some(i) = list.iter().position(|s| s == id) {
                list.remove(i);
            } else {
                list.push(id.to_string());
            }
            a.cfg.topics = list.join(",");
            // категории — для интернета; если сейчас стоит «Моя папка», переключаем на Wallhaven
            if a.cfg.source == "folder" {
                a.cfg.source = "wallhaven".into();
            }
        }
        M_CLOCK_ON => a.cfg.clock = !a.cfg.clock,
        x if (M_STYLE..M_STYLE + STYLES.len()).contains(&x) => a.cfg.style = STYLES[x - M_STYLE].0.into(),
        M_SECONDS => a.cfg.seconds = !a.cfg.seconds,
        M_H24 => a.cfg.time_format = if a.cfg.is_24h() { "12".into() } else { "24".into() },
        M_SHADOW => a.cfg.shadow = !a.cfg.shadow,
        x if (M_LANG..M_LANG + LANGS.len()).contains(&x) => {
            a.cfg.lang = LANGS[x - M_LANG].0.into();
            a.cfg.time_format = "auto".into();
        }
        x if (M_POS..M_POS + POSITIONS.len()).contains(&x) => {
            a.cfg.position = POSITIONS[x - M_POS].0.into();
            a.cfg.offset_x = 0;
            a.cfg.offset_y = if a.cfg.position == "center" { -4 } else { 0 };
        }
        x if (M_SCALE..M_SCALE + SCALES.len()).contains(&x) => a.cfg.scale = SCALES[x - M_SCALE],
        x if (M_COLOR..M_COLOR + COLORS.len()).contains(&x) => a.cfg.color = COLORS[x - M_COLOR].0,
        x if (M_OPACITY..M_OPACITY + OPACITIES.len()).contains(&x) => a.cfg.opacity = OPACITIES[x - M_OPACITY],
        x if (M_FONT..M_FONT + FONTS.len()).contains(&x) && !FONTS[x - M_FONT].0.is_empty() => a.cfg.font = FONTS[x - M_FONT].0.into(),
        x if (M_FONT2..M_FONT2 + FONTS2.len()).contains(&x) => a.cfg.font2 = FONTS2[x - M_FONT2].0.into(),
        M_CONFIG => return After::Open(a.cfg_path.clone()),
        M_FOLDER => {
            let f = a.folder();
            let _ = std::fs::create_dir_all(&f);
            return After::Open(f);
        }
        M_AUTOSTART => {
            set_autostart(!autostart_enabled());
            return After::Nothing;
        }
        M_EXIT => return After::Quit,
        M_SETTINGS => return After::Settings,
        x if (M_UI_THEME..M_UI_THEME + UI_THEMES.len()).contains(&x) => a.cfg.ui_theme = UI_THEMES[x - M_UI_THEME].0.into(),
        x if (M_UI_LANG..M_UI_LANG + UI_LANGS.len()).contains(&x) => a.cfg.ui_lang = UI_LANGS[x - M_UI_LANG].0.into(),
        M_UPDATE_NOW => {
            a.start_update_install();
            return After::Nothing;
        }
        M_UPDATE_CHECK => {
            a.start_update_check(true);
            return After::Nothing;
        }
        M_UPDATE_AUTO => a.cfg.update_check = !a.cfg.update_check,
        M_UPDATE_PAGE => {
            if let UpdState::Available(r) = &a.upd {
                return After::Open(PathBuf::from(r.page.clone()));
            }
            return After::Nothing;
        }
        M_WEATHER_ON => a.cfg.weather = !a.cfg.weather,
        x if (M_BACKDROP..M_BACKDROP + BACKDROPS.len()).contains(&x) => a.cfg.backdrop = BACKDROPS[x - M_BACKDROP].0.into(),
        M_WEATHER_REFRESH => {
            a.start_weather();
            return After::Nothing;
        }
        x if (M_WPOS..M_WPOS + POSITIONS.len()).contains(&x) => a.cfg.weather_position = POSITIONS[x - M_WPOS].0.into(),
        x if (M_WMON..M_WMON + CLOCK_MONS.len()).contains(&x) => a.cfg.weather_monitor = CLOCK_MONS[x - M_WMON].0.into(),
        x if (M_WSCALE..M_WSCALE + SCALES.len()).contains(&x) => a.cfg.weather_scale = SCALES[x - M_WSCALE],
        M_OPEN_LOG => return After::Open(util::app_dir().join("wallclock.log")),
        M_REPORT => return After::Open(PathBuf::from(crash::issue_url())),
        M_OPEN_DIR => return After::Open(util::app_dir()),
        _ => return After::Nothing,
    }
    if a.cfg != old {
        a.save_cfg();
        a.apply_cfg(&old);
    }
    After::Nothing
}

fn show_menu(hwnd: HWND) {
    let mut bitmaps = Vec::new();
    let Some(menu) = with_app(|a| build_menu(a, &mut bitmaps)) else { return };
    let cmd = unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let c = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        for b in bitmaps {
            windows_sys::Win32::Graphics::Gdi::DeleteObject(b as _);
        }
        c as usize
    };
    if cmd == 0 {
        return;
    }
    match with_app(|a| on_menu(a, cmd)).unwrap_or(After::Nothing) {
        After::Nothing => {}
        After::Open(p) => unsafe {
            ShellExecuteW(hwnd, wide("open").as_ptr(), wide_os(p.as_os_str()).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        },
        After::Settings => ui::open(),
        After::Quit => unsafe {
            DestroyWindow(hwnd);
        },
    }
}

// ---------------- главное окно (невидимое) ----------------

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lp as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                // щелчок по значку — окно настроек
                WM_LBUTTONUP => ui::open(),
                // NIN_BALLOONUSERCLICK — щёлкнули по уведомлению: об ошибке или об обновлении
                0x405 => {
                    if CRASH_BALLOON.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        ShellExecuteW(hwnd, wide("open").as_ptr(), wide(&crash::issue_url()).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
                    } else {
                        ui::open_page(7)
                    }
                }
                _ => {}
            }
            0
        }
        WM_UPDATE_CHECKED => {
            if let Some(r) = util::parcel_take::<Result<Option<update::Release>, String>>(lp) {
                let manual = wp != 0;
                if with_app(|a| a.update_checked(manual, r.clone())).is_none() {
                    // состояние занято — проверим ещё раз позже
                    SetTimer(hwnd, T_UPDATE, 2_000, None);
                }
            }
            0
        }
        WM_UPDATE_READY => {
            if let Some(r) = util::parcel_take::<Result<PathBuf, String>>(lp) {
                if with_app(|a| a.update_ready(r)).unwrap_or(false) {
                    DestroyWindow(hwnd); // закрываемся — новая версия встанет на место и запустится
                }
            }
            0
        }
        WM_WP_DONE => {
            match util::parcel_take::<wallpaper::Outcome>(lp) {
                Some(out) => *WP_PENDING.lock().unwrap() = Some(out),
                None => return 0, // не наша посылка (поддельное сообщение)
            }
            let done = with_app(|a| {
                if let Some(out) = WP_PENDING.lock().unwrap().take() {
                    a.change_done(out);
                }
            });
            if done.is_none() {
                SetTimer(hwnd, T_WP_RETRY, 200, None);
            }
            0
        }
        WM_TIMER => {
            let done = with_app(|a| match wp {
                T_CLOCK => {
                    a.tick_clock();
                    a.check_schedule();
                }
                T_WALL => {
                    KillTimer(hwnd, T_WALL);
                    // [10] таймер Windows не длиннее суток — если период ещё не прошёл, просто ждём дальше
                    let period = a.cfg.interval_min as u64 * 60;
                    if util::now_unix().saturating_sub(a.last_change) + 5 >= period {
                        if a.cfg.live == "video" && !a.cfg.video_rotate {
                            // видео выбрано вручную и меняться само не должно — просто ждём следующего круга
                            a.last_change = util::now_unix();
                            a.schedule_wallpaper();
                        } else {
                            a.start_change();
                        }
                    } else {
                        a.schedule_wallpaper();
                    }
                }
                T_WP_RETRY => {
                    KillTimer(hwnd, T_WP_RETRY);
                    PostMessageW(hwnd, WM_WP_DONE, 0, 0);
                }
                T_ZFIX => {
                    KillTimer(hwnd, T_ZFIX);
                    a.clock.fix_z();
                }
                T_UPDATE => {
                    KillTimer(hwnd, T_UPDATE);
                    if a.cfg.update_check {
                        a.start_update_check(false);
                        SetTimer(hwnd, T_UPDATE, 24 * 3600 * 1000, None); // и дальше раз в сутки
                    }
                }
                T_LIVE_RETRY => {
                    KillTimer(hwnd, T_LIVE_RETRY);
                    PostMessageW(hwnd, live::WM_LIVE, 0, 0);
                }
                T_AFTER_WP => {
                    // первая проверка через 1,5 с, вторая — ещё через 2,5 с (Explorer может доделывать переход)
                    KillTimer(hwnd, T_AFTER_WP);
                    a.tick_clock();
                    a.after_wp_checks += 1;
                    if a.after_wp_checks < 2 {
                        SetTimer(hwnd, T_AFTER_WP, 2500, None);
                    } else {
                        a.after_wp_checks = 0;
                    }
                }
                T_SYS_RETRY => {
                    // [9] системное событие пришло, когда программа была занята, — доделываем
                    KillTimer(hwnd, T_SYS_RETRY);
                    a.request_bg();
                    a.tick_clock();
                }
                T_RELOAD => {
                    KillTimer(hwnd, T_RELOAD);
                    a.reload_if_changed();
                }
                T_BG => {
                    KillTimer(hwnd, T_BG);
                    a.start_bg();
                }
                T_WEATHER => {
                    KillTimer(hwnd, T_WEATHER);
                    a.start_weather();
                }
                T_WEATHER_RETRY => {
                    KillTimer(hwnd, T_WEATHER_RETRY);
                    PostMessageW(hwnd, WM_WEATHER_DONE, 0, 0);
                }
                T_BG_RETRY => {
                    KillTimer(hwnd, T_BG_RETRY);
                    PostMessageW(hwnd, WM_BG_DONE, 0, 0);
                }
                T_TRIM => {
                    KillTimer(hwnd, T_TRIM);
                    util::trim_memory();
                }
                _ => {}
            });
            if done.is_none() {
                // состояние занято (вложенный вызов) — попробуем чуть позже
                SetTimer(hwnd, wp, 200, None);
            }
            0
        }
        WM_WEATHER_DONE => {
            // результат кладём в «ячейку» и забираем, когда состояние программы свободно
            match util::parcel_take::<(f64, f64, Result<weather::Weather, String>)>(lp) {
                Some(r) => *WEATHER_PENDING.lock().unwrap() = Some(r),
                None => return 0, // не наша посылка
            }
            let done = with_app(|a| {
                if let Some((lat, lon, r)) = WEATHER_PENDING.lock().unwrap().take() {
                    a.weather_done(lat, lon, r);
                }
            });
            if done.is_none() {
                SetTimer(hwnd, T_WEATHER_RETRY, 200, None);
            }
            0
        }
        // процесс живых обоев прислал новости: показать или убрать слои поверх окон часов
        live::WM_LIVE => {
            match with_app(|a| {
                if a.live.pump() {
                    a.sync_live(); // процесс закрылся — запустить заново (если режим ещё включён)
                }
            }) {
                Some(()) => {}
                None => {
                    SetTimer(hwnd, T_LIVE_RETRY, 200, None);
                }
            }
            0
        }
        WM_OPEN_UI => {
            ui::open();
            0
        }
        WM_BG_DONE => {
            BG_PENDING.fetch_or(wp, std::sync::atomic::Ordering::Relaxed);
            let mask = BG_PENDING.load(std::sync::atomic::Ordering::Relaxed);
            if with_app(|a| a.bg_done(mask)).is_some() {
                BG_PENDING.fetch_and(!mask, std::sync::atomic::Ordering::Relaxed);
            } else {
                SetTimer(hwnd, T_BG_RETRY, 200, None);
            }
            0
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            if with_app(|a| {
                a.request_bg();
                a.tick_clock();
            })
            .is_none()
            {
                SetTimer(hwnd, T_SYS_RETRY, 300, None);
            }
            0
        }
        WM_TIMECHANGE => {
            with_app(|a| a.tick_clock());
            0
        }
        WM_SETTINGCHANGE => {
            let what = wp as u32;
            if what == SPI_SETWORKAREA || what == SPI_SETDESKWALLPAPER {
                // рабочая область или обои поменялись (обои — возможно, другой программой) — обновим фон и часы
                if with_app(|a| {
                    if what == SPI_SETDESKWALLPAPER {
                        a.request_bg();
                    }
                    a.tick_clock();
                })
                .is_none()
                {
                    SetTimer(hwnd, T_SYS_RETRY, 300, None);
                }
            }
            0
        }
        WM_POWERBROADCAST => {
            if wp as u32 == PBT_APMRESUMEAUTOMATIC {
                with_app(|a| {
                    a.tick_clock();
                    a.schedule_wallpaper();
                    a.start_weather();
                });
            }
            1
        }
        WM_DESTROY => {
            ui::close();
            with_app(|a| {
                a.tray(NIM_DELETE, "");
                a.live.stop(); // процесс живых обоев уберёт свои слои и закроется
                a.clock.destroy();
            });
            PostQuitMessage(0);
            0
        }
        _ => {
            // Explorer перезапустился — возвращаем значок и часы
            let tc = with_app(|a| a.taskbar_created).unwrap_or(0);
            if tc != 0 && msg == tc {
                with_app(|a| {
                    a.tray(NIM_ADD, "Hora");
                    a.clock.destroy();
                    a.tick_clock();
                });
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

/// Последнее уведомление в трее — о падении (щелчок откроет отчёт, а не настройки).
static CRASH_BALLOON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // установка и удаление (HoraSetup.exe, «Параметры → Приложения»)
    match args.get(1).map(|s| s.as_str()) {
        Some("--uninstall") => return setup::uninstall(),
        // тихая установка для winget: HoraSetup.exe --silent [--all-users]
        Some("--silent") => std::process::exit(setup::silent_install(args.iter().any(|a| a.eq_ignore_ascii_case("--all-users")))),
        // замена программы новой версией (см. update.rs)
        Some("--update-coordinator") => {
            util::set_log(false);
            let target = args.get(2).map(PathBuf::from).unwrap_or_default();
            let pid = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
            std::process::exit(update::coordinator(target, pid))
        }
        // проверка обновления целиком без окна: найти, скачать, проверить подпись, заменить себя
        Some("--update-selftest") => {
            // как кнопка «Обновить»: найденную версию перед установкой перепроверяем (freshest)
            let code = match update::check() {
                Ok(Some(found)) => {
                    let r = update::freshest(found);
                    match update::download(&r).and_then(|e| update::start(&e)) {
                        Ok(()) => {
                            log(&format!("обновление {}: скачано, подпись верна, замена запущена", r.version));
                            0
                        }
                        Err(e) => {
                            log(&format!("обновление {}: {e}", r.version));
                            2
                        }
                    }
                }
                Ok(None) => {
                    log("обновление: уже последняя версия");
                    1
                }
                Err(e) => {
                    log(&format!("обновление: {e}"));
                    3
                }
            };
            std::process::exit(code)
        }
        Some("--update-apply-admin") => {
            util::set_log(false);
            std::process::exit(update::apply_admin())
        }
        Some("--uninstall-system") => {
            util::set_log(false); // от администратора журнал в профиль не пишем
            std::process::exit(setup::uninstall_system())
        }
        Some("--install-all") => {
            util::set_log(false);
            std::process::exit(setup::install_elevated())
        }
        // перезапуск после падения: сначала дождаться, пока упавшая копия завершится
        Some("--after-crash") => crash::wait_for(args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0)),
        _ if setup::is_setup_exe() => return setup::run(),
        _ => {}
    }
    let dir = util::app_dir();
    let cfg_path = dir.join("config.ini");

    // Режим проверки дизайна: WallClock.exe --render-test out.bmp [стиль] [язык] [ширина] [высота] [шрифт] [цвет]
    // процесс живых обоев: рисует эффекты и видео, настройки получает от главной программы
    if args.get(1).map(|s| s == "--live").unwrap_or(false) {
        crash::install(false);
        live::child_main(args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0));
        return;
    }
    if args.get(1).map(|s| s == "--render-test").unwrap_or(false) {
        let mut cfg = Config::load(&cfg_path);
        if let Some(s) = args.get(3) {
            cfg.style = s.clone();
        }
        if let Some(l) = args.get(4) {
            cfg.lang = l.clone();
        }
        let w = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(1920);
        let h = args.get(6).and_then(|v| v.parse().ok()).unwrap_or(1080);
        if let Some(f) = args.get(7) {
            cfg.font = f.clone();
        }
        if let Some(c) = args.get(8).and_then(|v| u32::from_str_radix(v, 16).ok()) {
            cfg.color = c;
        }
        // девятый аргумент halo — проверить ореол на пёстром фоне
        if args.get(9).map(|s| s == "halo").unwrap_or(false) {
            cfg.halo = true;
            cfg.shadow = true;
        }
        let out = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "clock-test.bmp".into()));
        // стиль «weather» — виджет погоды на примерных данных
        let sample = weather::Weather {
            temp: -3.4,
            feels: -7.2,
            humidity: 81.0,
            wind: 4.1,
            code: 3,
            is_day: true,
            days: (0..4).map(|i| weather::Day { dow: (5 + i) % 7, tmax: [-1.0, 0.4, 2.2, 1.0][i], tmin: [-6.0, -4.3, -3.1, -5.0][i] }).collect(),
            fetched: (14, 30),
        };
        let wx = if cfg.style == "weather" {
            if cfg.weather_city.is_empty() {
                cfg.weather_city = if cfg.lang == "en" { "Berlin, Germany".into() } else { "Берлин, Германия".into() };
            }
            Some(&sample)
        } else {
            None
        };
        std::process::exit(if clock::render_test(&cfg, &out, (w, h), wx) { 0 } else { 1 });
    }

    // Проверка загрузки без установки обоев: WallClock.exe --fetch-test <категории> <папка> [wide]
    if args.get(1).map(|s| s == "--fetch-test").unwrap_or(false) {
        let mut cfg = Config::load(&cfg_path);
        cfg.topics = args.get(2).cloned().unwrap_or_default();
        let dir = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| ".".into()));
        let _ = std::fs::create_dir_all(&dir);
        let dirs = wallpaper::Dirs { cache: dir.clone(), folder: dir.clone(), favorites: dir.clone(), banned: Vec::new() };
        // четвёртый аргумент wide — подбор для единых обоев на все экраны
        let wide = args.get(4).map(|s| s == "wide").unwrap_or(false);
        let mons = clock::monitors();
        let screen = if wide {
            let (l, t, r, b) = mons.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |a, m| (a.0.min(m.rect.left), a.1.min(m.rect.top), a.2.max(m.rect.right), a.3.max(m.rect.bottom)));
            (r - l, b - t)
        } else {
            (mons[0].rect.right - mons[0].rect.left, mons[0].rect.bottom - mons[0].rect.top)
        };
        let r = wallpaper::from_wallhaven(&cfg, &dirs, screen, wide, &mut util::Rng::new());
        let text = match &r {
            Ok(p) => format!("OK {}", p.display()),
            Err(e) => format!("ERR {e}"),
        };
        let _ = std::fs::write(dir.join("result.txt"), text);
        std::process::exit(if r.is_ok() { 0 } else { 1 });
    }

    crash::install(true);
    // проверка перехвата падений: Hora.exe --crash-test panic|av
    match args.get(1).map(|s| s.as_str()).zip(args.get(2).map(|s| s.as_str())) {
        Some(("--crash-test", "panic")) => panic!("проверка перехвата падений"),
        Some(("--crash-test", "av")) => unsafe { std::ptr::write_volatile(8 as *mut u32, 1) },
        _ => {}
    }
    if args.get(1).map(|s| s == "--crash-url").unwrap_or(false) {
        let _ = std::fs::write(dir.join("crash-url.txt"), crash::issue_url());
        return;
    }

    unsafe {
        // только одна копия программы
        CreateMutexW(std::ptr::null(), 0, wide("Local\\WallClock_SingleInstance").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // после падения вторая копия молча уходит (окно настроек не открываем)
            if args.get(1).map(|s| s == "--after-crash").unwrap_or(false) {
                return;
            }
            let other = FindWindowW(wide("WallClockMain").as_ptr(), std::ptr::null());
            if !other.is_null() {
                AllowSetForegroundWindow(u32::MAX); // ASFW_ANY — пусть первая копия выведет окно вперёд
                PostMessageW(other, WM_OPEN_UI, 0, 0);
            }
            return;
        }
    }

    let cache = dir.join("cache");
    let _ = std::fs::create_dir_all(&cache);
    let first_run = !cfg_path.exists();
    let cfg = Config::load(&cfg_path);

    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let cls = wide("WallClockMain");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(main_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: LoadIconW(hinst, 1 as _),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, cls.as_ptr(), wide(APP_NAME).as_ptr(), WS_POPUP, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        if hwnd.is_null() {
            return;
        }

        i18n::set_lang(&cfg.ui_lang);
        let mut app = App {
            hwnd,
            cfg,
            cfg_path: cfg_path.clone(),
            cfg_mtime: None,
            state_path: cache.join("state.txt"),
            cache: cache.clone(),
            clock: clock::Clock::new(hinst, cache.clone()),
            busy: false,
            bg_busy: false,
            bg_again: false,
            current: Vec::new(),
            last_change: 0,
            status: String::new(),
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            weather: None,
            weather_busy: false,
            weather_status: String::new(),
            after_wp_checks: 0,
            fav_names: Vec::new(),
            banned: Vec::new(),
            last_banned: Vec::new(),
            slot_hour: local_hour(),
            live: live::Live::new(hwnd as isize),
            videos: Vec::new(),
            videos_at: None,
            live_mode: live::Mode::Off,
            banned_favs: Vec::new(),
            upd: UpdState::Idle,
            upd_notified: String::new(),
        };
        if first_run {
            app.save_cfg();
        } else {
            app.cfg_mtime = mtime(&cfg_path);
        }
        let _ = std::fs::create_dir_all(app.folder());
        app.load_state();
        app.refresh_lists();
        app.tray(NIM_ADD, "Hora");
        if crash::take_new() {
            log("в прошлый раз программа упала — предлагаем сообщить об ошибке");
            CRASH_BALLOON.store(true, std::sync::atomic::Ordering::Relaxed);
            app.balloon(tr("Hora закрылась из-за ошибки"), tr("Программа перезапущена. Нажмите, чтобы сообщить об ошибке на GitHub — это поможет её исправить."));
        }
        migrate_autostart();
        update::cleanup(); // остатки прошлого обновления
        if app.cfg.update_check {
            SetTimer(hwnd, T_UPDATE, 30_000, None);
        }
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            let rects: Vec<RECT> = clock::monitors().iter().map(|m| m.rect).collect();
            if bg::is_fresh(&a.cache, &rects) {
                a.clock.bg_updated(usize::MAX, None);
            }
            a.tick_clock();
            a.request_bg();
            a.schedule_wallpaper();
            a.start_weather();
        });
        MAIN_HWND.store(hwnd as isize, std::sync::atomic::Ordering::Relaxed);
        log("запуск");
        SetTimer(hwnd, T_TRIM, 3000, None);

        // следим за изменениями config.ini без опроса — система сама разбудит нас
        let watch = FindFirstChangeNotificationW(wide_os(dir.as_os_str()).as_ptr(), 0, FILE_NOTIFY_CHANGE_LAST_WRITE);
        let handles = [watch];
        let n = if watch == INVALID_HANDLE_VALUE || watch.is_null() { 0 } else { 1 };

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let r = MsgWaitForMultipleObjects(n, handles.as_ptr(), 0, INFINITE, QS_ALLINPUT);
            if n == 1 && r == WAIT_OBJECT_0 {
                FindNextChangeNotification(watch);
                SetTimer(hwnd, T_RELOAD, 400, None);
            }
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    return;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
