// Большие часы на рабочем столе.
// Окно встраивается в рабочий стол между картинкой обоев и иконками (как у Wallpaper Engine),
// поэтому не мешает кликам, не перекрывает иконки и не пропадает по Win+D.
// Перерисовка — раз в минуту (или раз в секунду, если включены секунды); остальное время программа спит.
use crate::config::Config;
use crate::util::{log, wide};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// ---------------- встроенные шрифты (лицензия OFL, см. assets/fonts) ----------------

// Встроенные шрифты: (имя семейства, файл). В память грузится только тот, что реально выбран.
pub const EMBEDDED: [(&str, &[u8]); 15] = [
    // свой шрифт программы (tools/make_font.py): буквы с разрывами, латиница и кириллица
    ("WallClock Split", include_bytes!("../assets/fonts/WallClockSplit.ttf")),
    ("Megrim", include_bytes!("../assets/fonts/Megrim.ttf")),
    ("Michroma", include_bytes!("../assets/fonts/Michroma.ttf")),
    ("Unbounded", include_bytes!("../assets/fonts/Unbounded.ttf")),
    ("Jura", include_bytes!("../assets/fonts/Jura.ttf")),
    ("Russo One", include_bytes!("../assets/fonts/RussoOne.ttf")),
    ("Oswald", include_bytes!("../assets/fonts/Oswald.ttf")),
    ("Comfortaa", include_bytes!("../assets/fonts/Comfortaa.ttf")),
    ("Tektur", include_bytes!("../assets/fonts/Tektur.ttf")),
    ("Exo 2", include_bytes!("../assets/fonts/Exo2.ttf")),
    ("Montserrat Alternates Light", include_bytes!("../assets/fonts/MontserratAlternates.ttf")),
    ("Cormorant Garamond", include_bytes!("../assets/fonts/CormorantGaramond.ttf")),
    ("Press Start 2P", include_bytes!("../assets/fonts/PressStart2P.ttf")),
    ("Hora Pixel", include_bytes!("../assets/fonts/HoraPixel.ttf")),
    ("Hora Block", include_bytes!("../assets/fonts/HoraBlock.ttf")),
];

#[derive(Default)]
pub struct FontStore {
    loaded: Vec<&'static str>,
}

thread_local! {
    // какие встроенные шрифты уже загружены (общий список для часов и окна настроек)
    static LOADED_FONTS: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl FontStore {
    /// Подгружает встроенный шрифт в память только когда он реально нужен.
    pub fn ensure(&mut self, face: &str) {
        for (name, data) in EMBEDDED {
            if !name.eq_ignore_ascii_case(face) {
                continue;
            }
            let fresh = LOADED_FONTS.with(|l| {
                let mut l = l.borrow_mut();
                if l.contains(&name) {
                    false
                } else {
                    l.push(name);
                    true
                }
            });
            if fresh {
                let mut n: u32 = 0;
                let h = unsafe { AddFontMemResourceEx(data.as_ptr() as *const c_void, data.len() as u32, std::ptr::null(), &mut n) };
                if h.is_null() {
                    log(&format!("не удалось загрузить шрифт {name}"));
                }
            }
            if !self.loaded.contains(&name) {
                self.loaded.push(name);
            }
        }
    }
}

// ---------------- описание того, что рисовать ----------------

#[derive(Clone)]
struct FontSpec {
    face: String,
    px: i32,
    weight: i32,
    stretch: f32, // 1.0 — обычная ширина букв, больше — шире
}

struct Line {
    text: Vec<u16>,
    font: FontSpec,
    spacing: i32,  // межбуквенный интервал, px
    gap: i32,      // отступ сверху от предыдущей строки, px
    outline: bool, // рисовать только контур букв
    bold: f32,     // утолщение штрихов (доля размера шрифта), 0 — без утолщения
}

enum Layout {
    Stack(Vec<Line>),
    Split { left: Vec<Line>, right: Vec<Line>, bar_w: i32, gap: i32, top: bool },
    /// стрелочный циферблат радиусом r, под ним строки текста
    Dial { r: i32, gap: i32, lines: Vec<Line> },
}

const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAYS_RU: [&str; 7] = ["воскресенье", "понедельник", "вторник", "среда", "четверг", "пятница", "суббота"];
const DAYS_RU_SHORT: [&str; 7] = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_RU: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];

pub type SysTime = windows_sys::Win32::Foundation::SYSTEMTIME;

struct Texts {
    day: String,
    day_short: String,
    date_full: String,  // 13 НОЯБРЯ 2026 / NOVEMBER 13, 2026
    date_short: String, // 13 НОЯБРЯ / NOVEMBER 13
    time: String,       // 14:05 или 2:05 (без AM/PM)
    ampm: String,       // "" | AM | PM
    hh: String,
    mm: String,
    ss: String,
    dnum: String,  // 13
    month: String, // НОЯБРЯ / NOVEMBER
}

fn texts(cfg: &Config, st: &SysTime) -> Texts {
    let en = cfg.lang == "en";
    let dow = (st.wDayOfWeek as usize) % 7;
    let mon = (st.wMonth as usize).clamp(1, 12) - 1;
    let h24 = cfg.is_24h();
    let mut h = st.wHour as u32;
    let mut ampm = String::new();
    if !h24 {
        ampm = if h < 12 { "AM".into() } else { "PM".into() };
        h %= 12;
        if h == 0 {
            h = 12;
        }
    }
    let mut time = if h24 { format!("{:02}:{:02}", h, st.wMinute) } else { format!("{}:{:02}", h, st.wMinute) };
    if cfg.seconds {
        time += &format!(":{:02}", st.wSecond);
    }
    let (day, day_short, date_full, date_short) = if en {
        (
            DAYS_EN[dow].to_uppercase(),
            DAYS_EN[dow][..3].to_uppercase(),
            format!("{} {}, {}", MONTHS_EN[mon], st.wDay, st.wYear).to_uppercase(),
            format!("{} {}", MONTHS_EN[mon], st.wDay).to_uppercase(),
        )
    } else {
        (
            DAYS_RU[dow].to_uppercase(),
            DAYS_RU_SHORT[dow].to_uppercase(),
            format!("{} {} {}", st.wDay, MONTHS_RU[mon], st.wYear).to_uppercase(),
            format!("{} {}", st.wDay, MONTHS_RU[mon]).to_uppercase(),
        )
    };
    Texts {
        day,
        day_short,
        date_full,
        date_short,
        time,
        ampm,
        hh: if h24 { format!("{:02}", h) } else { format!("{:02}", h) },
        mm: format!("{:02}", st.wMinute),
        ss: format!("{:02}", st.wSecond),
        dnum: st.wDay.to_string(),
        month: if en { MONTHS_EN[mon].to_uppercase() } else { MONTHS_RU[mon].to_uppercase() },
    }
}

/// Время словами с округлением до 5 минут: (верхняя строка, крупная строка).
fn words(en: bool, st: &SysTime) -> (String, String) {
    let mut h = st.wHour as usize;
    let mut m5 = (st.wMinute as usize + 2) / 5 * 5;
    if m5 == 60 {
        m5 = 0;
        h = (h + 1) % 24;
    }
    // к «без …» и «… минут второго» относится следующий час
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let next = h12 % 12 + 1;
    if en {
        const N: [&str; 13] = ["", "ONE", "TWO", "THREE", "FOUR", "FIVE", "SIX", "SEVEN", "EIGHT", "NINE", "TEN", "ELEVEN", "TWELVE"];
        if m5 == 0 {
            return match h {
                12 => ("IT'S".into(), "NOON".into()),
                0 => ("IT'S".into(), "MIDNIGHT".into()),
                _ => ("IT'S".into(), format!("{} O'CLOCK", N[h12])),
            };
        }
        let (top, hour) = match m5 {
            5 => ("FIVE PAST", h12),
            10 => ("TEN PAST", h12),
            15 => ("QUARTER PAST", h12),
            20 => ("TWENTY PAST", h12),
            25 => ("TWENTY-FIVE PAST", h12),
            30 => ("HALF PAST", h12),
            35 => ("TWENTY-FIVE TO", next),
            40 => ("TWENTY TO", next),
            45 => ("QUARTER TO", next),
            50 => ("TEN TO", next),
            _ => ("FIVE TO", next),
        };
        (top.into(), N[hour].into())
    } else {
        const CARD: [&str; 13] = ["", "ЧАС", "ДВА", "ТРИ", "ЧЕТЫРЕ", "ПЯТЬ", "ШЕСТЬ", "СЕМЬ", "ВОСЕМЬ", "ДЕВЯТЬ", "ДЕСЯТЬ", "ОДИННАДЦАТЬ", "ДВЕНАДЦАТЬ"];
        const ORD: [&str; 13] = ["", "ПЕРВОГО", "ВТОРОГО", "ТРЕТЬЕГО", "ЧЕТВЁРТОГО", "ПЯТОГО", "ШЕСТОГО", "СЕДЬМОГО", "ВОСЬМОГО", "ДЕВЯТОГО", "ДЕСЯТОГО", "ОДИННАДЦАТОГО", "ДВЕНАДЦАТОГО"];
        if m5 == 0 {
            return match h {
                12 => ("РОВНО".into(), "ПОЛДЕНЬ".into()),
                0 => ("РОВНО".into(), "ПОЛНОЧЬ".into()),
                _ => {
                    let unit = match h12 {
                        1 => "",
                        2..=4 => " ЧАСА",
                        _ => " ЧАСОВ",
                    };
                    ("РОВНО".into(), format!("{}{}", CARD[h12], unit))
                }
            };
        }
        match m5 {
            5 => ("ПЯТЬ МИНУТ".into(), ORD[next].into()),
            10 => ("ДЕСЯТЬ МИНУТ".into(), ORD[next].into()),
            15 => ("ЧЕТВЕРТЬ".into(), ORD[next].into()),
            20 => ("ДВАДЦАТЬ МИНУТ".into(), ORD[next].into()),
            25 => ("ДВАДЦАТЬ ПЯТЬ МИНУТ".into(), ORD[next].into()),
            30 => ("ПОЛОВИНА".into(), ORD[next].into()),
            35 => ("БЕЗ ДВАДЦАТИ ПЯТИ".into(), CARD[next].into()),
            40 => ("БЕЗ ДВАДЦАТИ".into(), CARD[next].into()),
            45 => ("БЕЗ ЧЕТВЕРТИ".into(), CARD[next].into()),
            50 => ("БЕЗ ДЕСЯТИ".into(), CARD[next].into()),
            _ => ("БЕЗ ПЯТИ".into(), CARD[next].into()),
        }
    }
}

/// Собирает строки и шрифты для выбранного стиля. `k` — базовая единица размера (высота экрана × масштаб).
fn build(cfg: &Config, st: &SysTime, k: f32) -> Layout {
    let en = cfg.lang == "en";
    let t = texts(cfg, st);
    let px = |f: f32| ((k * f).round() as i32).max(6);
    let big_face = |def_en: &str, def_ru: &str| -> String {
        if cfg.font.is_empty() || cfg.font.eq_ignore_ascii_case("auto") {
            if en { def_en.into() } else { def_ru.into() }
        } else {
            cfg.font.clone()
        }
    };
    let small_face = || -> String {
        if cfg.font2.is_empty() || cfg.font2.eq_ignore_ascii_case("auto") {
            "Bahnschrift".into()
        } else {
            cfg.font2.clone()
        }
    };
    let line = |text: &str, face: String, size: i32, weight: i32, spacing_em: f32, gap: i32| Line {
        text: text.encode_utf16().collect(),
        font: FontSpec { face, px: size, weight, stretch: 1.0 },
        spacing: (size as f32 * spacing_em).round() as i32,
        gap,
        outline: false,
        bold: 0.0,
    };
    let outlined = |mut l: Line| {
        l.outline = true;
        l
    };
    let custom_big = !(cfg.font.is_empty() || cfg.font.eq_ignore_ascii_case("auto"));
    let time_ampm = if t.ampm.is_empty() { t.time.clone() } else { format!("{} {}", t.time, t.ampm) };

    match cfg.style.as_str() {
        // Крупное время и подпись с датой
        "minimal" => {
            let sub = if t.ampm.is_empty() {
                format!("{}, {}", t.day, t.date_short)
            } else {
                format!("{} · {}, {}", t.ampm, t.day, t.date_short)
            };
            Layout::Stack(vec![
                line(&t.time, big_face("Jura", "Jura"), px(0.19), 400, 0.0, 0),
                line(&sub, small_face(), px(0.026), 300, 0.28, px(0.04)),
            ])
        }
        // Время | день недели и дата
        "split" => Layout::Split {
            left: vec![line(&t.time, big_face("Unbounded", "Unbounded"), px(0.12), 400, 0.0, 0)],
            right: vec![
                line(&t.day, small_face(), px(0.036), 600, 0.18, 0),
                line(&if t.ampm.is_empty() { t.date_full.clone() } else { format!("{} · {}", t.date_full, t.ampm) }, small_face(), px(0.026), 300, 0.12, px(0.022)),
            ],
            bar_w: px(0.004).max(2),
            gap: px(0.035),
            top: false,
        },
        // Часы над минутами
        "vertical" => {
            let face = big_face("Michroma", "Unbounded");
            let mut v = vec![
                line(&t.hh, face.clone(), px(0.2), 400, 0.06, 0),
                line(&t.mm, face.clone(), px(0.2), 400, 0.06, px(0.05)),
            ];
            if cfg.seconds {
                v.push(line(&t.ss, face, px(0.2), 400, 0.06, px(0.05)));
            }
            let sub = format!("{} · {}{}", t.day_short, t.date_short, if t.ampm.is_empty() { String::new() } else { format!(" · {}", t.ampm) });
            v.push(line(&sub, small_face(), px(0.024), 400, 0.3, px(0.06)));
            Layout::Stack(v)
        }
        // Время словами: «ДВАДЦАТЬ МИНУТ / ВТОРОГО»
        "words" => {
            let (top, big) = words(en, st);
            Layout::Stack(vec![
                line(&top, small_face(), px(0.04), 300, 0.24, 0),
                line(&big, big_face("Michroma", "Unbounded"), px(0.1), 400, 0.12, px(0.032)),
                line(&format!("{} · {}, {}", time_ampm, t.day, t.date_short), small_face(), px(0.021), 400, 0.22, px(0.045)),
            ])
        }
        // Календарь: огромное число, рядом месяц, день недели и время
        "calendar" => Layout::Split {
            left: vec![line(&t.dnum, big_face("Unbounded", "Unbounded"), px(0.21), 400, 0.0, 0)],
            right: vec![
                line(&t.month, small_face(), px(0.05), 600, 0.12, 0),
                line(&t.day, small_face(), px(0.03), 300, 0.24, px(0.03)),
                line(&time_ampm, small_face(), px(0.03), 400, 0.16, px(0.022)),
            ],
            bar_w: px(0.004).max(2),
            gap: px(0.035),
            top: false,
        },
        // Контур: большое время полыми буквами
        "outline" => Layout::Stack(vec![
            outlined(line(&time_ampm, big_face("Michroma", "Michroma"), px(0.19), 400, 0.04, 0)),
            line(&format!("{}, {}", t.day, t.date_short), small_face(), px(0.026), 400, 0.3, px(0.05)),
        ]),
        // Постер: очень крупные узкие жирные цифры
        "poster" => {
            let face = if custom_big { cfg.font.clone() } else { "Bahnschrift Condensed".into() };
            Layout::Stack(vec![
                line(&t.day, small_face(), px(0.03), 600, 0.5, 0),
                line(&t.time, face, px(0.32), 700, 0.0, px(0.04)),
                line(&if t.ampm.is_empty() { t.date_full.clone() } else { format!("{} · {}", t.ampm, t.date_full) }, small_face(), px(0.028), 300, 0.36, px(0.045)),
            ])
        }
        // Одной строкой — компактно
        "line" => Layout::Split {
            left: vec![line(&time_ampm, big_face("Unbounded", "Unbounded"), px(0.05), 400, 0.0, 0)],
            right: vec![line(&format!("{}, {}", t.day, t.date_full), small_face(), px(0.024), 400, 0.16, 0)],
            bar_w: px(0.003).max(2),
            gap: px(0.022),
            top: false,
        },
        // Техно: цифры вразрядку (Michroma годится и для русского — в цифрах кириллица не нужна)
        "techno" => Layout::Stack(vec![
            line(&t.day, small_face(), px(0.024), 400, 0.6, 0),
            line(&time_ampm, big_face("Michroma", "Michroma"), px(0.11), 400, 0.38, px(0.04)),
            line(&t.date_full, small_face(), px(0.024), 300, 0.3, px(0.04)),
        ]),
        // Стрелочные часы
        "analog" => Layout::Dial {
            r: px(0.17),
            gap: px(0.05),
            lines: vec![
                line(&t.day, small_face(), px(0.03), 600, 0.3, 0),
                line(&t.date_short, small_face(), px(0.022), 300, 0.28, px(0.022)),
            ],
        },
        // «Точно как на фото»: день недели своим шрифтом с разрывами в буквах (русский или английский)
        "photo" => {
            let dow = (st.wDayOfWeek as usize) % 7;
            let mon = (st.wMonth as usize).clamp(1, 12) - 1;
            let (day, date) = if en {
                (DAYS_EN[dow].to_uppercase(), format!("{} {}, {}", MONTHS_EN[mon], st.wDay, st.wYear).to_uppercase())
            } else {
                (DAYS_RU[dow].to_uppercase(), format!("{} {} {}", st.wDay, MONTHS_RU[mon], st.wYear).to_uppercase())
            };
            let mut h = st.wHour as u32;
            let time = if cfg.is_24h() {
                format!("{:02}:{:02}", h, st.wMinute)
            } else {
                let ampm = if h < 12 { "AM" } else { "PM" };
                h %= 12;
                if h == 0 {
                    h = 12;
                }
                format!("{}:{:02} {}", h, st.wMinute, ampm)
            };
            let time = if cfg.seconds { format!("{}:{:02}", time.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == ' '), st.wSecond) } else { time };
            let face = if cfg.font.is_empty() || cfg.font.eq_ignore_ascii_case("auto") { "WallClock Split".to_string() } else { cfg.font.clone() };
            let mut big = line(&day, face.clone(), px(0.1), 400, 0.2, 0);
            // Megrim тонкий — для него утолщение и растяжение; свой шрифт уже нужной толщины
            if face == "Megrim" {
                big.font.stretch = 1.35;
                big.bold = 0.032;
                big.spacing = (big.font.px as f32 * 0.17) as i32;
            }
            Layout::Stack(vec![
                big,
                line(&date, small_face(), px(0.029), 400, 0.03, px(0.04)),
                line(&format!("- {} -", time), small_face(), px(0.018), 400, 0.06, px(0.024)),
            ])
        }
        // «Как на фото»: огромный день недели вразрядку, под ним дата и время
        _ => Layout::Stack(vec![
            line(&t.day, big_face("Michroma", "Unbounded"), px(0.092), 400, 0.42, 0),
            line(&t.date_full, small_face(), px(0.031), 600, 0.08, px(0.04)),
            line(&format!("- {} -", time_ampm), small_face(), px(0.022), 400, 0.14, px(0.024)),
        ]),
    }
}

/// Виджет погоды: слева большой значок, справа город, температура, описание, подробности и прогноз.
fn build_weather(cfg: &Config, w: &crate::weather::Weather, k: f32) -> Layout {
    use crate::weather::{deg, describe, icon};
    let en = cfg.lang == "en";
    let px = |f: f32| ((k * f).round() as i32).max(6);
    let small = if cfg.font2.is_empty() || cfg.font2.eq_ignore_ascii_case("auto") { "Bahnschrift".to_string() } else { cfg.font2.clone() };
    // в Michroma нет знака «°», поэтому по умолчанию — Unbounded
    let big = if cfg.font.is_empty() || cfg.font.eq_ignore_ascii_case("auto") || cfg.font == "Michroma" { "Unbounded".to_string() } else { cfg.font.clone() };
    let line = |text: &str, face: &str, size: i32, weight: i32, spacing_em: f32, gap: i32| Line {
        text: text.encode_utf16().collect(),
        font: FontSpec { face: face.to_string(), px: size, weight, stretch: 1.0 },
        spacing: (size as f32 * spacing_em).round() as i32,
        gap,
        outline: false,
        bold: 0.0,
    };
    let mut right = Vec::new();
    let has_city = !cfg.weather_city.is_empty();
    if has_city {
        let city = cfg.weather_city.split(',').next().unwrap_or("").trim().to_uppercase();
        right.push(line(&city, &small, px(0.016), 600, 0.3, 0));
    }
    let g = if right.is_empty() { 0 } else { px(0.02) };
    right.push(line(&deg(w.temp), &big, px(0.07), 400, 0.04, g));
    right.push(line(&describe(w.code, en).to_uppercase(), &small, px(0.02), 600, 0.12, px(0.02)));
    let mut det = Vec::new();
    if w.feels.is_finite() {
        det.push(format!("{} {}", if en { "feels" } else { "ощущается" }, deg(w.feels)));
    }
    if w.wind.is_finite() {
        det.push(format!("{} {:.0} {}", if en { "wind" } else { "ветер" }, w.wind, if en { "m/s" } else { "м/с" }));
    }
    if w.humidity.is_finite() {
        det.push(format!("{:.0}%", w.humidity));
    }
    right.push(line(&det.join(" · "), &small, px(0.016), 400, 0.06, px(0.016)));
    // прогноз на следующие три дня
    let fc: Vec<String> = w
        .days
        .iter()
        .skip(1)
        .take(3)
        .map(|d| {
            let name = if en { DAYS_EN[d.dow][..3].to_uppercase() } else { DAYS_RU_SHORT[d.dow].to_uppercase() };
            format!("{} {}/{}", name, deg(d.tmax), deg(d.tmin))
        })
        .collect();
    if !fc.is_empty() {
        right.push(line(&fc.join("   "), &small, px(0.016), 400, 0.08, px(0.022)));
    }
    Layout::Split {
        // значок опускаем до уровня строки с температурой (под названием города)
        left: vec![line(icon(w.code, w.is_day), "Segoe UI Symbol", px(0.1), 400, 0.0, if has_city { px(0.016 * 0.72 + 0.02) } else { 0 })],
        right,
        bar_w: 0,
        gap: px(0.02),
        top: true,
    }
}

/// На каких мониторах показывать погоду (индексы в списке monitors()).
pub fn weather_targets(cfg: &Config, mons: &[Mon]) -> Vec<usize> {
    match cfg.weather_monitor.as_str() {
        "all" => (0..mons.len()).collect(),
        "second" => vec![if mons.len() > 1 { 1 } else { 0 }],
        _ => vec![0],
    }
}

// ---------------- раскладка и отрисовка ----------------

enum Item {
    Text { line: usize, x: i32, y: i32 },
    Bar(RECT),
    Dial { x: i32, y: i32, r: i32 },
}

struct Measured {
    font: HFONT,
    width: i32,
    ascent: i32,
    height: i32,
    cap: i32,
}

fn create_font(f: &FontSpec, width: i32) -> HFONT {
    let face = wide(&f.face);
    unsafe {
        CreateFontW(
            -f.px,
            width,
            0,
            0,
            f.weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as _,
            OUT_TT_PRECIS as _,
            CLIP_DEFAULT_PRECIS as _,
            NONANTIALIASED_QUALITY as _,
            (DEFAULT_PITCH | FF_DONTCARE) as _,
            face.as_ptr(),
        )
    }
}

fn measure(dc: HDC, l: &Line) -> Measured {
    unsafe {
        let mut font = create_font(&l.font, 0);
        let mut old = SelectObject(dc, font as _);
        // растянутый по ширине шрифт: берём обычную среднюю ширину буквы и умножаем
        if (l.font.stretch - 1.0).abs() > 0.01 {
            let mut tm0: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(dc, &mut tm0);
            SelectObject(dc, old);
            DeleteObject(font as _);
            font = create_font(&l.font, ((tm0.tmAveCharWidth as f32) * l.font.stretch).round().max(1.0) as i32);
            old = SelectObject(dc, font as _);
        }
        SetTextCharacterExtra(dc, l.spacing);
        let mut sz = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(dc, l.text.as_ptr(), l.text.len() as i32, &mut sz);
        let mut tm: TEXTMETRICW = std::mem::zeroed();
        GetTextMetricsW(dc, &mut tm);
        // Высота заглавных: меряем реальный контур буквы «H» — метаданным некоторых шрифтов верить нельзя
        let mut cap = 0i32;
        let mut gm: GLYPHMETRICS = std::mem::zeroed();
        let one = FIXED { fract: 0, value: 1 };
        let zero = FIXED { fract: 0, value: 0 };
        let mat = MAT2 { eM11: one, eM12: zero, eM21: zero, eM22: one };
        if GetGlyphOutlineW(dc, 'H' as u32, GGO_METRICS, &mut gm, 0, std::ptr::null_mut(), &mat) != GDI_ERROR as u32 {
            cap = gm.gmptGlyphOrigin.y;
        }
        let need = if cap > 0 { 0 } else { GetOutlineTextMetricsW(dc, 0, std::ptr::null_mut()) };
        if need > 0 {
            let mut buf = vec![0u64; (need as usize + 7) / 8];
            let otm = buf.as_mut_ptr() as *mut OUTLINETEXTMETRICW;
            if GetOutlineTextMetricsW(dc, need, otm) > 0 {
                cap = (*otm).otmsCapEmHeight as i32;
            }
        }
        if cap <= 0 {
            cap = tm.tmAscent * 7 / 10;
        }
        SelectObject(dc, old);
        Measured { font, width: (sz.cx - l.spacing).max(1), ascent: tm.tmAscent, height: tm.tmHeight, cap }
    }
}

/// Раскладывает строки столбиком; возвращает позиции, ширину и высоту (по заглавным буквам).
fn stack(ms: &[Measured], lines: &[Line], base: usize, align: i32) -> (Vec<Item>, i32, i32) {
    let w = ms.iter().map(|m| m.width).max().unwrap_or(1);
    let mut items = Vec::new();
    let mut y = 0;
    for (i, m) in ms.iter().enumerate() {
        let cap_top = if i == 0 { lines[i].gap } else { y + lines[i].gap };
        let baseline = cap_top + m.cap;
        let x = match align {
            0 => 0,
            2 => w - m.width,
            _ => (w - m.width) / 2,
        };
        items.push(Item::Text { line: base + i, x, y: baseline - m.ascent });
        y = baseline;
    }
    (items, w, y)
}

/// Готовый кадр: картинка в памяти (DIB) + прямоугольники текста и всего изображения с тенью.
pub struct Frame {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    pub bits: *mut u32,
    pub w: i32,
    pub h: i32,
    pub text: RECT,
    pub all: RECT,
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as _);
            DeleteDC(self.dc);
        }
    }
}

fn align_of(pos: &str) -> i32 {
    if pos.ends_with("left") || pos == "left" {
        0
    } else if pos.ends_with("right") || pos == "right" {
        2
    } else {
        1
    }
}

/// Рисует часы в память. `area` — размеры монитора (ширина, высота).
pub fn compose(cfg: &Config, st: &SysTime, area: (i32, i32), fonts: &mut FontStore) -> Option<Frame> {
    let base_k = area.1 as f32 * cfg.scale as f32 / 100.0;
    compose_with(cfg, st, area, fonts, base_k, &cfg.position, &|k| build(cfg, st, k))
}

/// Виджет погоды — рисуется тем же способом, что и часы.
pub fn compose_weather(cfg: &Config, st: &SysTime, area: (i32, i32), fonts: &mut FontStore, w: &crate::weather::Weather) -> Option<Frame> {
    let base_k = area.1 as f32 * cfg.weather_scale as f32 / 100.0;
    compose_with(cfg, st, area, fonts, base_k, &cfg.weather_position, &|k| build_weather(cfg, w, k))
}

/// Во сколько раз крупнее рисуем текст перед уменьшением (4×4 точки → 1, это 17 уровней сглаживания).
const SS: i32 = 4;

/// Чёрно-белый DIB (1 бит на точку): палитра из чёрного и белого.
#[repr(C)]
struct MonoInfo {
    hdr: BITMAPINFOHEADER,
    colors: [u32; 2],
}

fn compose_with(cfg: &Config, st: &SysTime, area: (i32, i32), fonts: &mut FontStore, base_k: f32, position: &str, build: &dyn Fn(f32) -> Layout) -> Option<Frame> {
    // Всё раскладываем и рисуем в SS раз крупнее (координаты «_s»), потом уменьшаем с усреднением.
    let max_w = (area.0 as f32 * 0.9) as i32 * SS;
    let mut k = base_k;
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        // до двух попыток: если надпись шире экрана — уменьшаем
        for _ in 0..3 {
            let layout = build(k * SS as f32);
            let all_lines: Vec<&Line> = match &layout {
                Layout::Stack(v) => v.iter().collect(),
                Layout::Split { left, right, .. } => left.iter().chain(right.iter()).collect(),
                Layout::Dial { lines, .. } => lines.iter().collect(),
            };
            for l in &all_lines {
                fonts.ensure(&l.font.face);
            }
            let ms: Vec<Measured> = all_lines.iter().map(|l| measure(dc, l)).collect();
            let align = align_of(position);
            let (items, w, h) = match &layout {
                Layout::Stack(v) => stack(&ms, v, 0, align),
                Layout::Split { left, right, bar_w, gap, top } => {
                    let (li, lw, lh) = stack(&ms[..left.len()], left, 0, 2);
                    let (ri, rw, rh) = stack(&ms[left.len()..], right, left.len(), 0);
                    let h = lh.max(rh);
                    let mut items = Vec::new();
                    let (ly, ry) = if *top { (0, 0) } else { ((h - lh) / 2, (h - rh) / 2) };
                    let rx = lw + gap * 2 + bar_w;
                    for it in li {
                        if let Item::Text { line, x, y } = it {
                            items.push(Item::Text { line, x, y: y + ly });
                        }
                    }
                    let bar_h = (h as f32 * 1.0) as i32;
                    items.push(Item::Bar(RECT { left: lw + gap, top: (h - bar_h) / 2, right: lw + gap + bar_w, bottom: (h - bar_h) / 2 + bar_h }));
                    for it in ri {
                        if let Item::Text { line, x, y } = it {
                            items.push(Item::Text { line, x: x + rx, y: y + ry });
                        }
                    }
                    (items, rx + rw, h)
                }
                Layout::Dial { r, gap, lines } => {
                    let (li, lw, lh) = stack(&ms, lines, 0, 1);
                    let w = lw.max(r * 2);
                    let mut items = vec![Item::Dial { x: (w - r * 2) / 2, y: 0, r: *r }];
                    for it in li {
                        if let Item::Text { line, x, y } = it {
                            items.push(Item::Text { line, x: x + (w - lw) / 2, y: y + r * 2 + gap });
                        }
                    }
                    (items, w, r * 2 + gap + lh)
                }
            };
            if w > max_w && k > base_k * 0.4 {
                for m in &ms {
                    DeleteObject(m.font as _);
                }
                k *= max_w as f32 / w as f32 * 0.97;
                continue;
            }

            // --- холст с запасом под выносные элементы букв и тень (в обычном размере) ---
            let max_px = all_lines.iter().map(|l| l.font.px).max().unwrap_or(10) / SS;
            let blur = if cfg.shadow { ((area.1 as f32) * if cfg.halo { 0.0035 } else { 0.0045 }).round().max(2.0) as i32 } else { 0 };
            // обычная тень чуть смещена вниз-вправо; ореол — ровно вокруг букв
            let off = if cfg.shadow && !cfg.halo { (blur / 2).max(1) } else { 0 };
            let m = max_px * 6 / 10 + blur * 3 + off;
            let cw = (w + SS - 1) / SS + m * 2;
            let ch = (h + SS - 1) / SS + m * 2;
            let (cw_s, ch_s, m_s) = (cw * SS, ch * SS, m * SS);

            // 1) текст — в крупную чёрно-белую маску
            let mut mi: MonoInfo = std::mem::zeroed();
            mi.hdr.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            mi.hdr.biWidth = cw_s;
            mi.hdr.biHeight = -ch_s; // сверху вниз
            mi.hdr.biPlanes = 1;
            mi.hdr.biBitCount = 1;
            mi.hdr.biClrUsed = 2;
            mi.colors = [0x0000_0000, 0x00FF_FFFF];
            let mut mbits: *mut c_void = std::ptr::null_mut();
            let mono = CreateDIBSection(dc, &mi as *const MonoInfo as *const BITMAPINFO, DIB_RGB_COLORS, &mut mbits, std::ptr::null_mut(), 0);
            if mono.is_null() || mbits.is_null() {
                for m in &ms {
                    DeleteObject(m.font as _);
                }
                DeleteDC(dc);
                return None;
            }
            let old_mono = SelectObject(dc, mono as _);
            SetBkMode(dc, TRANSPARENT as _);
            SetTextColor(dc, 0x00FF_FFFF);
            let white = GetStockObject(WHITE_BRUSH);
            for it in &items {
                match it {
                    Item::Text { line, x, y } => {
                        let l = all_lines[*line];
                        let prev = SelectObject(dc, ms[*line].font as _);
                        SetTextCharacterExtra(dc, l.spacing);
                        let r = (l.font.px as f32 * l.bold).round() as i32;
                        if r <= 0 {
                            TextOutW(dc, x + m_s, y + m_s, l.text.as_ptr(), l.text.len() as i32);
                        } else {
                            // утолщение штрихов: та же строка со сдвигами внутри круга радиуса r
                            let step = (r / 4).max(1);
                            let mut dy = -r;
                            while dy <= r {
                                let mut dx = -r;
                                while dx <= r {
                                    if dx * dx + dy * dy <= r * r {
                                        TextOutW(dc, x + m_s + dx, y + m_s + dy, l.text.as_ptr(), l.text.len() as i32);
                                    }
                                    dx += step;
                                }
                                dy += step;
                            }
                        }
                        SelectObject(dc, prev);
                    }
                    Item::Bar(r) => {
                        let rr = RECT { left: r.left + m_s, top: r.top + m_s, right: r.right + m_s, bottom: r.bottom + m_s };
                        FillRect(dc, &rr, white as _);
                    }
                    Item::Dial { .. } => {}
                }
            }
            for mm in &ms {
                DeleteObject(mm.font as _);
            }
            GdiFlush();

            // 2) уменьшаем: в каждом квадрате 4×4 считаем закрашенные точки → прозрачность 0..255
            let stride = (((cw_s + 31) / 32) * 4) as usize;
            let mask = std::slice::from_raw_parts(mbits as *const u8, stride * ch_s as usize);
            let n = (cw * ch) as usize;
            let mut alpha = vec![0u8; n];
            for y in 0..ch as usize {
                let rows = [&mask[(y * 4) * stride..], &mask[(y * 4 + 1) * stride..], &mask[(y * 4 + 2) * stride..], &mask[(y * 4 + 3) * stride..]];
                for x in 0..cw as usize {
                    let shift = if x % 2 == 0 { 4 } else { 0 }; // в байте 8 точек: старший полубайт — левые четыре
                    let mut cnt = 0u32;
                    for r in &rows {
                        cnt += ((r[x / 2] >> shift) & 0x0F).count_ones();
                    }
                    alpha[y * cw as usize + x] = (cnt * 255 / 16) as u8;
                }
            }
            SelectObject(dc, old_mono);
            DeleteObject(mono as _);

            // 3) итоговая картинка в обычном размере (32 бита на точку, с прозрачностью)
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = cw;
            bi.bmiHeader.biHeight = -ch;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp as _);
            let px = std::slice::from_raw_parts_mut(bits as *mut u32, n);
            for it in &items {
                match it {
                    // контурные строки: из залитых букв вычитаем «сжатые» — остаётся сглаженная обводка
                    Item::Text { line, x, y } if all_lines[*line].outline => {
                        let kk = (all_lines[*line].font.px as f32 / SS as f32 * 0.018).round().max(2.0) as i32;
                        let pad = kk + 4;
                        let r = RECT {
                            left: (x / SS + m - pad).max(0),
                            top: (y / SS + m).max(0),
                            right: ((x + ms[*line].width) / SS + m + pad).min(cw),
                            bottom: ((y + ms[*line].height) / SS + m).min(ch),
                        };
                        outline_region(&mut alpha, cw, r, kk);
                    }
                    Item::Dial { x, y, r } => {
                        let (rr, cx, cy) = (*r as f32 / SS as f32, (x + m_s) as f32 / SS as f32, (y + m_s) as f32 / SS as f32);
                        draw_dial(&mut alpha, cw, cx + rr, cy + rr, rr, st, cfg.seconds)
                    }
                    _ => {}
                }
            }
            let text = bbox(&alpha, cw, ch).unwrap_or(RECT { left: 0, top: 0, right: 1, bottom: 1 });
            let mut all = text;
            if cfg.shadow {
                let mut sh = vec![0u8; n];
                for y in 0..(ch - off) {
                    let src = (y * cw) as usize;
                    let dst = ((y + off) * cw + off) as usize;
                    sh[dst..dst + (cw - off) as usize].copy_from_slice(&alpha[src..src + (cw - off) as usize]);
                }
                for _ in 0..3 {
                    box_blur(&mut sh, cw, ch, blur);
                }
                if let Some(sb) = bbox(&sh, cw, ch) {
                    all = RECT {
                        left: all.left.min(sb.left),
                        top: all.top.min(sb.top),
                        right: all.right.max(sb.right),
                        bottom: all.bottom.max(sb.bottom),
                    };
                }
                for i in 0..n {
                    let a = alpha[i] as u32;
                    let sv = if cfg.halo { (sh[i] as u32 * 3).min(255) } else { sh[i] as u32 * 150 / 255 };
                    let s = sv * (255 - a) / 255;
                    alpha[i] = (a + s).min(255) as u8;
                    sh[i] = a as u8; // запоминаем «чистый» текст для цвета
                }
                write_pixels(px, &alpha, Some(&sh), text_color(cfg));
            } else {
                write_pixels(px, &alpha, None, text_color(cfg));
            }
            return Some(Frame { dc, bmp, old, bits: bits as *mut u32, w: cw, h: ch, text, all });
        }
        DeleteDC(dc);
        None
    }
}

fn text_color(cfg: &Config) -> u32 {
    if cfg.color == crate::config::COLOR_AUTO { 0xFFFFFF } else { cfg.color }
}

fn luma(c: u32) -> u32 {
    (((c >> 16) & 0xFF) * 299 + ((c >> 8) & 0xFF) * 587 + (c & 0xFF) * 114) / 1000
}

/// Пишет премультиплицированные BGRA-пиксели: цвет текста там, где текст; тень — где тень.
/// Под светлым текстом тень чёрная, под тёмным — светлая (иначе тёмный текст не читается).
fn write_pixels(px: &mut [u32], alpha: &[u8], text_a: Option<&[u8]>, color: u32) {
    let (r, g, b) = ((color >> 16) & 0xFF, (color >> 8) & 0xFF, color & 0xFF);
    let shadow: u32 = if luma(color) < 110 { 235 } else { 0 };
    for i in 0..px.len() {
        let oa = alpha[i] as u32;
        let ta = match text_a {
            Some(t) => t[i] as u32,
            None => oa,
        };
        let sa = oa - ta.min(oa);
        let s = shadow * sa / 255;
        px[i] = (oa << 24) | ((r * ta / 255 + s) << 16) | ((g * ta / 255 + s) << 8) | (b * ta / 255 + s);
    }
}

/// Оставляет от залитых букв в прямоугольнике только обводку толщиной ~k пикселей.
fn outline_region(a: &mut [u8], cw: i32, r: RECT, k: i32) {
    let (w, h) = ((r.right - r.left) as usize, (r.bottom - r.top) as usize);
    if w == 0 || h == 0 {
        return;
    }
    let k = k as usize;
    let mut src = vec![0u8; w * h];
    for y in 0..h {
        let s = (r.top as usize + y) * cw as usize + r.left as usize;
        src[y * w..(y + 1) * w].copy_from_slice(&a[s..s + w]);
    }
    // «сжатие» = минимум по квадрату (2k+1)×(2k+1): сначала по строкам, потом по столбцам
    let mut tmp = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            tmp[y * w + x] = if x < k || x + k >= w { 0 } else { *src[y * w + x - k..=y * w + x + k].iter().min().unwrap() };
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut mn = 255u8;
            if y < k || y + k >= h {
                mn = 0;
            } else {
                for yy in y - k..=y + k {
                    mn = mn.min(tmp[yy * w + x]);
                }
            }
            let i = (r.top as usize + y) * cw as usize + r.left as usize + x;
            a[i] = src[y * w + x].saturating_sub(mn);
        }
    }
}

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (ax + dx * t, ay + dy * t);
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}

/// Рисует стрелочный циферблат со сглаживанием (GDI рисует линии без сглаживания, поэтому считаем сами).
fn draw_dial(a: &mut [u8], cw: i32, cx: f32, cy: f32, r: f32, st: &SysTime, seconds: bool) {
    // отрезки: (x1, y1, x2, y2, полутолщина)
    let mut ticks: Vec<(f32, f32, f32, f32, f32)> = Vec::new();
    for i in 0..60 {
        let five = i % 5 == 0;
        let major = i % 15 == 0;
        let (r0, hw) = if major {
            (0.74, 0.026)
        } else if five {
            (0.8, 0.014)
        } else {
            (0.9, 0.005)
        };
        let (s, c) = (i as f32 * 6.0).to_radians().sin_cos();
        ticks.push((cx + s * r * r0, cy - c * r * r0, cx + s * r * 0.95, cy - c * r * 0.95, (r * hw).max(0.6)));
    }
    let hand = |deg: f32, len: f32, tail: f32, hw: f32| {
        let (s, c) = deg.to_radians().sin_cos();
        (cx - s * r * tail, cy + c * r * tail, cx + s * r * len, cy - c * r * len, (r * hw).max(0.8))
    };
    let sec = st.wSecond as f32;
    let min = st.wMinute as f32 + if seconds { sec / 60.0 } else { 0.0 };
    let hour = (st.wHour % 12) as f32 + min / 60.0;
    let mut hands = vec![hand(hour * 30.0, 0.5, 0.06, 0.032), hand(min * 6.0, 0.78, 0.06, 0.02)];
    if seconds {
        hands.push(hand(sec * 6.0, 0.86, 0.16, 0.007));
    }
    let ri = r.ceil() as i32 + 2;
    let ch = a.len() as i32 / cw;
    for y in (cy as i32 - ri).max(0)..(cy as i32 + ri).min(ch) {
        for x in (cx as i32 - ri).max(0)..(cx as i32 + ri).min(cw) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dc = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            // сглаживание: покрытие = 0.5 − расстояние до края фигуры (в пикселях)
            let mut cov = (0.5 - ((dc - r * 0.99).abs() - (r * 0.006).max(0.6))).clamp(0.0, 1.0); // тонкое кольцо
            cov = cov.max((0.5 - (dc - r * 0.045)).clamp(0.0, 1.0)); // центр
            if dc > r * 0.7 && dc < r {
                for t in &ticks {
                    cov = cov.max((0.5 - (seg_dist(fx, fy, t.0, t.1, t.2, t.3) - t.4)).clamp(0.0, 1.0));
                }
            }
            for t in &hands {
                cov = cov.max((0.5 - (seg_dist(fx, fy, t.0, t.1, t.2, t.3) - t.4)).clamp(0.0, 1.0));
            }
            let i = (y * cw + x) as usize;
            a[i] = a[i].max((cov * 255.0) as u8);
        }
    }
}

fn bbox(a: &[u8], w: i32, h: i32) -> Option<RECT> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, -1, -1);
    for y in 0..h {
        let row = &a[(y * w) as usize..((y + 1) * w) as usize];
        for (x, &v) in row.iter().enumerate() {
            if v > 6 {
                let x = x as i32;
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < 0 {
        None
    } else {
        Some(RECT { left: x0, top: y0, right: x1 + 1, bottom: y1 + 1 })
    }
}

/// Быстрое «прямоугольное» размытие (3 прохода ≈ гауссово).
fn box_blur(buf: &mut [u8], w: i32, h: i32, r: i32) {
    if r <= 0 {
        return;
    }
    let (w, h, r) = (w as usize, h as usize, r as usize);
    let div = (2 * r + 1) as u32;
    let mut tmp = vec![0u8; buf.len()];
    for y in 0..h {
        let row = &buf[y * w..(y + 1) * w];
        let mut sum: u32 = row[..r.min(w)].iter().map(|&v| v as u32).sum();
        for x in 0..w {
            if x + r < w {
                sum += row[x + r] as u32;
            }
            if x > r {
                sum -= row[x - r - 1] as u32;
            }
            tmp[y * w + x] = (sum / div) as u8;
        }
    }
    for x in 0..w {
        let mut sum: u32 = (0..r.min(h)).map(|y| tmp[y * w + x] as u32).sum();
        for y in 0..h {
            if y + r < h {
                sum += tmp[(y + r) * w + x] as u32;
            }
            if y > r {
                sum -= tmp[(y - r - 1) * w + x] as u32;
            }
            buf[y * w + x] = (sum / div) as u8;
        }
    }
}

// ---------------- встраивание в рабочий стол ----------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Host {
    /// Windows 11 24H2+: слой обоев (WorkerW) лежит внутри Progman
    Raised { progman: HWND, defview: HWND },
    /// Старая схема: отдельное окно WorkerW позади иконок
    Classic { workerw: HWND },
    None,
}

unsafe extern "system" fn enum_find(hwnd: HWND, lp: LPARAM) -> windows_sys::Win32::Foundation::BOOL {
    let view = FindWindowExW(hwnd, std::ptr::null_mut(), wide("SHELLDLL_DefView").as_ptr(), std::ptr::null());
    if !view.is_null() {
        let ww = FindWindowExW(std::ptr::null_mut(), hwnd, wide("WorkerW").as_ptr(), std::ptr::null());
        if !ww.is_null() {
            *(lp as *mut HWND) = ww;
            return 0;
        }
    }
    1
}

fn find_host() -> Host {
    unsafe {
        let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
        if progman.is_null() {
            return Host::None;
        }
        // просим Explorer создать отдельный слой под обоями
        let mut res: usize = 0;
        SendMessageTimeoutW(progman, 0x052C, 0xD, 0x1, SMTO_NORMAL, 1000, &mut res);
        SendMessageTimeoutW(progman, 0x052C, 0, 0, SMTO_NORMAL, 1000, &mut res);
        let inner = FindWindowExW(progman, std::ptr::null_mut(), wide("WorkerW").as_ptr(), std::ptr::null());
        if !inner.is_null() {
            let defview = FindWindowExW(progman, std::ptr::null_mut(), wide("SHELLDLL_DefView").as_ptr(), std::ptr::null());
            return Host::Raised { progman, defview };
        }
        let mut ww: HWND = std::ptr::null_mut();
        EnumWindows(Some(enum_find), &mut ww as *mut HWND as LPARAM);
        if !ww.is_null() {
            return Host::Classic { workerw: ww };
        }
        Host::None
    }
}

// ---------------- мониторы ----------------

#[derive(Clone, Copy)]
pub struct Mon {
    pub rect: RECT,
    pub work: RECT,
    pub primary: bool,
}

unsafe extern "system" fn enum_mon(h: HMONITOR, _dc: HDC, _r: *mut RECT, lp: LPARAM) -> windows_sys::Win32::Foundation::BOOL {
    let v = &mut *(lp as *mut Vec<Mon>);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(h, &mut mi) != 0 {
        v.push(Mon { rect: mi.rcMonitor, work: mi.rcWork, primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0 });
    }
    1
}

/// Все мониторы: сначала основной, дальше слева направо. Индекс в этом списке = номер монитора в программе.
pub fn monitors() -> Vec<Mon> {
    let mut v: Vec<Mon> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(enum_mon), &mut v as *mut Vec<Mon> as LPARAM);
    }
    v.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    if v.is_empty() {
        let (r, w) = primary_monitor();
        v.push(Mon { rect: r, work: w, primary: true });
    }
    v
}

/// На каких мониторах показывать часы (индексы в списке monitors()).
pub fn clock_targets(cfg: &Config, mons: &[Mon]) -> Vec<usize> {
    match cfg.clock_monitor.as_str() {
        "all" => (0..mons.len()).collect(),
        "second" => vec![if mons.len() > 1 { 1 } else { 0 }],
        _ => vec![0],
    }
}

// ---------------- окна часов ----------------

// Данные для отрисовки каждого окна часов (отдельно от остального состояния: WM_PAINT может прийти в любой момент)
struct PaintData {
    hwnd: HWND,
    bg_file: std::path::PathBuf, // фон этого монитора (cache\bg-N.raw)
    clock: Vec<u32>,             // премультиплицированные пиксели часов
    rect: RECT,                  // где часы лежат внутри окна
    wx: Vec<u32>,                // пиксели виджета погоды
    wrect: RECT,                 // где лежит виджет погоды
    opacity: u32,
}

thread_local! {
    static PAINT: std::cell::RefCell<Vec<PaintData>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Рисует кусок окна: фон из файла обоев + часы поверх. Память нужна только под этот кусок.
unsafe fn paint_region(hwnd: HWND, hdc: HDC, r: RECT) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return;
    }
    let mdc = CreateCompatibleDC(hdc);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(mdc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
    if bmp.is_null() {
        DeleteDC(mdc);
        return;
    }
    let old = SelectObject(mdc, bmp as _);
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
    PAINT.with(|all| {
        let all = all.borrow();
        let Some(p) = all.iter().find(|p| p.hwnd == hwnd) else {
            px.fill(0);
            return;
        };
        if !crate::bg::read_rect(&p.bg_file, r.left, r.top, w, h, px) {
            px.fill(0);
        }
        // накладываем слои: часы и виджет погоды
        for (layer, c) in [(&p.clock, p.rect), (&p.wx, p.wrect)] {
        let cw = c.right - c.left;
        if cw > 0 && !layer.is_empty() {
            let op = p.opacity.clamp(10, 100);
            let x0 = r.left.max(c.left);
            let x1 = r.right.min(c.right);
            let y0 = r.top.max(c.top);
            let y1 = r.bottom.min(c.bottom);
            for y in y0..y1 {
                for x in x0..x1 {
                    let s = layer[((y - c.top) * cw + (x - c.left)) as usize];
                    let a = (s >> 24) * op / 100;
                    if a == 0 {
                        continue;
                    }
                    let d = &mut px[((y - r.top) * w + (x - r.left)) as usize];
                    let mix = |sh: u32| -> u32 {
                        let sc = ((s >> sh) & 0xFF) * op / 100;
                        let dc = (*d >> sh) & 0xFF;
                        (sc + dc * (255 - a) / 255).min(255)
                    };
                    *d = (mix(16) << 16) | (mix(8) << 8) | mix(0);
                }
            }
        }
        }
    });
    BitBlt(hdc, r.left, r.top, w, h, mdc, 0, 0, SRCCOPY);
    SelectObject(mdc, old);
    DeleteObject(bmp as _);
    DeleteDC(mdc);
}

unsafe extern "system" fn face_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_ERASEBKGND => 1,
        WM_TIMER if wp == ANIM_TIMER => {
            anim_tick(hwnd);
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_region(hwnd, hdc, ps.rcPaint);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------------- анимация смены обоев ----------------
// Поверх окна часов ненадолго кладётся окно-накладка со СТАРЫМИ обоями, а само окно часов сразу
// рисует новые. Накладка исчезает одним из эффектов. Эффекты делает система (прозрачность, положение,
// форма окна), поэтому процессор почти не нагружается и лишней памяти не нужно.

const ANIM_TIMER: usize = 77;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Anim {
    Fade,
    Slide,
    Wipe,
    Circle,
    Blinds,
}

impl Anim {
    /// Эффект по настройке (None — без анимации).
    pub fn from_cfg(s: &str) -> Option<Anim> {
        match s {
            "fade" => Some(Anim::Fade),
            "slide" => Some(Anim::Slide),
            "wipe" => Some(Anim::Wipe),
            "circle" => Some(Anim::Circle),
            "blinds" => Some(Anim::Blinds),
            "random" => {
                let all = [Anim::Fade, Anim::Slide, Anim::Wipe, Anim::Circle, Anim::Blinds];
                Some(all[crate::util::Rng::new().below(all.len())])
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct AnimState {
    hwnd: HWND,
    kind: Anim,
    start: u64,
    dur: u64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

thread_local! {
    static ANIMS: std::cell::RefCell<Vec<AnimState>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn now_ms() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

/// Плавный разгон и торможение.
fn ease(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Один кадр анимации накладки; в конце накладка удаляется.
unsafe fn anim_tick(hwnd: HWND) {
    let Some(s) = ANIMS.with(|a| a.borrow().iter().find(|s| s.hwnd == hwnd).copied()) else {
        KillTimer(hwnd, ANIM_TIMER);
        return;
    };
    let t = ((now_ms().saturating_sub(s.start)) as f32 / s.dur.max(1) as f32).clamp(0.0, 1.0);
    let e = ease(t);
    if t >= 1.0 {
        finish_anim(hwnd);
        return;
    }
    match s.kind {
        Anim::Fade => {
            SetLayeredWindowAttributes(hwnd, 0, (255.0 * (1.0 - e)) as u8, LWA_ALPHA);
        }
        Anim::Slide => {
            // старые обои уезжают влево и бледнеют
            SetWindowPos(hwnd, std::ptr::null_mut(), s.x - (s.w as f32 * e) as i32, s.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            SetLayeredWindowAttributes(hwnd, 0, (255.0 * (1.0 - 0.6 * e)) as u8, LWA_ALPHA);
        }
        Anim::Wipe => {
            // шторка открывает новые обои слева направо
            let rgn = CreateRectRgn((s.w as f32 * e) as i32, 0, s.w, s.h);
            SetWindowRgn(hwnd, rgn, 1);
        }
        Anim::Circle => {
            // новые обои проступают кругом из центра
            let full = CreateRectRgn(0, 0, s.w, s.h);
            let r = ((s.w as f32).hypot(s.h as f32) / 2.0 * e) as i32;
            let (cx, cy) = (s.w / 2, s.h / 2);
            let ell = CreateEllipticRgn(cx - r, cy - r, cx + r, cy + r);
            CombineRgn(full, full, ell, RGN_DIFF);
            DeleteObject(ell as _);
            SetWindowRgn(hwnd, full, 1);
        }
        Anim::Blinds => {
            // жалюзи: полосы старых обоев сужаются
            let n = 10;
            let sh = (s.h + n - 1) / n;
            let vis = (sh as f32 * (1.0 - e)) as i32;
            let rgn = CreateRectRgn(0, 0, 0, 0);
            for i in 0..n {
                let strip = CreateRectRgn(0, i * sh, s.w, i * sh + vis);
                CombineRgn(rgn, rgn, strip, RGN_OR);
                DeleteObject(strip as _);
            }
            SetWindowRgn(hwnd, rgn, 1);
        }
    }
}

unsafe fn finish_anim(hwnd: HWND) {
    KillTimer(hwnd, ANIM_TIMER);
    ANIMS.with(|a| a.borrow_mut().retain(|s| s.hwnd != hwnd));
    PAINT.with(|p| p.borrow_mut().retain(|d| d.hwnd != hwnd));
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
}

/// Окна-накладки, которые сейчас анимируются.
fn anim_windows() -> Vec<HWND> {
    ANIMS.with(|a| a.borrow().iter().map(|s| s.hwnd).collect())
}

/// Окно часов на одном мониторе.
struct Face {
    hwnd: HWND,
    mon: usize,
    rect: RECT,
    visible: bool,
}

pub struct Clock {
    faces: Vec<Face>,
    parent: HWND,
    host: Host,
    fonts: FontStore,
    hinst: HINSTANCE,
    cache: std::path::PathBuf,
    /// true — окна непрозрачные на весь монитор и сами рисуют обои (схема для рабочего стола Windows);
    /// false — прозрачные окна только под часы (запасной вариант, если встроиться не удалось)
    opaque: bool,
    bg_ready: bool,
    /// размытые обои под подложками: (монитор, погода ли) → кэш
    blur_cache: Vec<((usize, bool), BlurCache)>,
    /// окна нужны на всех мониторах (живые обои), даже где нет часов и погоды
    pub all_monitors: bool,
}

/// Окно часов стоит сразу под иконками (между ним и иконками — только другие наши окна)?
unsafe fn right_below(defview: HWND, hwnd: HWND, ours: &[HWND]) -> bool {
    let mut w = GetWindow(defview, GW_HWNDNEXT);
    while !w.is_null() {
        if w == hwnd {
            return true;
        }
        if !ours.contains(&w) {
            return false;
        }
        w = GetWindow(w, GW_HWNDNEXT);
    }
    false
}

impl Clock {
    pub fn new(hinst: HINSTANCE, cache: std::path::PathBuf) -> Self {
        unsafe {
            let cls = wide("WallClockFace");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(face_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinst,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: cls.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
        }
        Clock {
            faces: Vec::new(),
            parent: std::ptr::null_mut(),
            host: Host::None,
            fonts: FontStore::default(),
            hinst,
            cache,
            opaque: false,
            bg_ready: false,
            blur_cache: Vec::new(),
            all_monitors: false,
        }
    }

    /// Окна живы, встроены в рабочий стол и стоят ровно на нужных мониторах?
    fn alive(&self, want: &[(usize, RECT)]) -> bool {
        unsafe {
            if self.faces.len() != want.len() {
                return false;
            }
            if !self.parent.is_null() && IsWindow(self.parent) == 0 {
                return false;
            }
            self.faces.iter().zip(want).all(|(f, (m, r))| {
                f.mon == *m
                    && crate::util::same_rect(&f.rect, r)
                    && IsWindow(f.hwnd) != 0
                    && (self.parent.is_null() || GetParent(f.hwnd) == self.parent)
            })
        }
    }

    /// Создаёт окно и встраивает его в рабочий стол (сразу под иконки).
    fn new_child(&self) -> Option<HWND> {
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                wide("WallClockFace").as_ptr(),
                wide("Hora").as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                self.hinst,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                log("не удалось создать окно часов");
                return None;
            }
            match self.host {
                Host::Raised { progman, defview } => {
                    // непрозрачное «слоёное» окно: так его рисует новый рабочий стол Windows 11
                    SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                    SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_CHILD | WS_CLIPSIBLINGS) as isize);
                    SetParent(hwnd, progman);
                    let after = if defview.is_null() { HWND_TOP } else { defview };
                    SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                Host::Classic { workerw } => {
                    SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                    SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_CHILD | WS_CLIPSIBLINGS) as isize);
                    SetParent(hwnd, workerw);
                }
                Host::None => {
                    SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
            Some(hwnd)
        }
    }

    fn create_face(&mut self, mon: usize, rect: RECT) -> Option<Face> {
        let hwnd = self.new_child()?;
        PAINT.with(|p| {
            p.borrow_mut().push(PaintData {
                hwnd,
                bg_file: crate::bg::file_for(&self.cache, mon),
                clock: Vec::new(),
                rect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                wx: Vec::new(),
                wrect: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                opacity: 100,
            })
        });
        Some(Face { hwnd, mon, rect, visible: false })
    }

    /// Создаёт окна часов на нужных мониторах (или пересоздаёт, если Explorer перезапускался / мониторы поменялись).
    fn ensure(&mut self, want: &[(usize, RECT)]) {
        if self.alive(want) {
            return;
        }
        self.destroy();
        self.host = find_host();
        let (parent, opaque) = match self.host {
            Host::Raised { progman, .. } => (progman, true),
            Host::Classic { workerw } => (workerw, true),
            Host::None => (std::ptr::null_mut(), false),
        };
        self.parent = parent;
        self.opaque = opaque;
        for (m, r) in want {
            if let Some(f) = self.create_face(*m, *r) {
                self.faces.push(f);
            }
        }
        log(&format!("часы встроены: {:?}, мониторов с часами: {}", self.host, self.faces.len()));
    }

    pub fn destroy(&mut self) {
        unsafe {
            for h in anim_windows() {
                finish_anim(h);
            }
            for f in &self.faces {
                if IsWindow(f.hwnd) != 0 {
                    DestroyWindow(f.hwnd);
                }
            }
        }
        self.faces.clear();
        PAINT.with(|p| p.borrow_mut().clear());
        self.parent = std::ptr::null_mut();
    }

    pub fn hide(&mut self) {
        for f in &mut self.faces {
            unsafe { ShowWindow(f.hwnd, SW_HIDE) };
            f.visible = false;
        }
    }

    /// Фон (подготовленные обои) обновился у мониторов из маски — перерисовать окна,
    /// при желании с анимацией перехода со старых обоев.
    /// Окно рабочего стола (Progman), внутри которого живут часы, — чтобы следить за перестановками в нём.
    pub fn host_window(&self) -> HWND {
        match self.host {
            Host::Raised { progman, .. } => progman,
            Host::Classic { workerw } => workerw,
            Host::None => std::ptr::null_mut(),
        }
    }

    /// Explorer при смене обоев кладёт свой слой обоев поверх часов. Возвращаем часы сразу под иконки.
    /// Возвращает true, если пришлось что-то переставить.
    pub fn fix_z(&mut self) -> bool {
        let Host::Raised { defview, .. } = self.host else { return false };
        if defview.is_null() {
            return false;
        }
        let mut ours: Vec<HWND> = self.faces.iter().map(|f| f.hwnd).collect();
        ours.extend(anim_windows());
        let mut moved = false;
        unsafe {
            // окна-накладки анимации должны быть выше окон часов, поэтому их ставим последними (сразу под иконки)
            for &h in ours.iter() {
                if IsWindow(h) != 0 && !right_below(defview, h, &ours) {
                    SetWindowPos(h, defview, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                    moved = true;
                }
            }
            if moved {
                for &h in anim_windows().iter() {
                    SetWindowPos(h, defview, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
        }
        moved
    }

    pub fn bg_updated(&mut self, mask: usize, anim: Option<(Anim, u32)>) {
        self.bg_ready = true;
        if !self.opaque {
            return;
        }
        for i in 0..self.faces.len() {
            let f = &self.faces[i];
            if mask & (1 << f.mon.min(31)) == 0 || !f.visible {
                continue;
            }
            if let Some((kind, ms)) = anim {
                self.start_anim(i, kind, ms);
            }
            let f = &self.faces[i];
            unsafe {
                InvalidateRect(f.hwnd, std::ptr::null(), 0);
                UpdateWindow(f.hwnd);
            }
        }
    }

    /// Кладёт поверх окна часов накладку со старыми обоями и запускает её исчезновение.
    fn start_anim(&mut self, i: usize, kind: Anim, ms: u32) {
        let face = self.faces[i].hwnd;
        let prev = crate::bg::prev_for(&self.cache, self.faces[i].mon);
        if !prev.is_file() {
            return;
        }
        let Some(ov) = self.new_child() else { return };
        unsafe {
            let mut wr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(face, &mut wr);
            let mut org = POINT { x: wr.left, y: wr.top };
            if !self.parent.is_null() {
                ScreenToClient(self.parent, &mut org);
            }
            let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
            // накладка: прошлый фон + те же часы, что сейчас на экране
            let z = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let (clock, rect, wx, wrect, opacity) = PAINT.with(|p| {
                p.borrow().iter().find(|d| d.hwnd == face).map(|d| (d.clock.clone(), d.rect, d.wx.clone(), d.wrect, d.opacity)).unwrap_or((Vec::new(), z, Vec::new(), z, 100))
            });
            PAINT.with(|p| p.borrow_mut().push(PaintData { hwnd: ov, bg_file: prev, clock, rect, wx, wrect, opacity }));
            // сразу под иконками — то есть над окном часов
            let after = match self.host {
                Host::Raised { defview, .. } if !defview.is_null() => defview,
                _ => HWND_TOP,
            };
            SetWindowPos(ov, after, org.x, org.y, w, h, SWP_NOACTIVATE);
            ShowWindow(ov, SW_SHOWNOACTIVATE);
            UpdateWindow(ov);
            ANIMS.with(|a| a.borrow_mut().push(AnimState { hwnd: ov, kind, start: now_ms(), dur: ms as u64, x: org.x, y: org.y, w, h }));
            SetTimer(ov, ANIM_TIMER, 15, None);
        }
    }

    /// Окна часов для живых обоев: окно, монитор и (для видео) картинки часов и погоды,
    /// которые надо положить поверх кадров.
    pub fn live_faces(&self, with_layers: bool) -> Vec<crate::live::Face> {
        if !self.opaque || !self.bg_ready {
            return Vec::new();
        }
        self.faces
            .iter()
            .filter(|f| f.visible)
            .map(|f| {
                let layers = if with_layers {
                    PAINT.with(|p| {
                        p.borrow()
                            .iter()
                            .find(|d| d.hwnd == f.hwnd)
                            .map(|d| {
                                let op = d.opacity.clamp(10, 100) as f32 / 100.0;
                                [(&d.clock, d.rect), (&d.wx, d.wrect)]
                                    .into_iter()
                                    .filter(|(px, r)| !px.is_empty() && r.right > r.left)
                                    .map(|(px, r)| crate::live::Layer { px: std::sync::Arc::new(px.clone()), rect: (r.left, r.top, r.right, r.bottom), opacity: op })
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                } else {
                    Vec::new()
                };
                crate::live::Face { hwnd: f.hwnd as isize, rect: (f.rect.left, f.rect.top, f.rect.right, f.rect.bottom), layers }
            })
            .collect()
    }

    /// Перерисовывает часы и виджет погоды на всех выбранных мониторах.
    pub fn render(&mut self, cfg: &Config, weather: Option<&crate::weather::Weather>) {
        let mons = monitors();
        let clock_on: Vec<usize> = if cfg.clock { clock_targets(cfg, &mons) } else { Vec::new() };
        let weather_on: Vec<usize> = if cfg.weather && weather.is_some() { weather_targets(cfg, &mons) } else { Vec::new() };
        let mut idx: Vec<usize> = clock_on.iter().chain(weather_on.iter()).copied().collect();
        idx.sort_unstable();
        idx.dedup();
        if self.all_monitors {
            idx = (0..mons.len()).collect();
        }
        if idx.is_empty() {
            self.hide();
            return;
        }
        let want: Vec<(usize, RECT)> = idx.iter().map(|&i| (i, mons[i].rect)).collect();
        self.ensure(&want);
        let mut st: SysTime = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
        // «наши» окна над часами — это и накладки анимации: их не считаем чужими при проверке порядка
        let mut ours: Vec<HWND> = self.faces.iter().map(|f| f.hwnd).collect();
        ours.extend(anim_windows());
        for i in 0..self.faces.len() {
            let mi = self.faces[i].mon;
            let w = if weather_on.contains(&mi) { weather } else { None };
            self.render_face(i, cfg, &st, mons[mi], &ours, clock_on.contains(&mi), w);
        }
        // кэш размытия под слоями, которые больше не рисуются (часы убрали с монитора, погоду выключили), — не держим
        self.blur_cache.retain(|((m, wx), _)| if *wx { weather_on.contains(m) } else { clock_on.contains(m) });
    }

    /// Готовит один слой (часы или погоду) для монитора: пиксели и место внутри окна монитора.
    fn layer(&mut self, cfg: &Config, st: &SysTime, m: Mon, mon_idx: usize, weather: Option<&crate::weather::Weather>) -> Option<(Vec<u32>, RECT)> {
        let (mon, work) = (m.rect, m.work);
        let area = (mon.right - mon.left, mon.bottom - mon.top);
        let make = |c: &Config, fonts: &mut FontStore| match weather {
            Some(w) => compose_weather(c, st, area, fonts, w),
            None => compose(c, st, area, fonts),
        };
        let mut fr = make(cfg, &mut self.fonts)?;
        let where_ = |fr: &Frame| match weather {
            Some(_) => place_at(fr, area, work, &cfg.weather_position, 0, 0),
            None => place(cfg, fr, area, work),
        };
        let (mut x, mut y) = where_(&fr);
        let file = crate::bg::file_for(&self.cache, mon_idx);
        let can_read = self.bg_ready && self.opaque;
        // что под текстом: доли светлого и тёмного (нужно для «авто»-цвета и «авто»-подложки)
        let (light, dark) = if can_read && (cfg.color == crate::config::COLOR_AUTO || cfg.backdrop == "auto") {
            let tl = (x - mon.left + (fr.text.left - fr.all.left), y - mon.top + (fr.text.top - fr.all.top));
            crate::bg::luma_split(&file, tl.0, tl.1, fr.text.right - fr.text.left, fr.text.bottom - fr.text.top).unwrap_or((0.0, 1.0))
        } else {
            (0.0, 1.0)
        };
        let mixed = light > 0.15 && dark > 0.15;
        let plate_on = can_read && (cfg.backdrop == "on" || (cfg.backdrop == "auto" && mixed));
        // цвет «авто»: с подложкой текст белый; без неё — тёмный на светлом, белый с ореолом на пёстром
        if cfg.color == crate::config::COLOR_AUTO && can_read && !plate_on {
            let redo = if light > 0.7 {
                // почти всё под текстом светлое — тёмный текст
                let mut c = cfg.clone();
                c.color = 0x1C1E22;
                Some(c)
            } else if light > 0.15 && dark > 0.15 {
                // пёстрый фон (и светлое, и тёмное) — белый текст с плотным тёмным ореолом, как у субтитров
                let mut c = cfg.clone();
                c.halo = true;
                c.shadow = true;
                Some(c)
            } else {
                None
            };
            if let Some(c) = redo {
                if let Some(f2) = make(&c, &mut self.fonts) {
                    fr = f2;
                    // у нового кадра другие поля (ореол, тень) — место считаем заново, чтобы текст не сдвинулся
                    (x, y) = where_(&fr);
                }
            }
        }
        // подложка: тёмное стекло под светлым текстом, светлое — под тёмным
        let text_dark = cfg.color != crate::config::COLOR_AUTO && luma(cfg.color) < 110;
        let plate = if plate_on { Some(Plate::for_height(area.1, !text_dark, cfg.backdrop_opacity)) } else { None };
        let reader = |rx: i32, ry: i32, rw: i32, rh: i32, out: &mut [u32]| crate::bg::read_rect(&file, rx, ry, rw, rh, out);
        // Кэш размытых обоев держим, только когда он окупается: часы обновляются каждую секунду
        // или открыто окно настроек (там щелчки часто перерисовывают часы). Иначе — память дороже.
        let slot = (mon_idx, weather.is_some());
        let keep = plate.is_some() && (cfg.seconds || crate::ui::is_open());
        if !keep {
            self.blur_cache.retain(|(s, _)| *s != slot);
            if self.blur_cache.is_empty() {
                self.blur_cache.shrink_to_fit();
            }
        }
        let cache = if keep {
            // «какие обои»: время изменения и размер файла фона — сменились обои, сменились и они
            let id = std::fs::metadata(&file)
                .map(|m| {
                    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
                    t ^ m.len().rotate_left(32)
                })
                .unwrap_or(0);
            let i = match self.blur_cache.iter().position(|(s, _)| *s == slot) {
                Some(i) => i,
                None => {
                    self.blur_cache.push((slot, BlurCache::default()));
                    self.blur_cache.len() - 1
                }
            };
            Some((&mut self.blur_cache[i].1, id))
        } else {
            None
        };
        // координаты слоя — внутри монитора (как и файл фона)
        Some(make_layer(&fr, x - mon.left, y - mon.top, plate.as_ref(), &reader, cache))
    }

    fn render_face(&mut self, i: usize, cfg: &Config, st: &SysTime, m: Mon, ours: &[HWND], show_clock: bool, weather: Option<&crate::weather::Weather>) {
        let mon = m.rect;
        let hwnd = self.faces[i].hwnd;
        let area = (mon.right - mon.left, mon.bottom - mon.top);
        unsafe {
            // Explorer при смене обоев может поднять свой слой поверх часов — ставим их сразу под иконки
            let mut insert_after: HWND = std::ptr::null_mut();
            let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
            if let Host::Raised { defview, .. } = self.host {
                if !defview.is_null() && !right_below(defview, hwnd, ours) {
                    insert_after = defview;
                    flags = SWP_NOACTIVATE;
                }
            }

            if self.opaque {
                // окно = весь монитор; часы и погода — картинки внутри него
                let mut org = POINT { x: mon.left, y: mon.top };
                ScreenToClient(self.parent, &mut org);
                SetWindowPos(hwnd, insert_after, org.x, org.y, area.0, area.1, flags);
                let mi = self.faces[i].mon;
                let z = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                let (clock, rect) = if show_clock { self.layer(cfg, st, m, mi, None).unwrap_or((Vec::new(), z)) } else { (Vec::new(), z) };
                let (wx, wrect) = match weather {
                    Some(w) => self.layer(cfg, st, m, mi, Some(w)).unwrap_or((Vec::new(), z)),
                    None => (Vec::new(), z),
                };
                let (old, wold) = PAINT.with(|all| {
                    let mut all = all.borrow_mut();
                    match all.iter_mut().find(|p| p.hwnd == hwnd) {
                        Some(p) => {
                            let old = (p.rect, p.wrect);
                            p.clock = clock;
                            p.rect = rect;
                            p.wx = wx;
                            p.wrect = wrect;
                            p.opacity = cfg.opacity;
                            old
                        }
                        None => (rect, wrect),
                    }
                });
                if !self.bg_ready {
                    return; // покажемся, когда фон будет готов
                }
                let f = &mut self.faces[i];
                if !f.visible {
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    f.visible = true;
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                } else {
                    for r in [old, rect, wold, wrect] {
                        if r.right > r.left {
                            InvalidateRect(hwnd, &r, 0);
                        }
                    }
                }
                UpdateWindow(hwnd);
                return;
            }

            // запасной вариант (если встроиться в рабочий стол не удалось): прозрачное окно только под часы
            if !show_clock {
                ShowWindow(hwnd, SW_HIDE);
                self.faces[i].visible = false;
                return;
            }
            let Some(fr) = compose(cfg, st, area, &mut self.fonts) else { return };
            let (x, y) = place(cfg, &fr, area, m.work);
            let size = SIZE { cx: fr.all.right - fr.all.left, cy: fr.all.bottom - fr.all.top };
            let mut pt = POINT { x, y };
            if !self.parent.is_null() {
                ScreenToClient(self.parent, &mut pt);
            }
            SetWindowPos(hwnd, insert_after, pt.x, pt.y, size.cx, size.cy, flags);
            let src = POINT { x: fr.all.left, y: fr.all.top };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: (cfg.opacity.clamp(10, 100) * 255 / 100) as u8,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let ok = UpdateLayeredWindow(hwnd, std::ptr::null_mut(), std::ptr::null(), &size, fr.dc, &src, 0, &blend, ULW_ALPHA);
            if ok == 0 {
                log(&format!("UpdateLayeredWindow ошибка {}", GetLastError()));
            }
            let f = &mut self.faces[i];
            if !f.visible {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                f.visible = true;
            }
        }
    }
}

// ---------------- подложка «матовое стекло» ----------------

/// Подложка под текстом: размытая копия обоев, затемнённая (или осветлённая), со скруглёнными краями.
pub struct Plate {
    pub dark: bool,  // true — тёмное стекло (под светлым текстом), false — светлое
    pub pad: i32,    // отступ от текста до края подложки
    pub radius: i32, // скругление углов
    pub blur: i32,   // сила размытия обоев под подложкой
    pub alpha: u32,  // непрозрачность 0..255
}

impl Plate {
    /// Размеры подложки — от высоты монитора (или картинки предпросмотра).
    pub fn for_height(h: i32, dark: bool, opacity_pct: u32) -> Plate {
        Plate {
            dark,
            pad: (h as f32 * 0.03) as i32,
            radius: (h as f32 * 0.028) as i32,
            blur: ((h as f32 * 0.012) as i32).max(3),
            alpha: opacity_pct.clamp(10, 100) * 255 / 100,
        }
    }
}

/// Доли светлых и тёмных точек в куске картинки (по каждой 4-й точке).
pub fn split_of(px: &[u32]) -> (f32, f32) {
    let (mut light, mut dark, mut n) = (0u32, 0u32, 0u32);
    for p in px.iter().step_by(4) {
        let l = luma(*p & 0xFF_FFFF);
        if l > 165 {
            light += 1;
        } else if l < 90 {
            dark += 1;
        }
        n += 1;
    }
    if n == 0 { (0.0, 0.0) } else { (light as f32 / n as f32, dark as f32 / n as f32) }
}

/// Размытые обои под подложкой. Считаются с запасом вокруг, поэтому, пока обои те же,
/// а часы не уехали за запас (ширина текста чуть меняется каждую секунду), размытие не пересчитывается.
pub struct BlurCache {
    id: u64,      // какие обои (время изменения и размер файла фона)
    blur: i32,    // сила размытия
    inner: RECT,  // где размытие точное
    area: RECT,   // что лежит в px: inner плюс краевой запас для размытия
    px: Vec<u32>, // размытые обои, 0x00RRGGBB
}

impl Default for BlurCache {
    fn default() -> Self {
        let z = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        BlurCache { id: 0, blur: 0, inner: z, area: z, px: Vec::new() }
    }
}

fn contains(a: &RECT, b: &RECT) -> bool {
    a.left <= b.left && a.top <= b.top && a.right >= b.right && a.bottom >= b.bottom
}

/// Размыть обои в прямоугольнике inner. Читаем с запасом 3·blur вокруг: три прохода размытия
/// «дотягиваются» до 3·blur, и у краёв берутся настоящие обои, а не чернота (без тёмной каймы).
fn blur_area(read_bg: &dyn Fn(i32, i32, i32, i32, &mut [u32]) -> bool, inner: RECT, blur: i32, id: u64) -> BlurCache {
    let m = blur * 3;
    let area = RECT { left: inner.left - m, top: inner.top - m, right: inner.right + m, bottom: inner.bottom + m };
    let (bw, bh) = (area.right - area.left, area.bottom - area.top);
    let mut px = vec![0u32; (bw * bh).max(0) as usize];
    if !read_bg(area.left, area.top, bw, bh, &mut px) {
        px.fill(0x0030_3036);
    }
    let mut ch: [Vec<u8>; 3] = [vec![0; px.len()], vec![0; px.len()], vec![0; px.len()]];
    for (i, v) in px.iter().enumerate() {
        ch[0][i] = ((v >> 16) & 0xFF) as u8;
        ch[1][i] = ((v >> 8) & 0xFF) as u8;
        ch[2][i] = (v & 0xFF) as u8;
    }
    for c in ch.iter_mut() {
        for _ in 0..3 {
            box_blur(c, bw, bh, blur);
        }
    }
    for (i, v) in px.iter_mut().enumerate() {
        *v = ((ch[0][i] as u32) << 16) | ((ch[1][i] as u32) << 8) | ch[2][i] as u32;
    }
    BlurCache { id, blur, inner, area, px }
}

/// Готовый слой: (подложка, если нужна) + кадр с текстом. Координаты — те же, в которых дан (x, y)
/// и в которых читает фон read_bg(x, y, w, h, out). Возвращает пиксели (премультиплицированные) и прямоугольник.
/// cache — (кэш размытия, «какие обои»): если дан, размытие берётся из него и обновляется только при нужде.
pub fn make_layer(fr: &Frame, x: i32, y: i32, plate: Option<&Plate>, read_bg: &dyn Fn(i32, i32, i32, i32, &mut [u32]) -> bool, cache: Option<(&mut BlurCache, u64)>) -> (Vec<u32>, RECT) {
    let (aw, ah) = (fr.all.right - fr.all.left, fr.all.bottom - fr.all.top);
    let frame = RECT { left: x, top: y, right: x + aw, bottom: y + ah };
    let (tw, th) = (fr.text.right - fr.text.left, fr.text.bottom - fr.text.top);
    let tx = x + (fr.text.left - fr.all.left);
    let ty = y + (fr.text.top - fr.all.top);
    let pr = plate.map(|p| RECT { left: tx - p.pad, top: ty - p.pad, right: tx + tw + p.pad, bottom: ty + th + p.pad });
    let u = match pr {
        Some(r) => RECT { left: frame.left.min(r.left), top: frame.top.min(r.top), right: frame.right.max(r.right), bottom: frame.bottom.max(r.bottom) },
        None => frame,
    };
    let (uw, uh) = (u.right - u.left, u.bottom - u.top);
    let mut out = vec![0u32; (uw * uh).max(0) as usize];

    // 1) подложка: размытые обои + оттенок, края скруглены со сглаживанием
    if let (Some(p), Some(r)) = (plate, pr) {
        let (pw, ph) = (r.right - r.left, r.bottom - r.top);
        let fresh;
        let bc: &BlurCache = match cache {
            Some((c, id)) => {
                if c.id != id || c.blur != p.blur || c.px.is_empty() || !contains(&c.inner, &r) {
                    // промах: размываем с запасом в отступ подложки со всех сторон
                    let s = p.pad.max(8);
                    *c = blur_area(read_bg, RECT { left: r.left - s, top: r.top - s, right: r.right + s, bottom: r.bottom + s }, p.blur, id);
                }
                c
            }
            None => {
                fresh = blur_area(read_bg, r, p.blur, 0);
                &fresh
            }
        };
        let bw = bc.area.right - bc.area.left;
        let (ox, oy) = (r.left - bc.area.left, r.top - bc.area.top);
        let rad = p.radius.min(pw / 2).min(ph / 2) as f32;
        let (hw, hh) = (pw as f32 / 2.0, ph as f32 / 2.0);
        for yy in 0..ph {
            for xx in 0..pw {
                // покрытие скруглённого прямоугольника (0..1) — для мягких краёв
                let qx = ((xx as f32 + 0.5 - hw).abs() - (hw - rad)).max(0.0);
                let qy = ((yy as f32 + 0.5 - hh).abs() - (hh - rad)).max(0.0);
                let cov = (0.5 - ((qx * qx + qy * qy).sqrt() - rad)).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let v = bc.px[((yy + oy) * bw + xx + ox) as usize];
                let tint = |v: u32| -> u32 {
                    let v = (v & 0xFF) as f32;
                    (if p.dark { v * 0.56 } else { v * 0.58 + 105.0 }).clamp(0.0, 255.0) as u32
                };
                // прозрачность подложки — из настроек (сквозь неё видны чёткие обои)
                let a = (cov * p.alpha as f32) as u32;
                let (rr, gg, bb) = (tint(v >> 16) * a / 255, tint(v >> 8) * a / 255, tint(v) * a / 255);
                let o = ((r.top - u.top + yy) * uw + (r.left - u.left + xx)) as usize;
                out[o] = (a << 24) | (rr << 16) | (gg << 8) | bb;
            }
        }
    }

    // 2) поверх — кадр с текстом (тоже премультиплицированный)
    let src = unsafe { std::slice::from_raw_parts(fr.bits, (fr.w * fr.h) as usize) };
    for row in 0..ah {
        for col in 0..aw {
            let s = src[((fr.all.top + row) * fr.w + fr.all.left + col) as usize];
            let sa = s >> 24;
            if sa == 0 {
                continue;
            }
            let o = ((frame.top - u.top + row) * uw + (frame.left - u.left + col)) as usize;
            let d = out[o];
            let mix = |sh: u32| -> u32 { (((s >> sh) & 0xFF) + ((d >> sh) & 0xFF) * (255 - sa) / 255).min(255) };
            out[o] = (mix(24) << 24) | (mix(16) << 16) | (mix(8) << 8) | mix(0);
        }
    }
    (out, u)
}

/// Где поставить картинку часов (левый верхний угол всего кадра с тенью) — по видимому тексту,
/// внутри рабочей области монитора `work`; `area` — размеры монитора.
pub fn place(cfg: &Config, fr: &Frame, area: (i32, i32), work: RECT) -> (i32, i32) {
    place_at(fr, area, work, &cfg.position, cfg.offset_x, cfg.offset_y)
}

/// То же для произвольного положения (виджет погоды ставится отдельно от часов).
pub fn place_at(fr: &Frame, area: (i32, i32), work: RECT, position: &str, offset_x: i32, offset_y: i32) -> (i32, i32) {
    let tw = fr.text.right - fr.text.left;
    let th = fr.text.bottom - fr.text.top;
    let (ww, wh) = (work.right - work.left, work.bottom - work.top);
    let margin = (area.1 as f32 * 0.07) as i32;
    let p = position;
    let mut x = if p.contains("left") {
        work.left + margin
    } else if p.contains("right") {
        work.right - margin - tw
    } else {
        work.left + (ww - tw) / 2
    };
    let mut y = if p.starts_with("top") {
        work.top + margin
    } else if p.starts_with("bottom") {
        work.bottom - margin - th
    } else {
        work.top + (wh - th) / 2
    };
    x += offset_x * area.0 / 100 - (fr.text.left - fr.all.left);
    y += offset_y * area.1 / 100 - (fr.text.top - fr.all.top);
    (x, y)
}

/// Сколько миллисекунд до следующей перерисовки (ровно на смене минуты/секунды).
pub fn ms_to_next_tick(cfg: &Config) -> u32 {
    let mut st: SysTime = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    let ms = st.wMilliseconds as u32;
    if cfg.seconds {
        1000 - ms + 15
    } else {
        (59 - st.wSecond as u32) * 1000 + (1000 - ms) + 30
    }
}

/// Возвращает (весь монитор, рабочая область без панели задач) основного монитора.
pub fn primary_monitor() -> (RECT, RECT) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut mi) != 0 {
            (mi.rcMonitor, mi.rcWork)
        } else {
            let r = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
            (r, r)
        }
    }
}

/// Для проверки дизайна: рисует часы (или погоду) поверх серого градиента и сохраняет в BMP.
pub fn render_test(cfg: &Config, out: &std::path::Path, area: (i32, i32), weather: Option<&crate::weather::Weather>) -> bool {
    let mut fonts = FontStore::default();
    let mut st: SysTime = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    let fr = match weather {
        Some(w) => compose_weather(cfg, &st, area, &mut fonts, w),
        None => compose(cfg, &st, area, &mut fonts),
    };
    let Some(fr) = fr else { return false };
    let pad = area.1 / 20;
    let w = (fr.all.right - fr.all.left) + pad * 2;
    let h = (fr.all.bottom - fr.all.top) + pad * 2;
    let src = unsafe { std::slice::from_raw_parts(fr.bits, (fr.w * fr.h) as usize) };
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    let row_pad = (4 - (w * 3) % 4) % 4;
    for yy in (0..h).rev() {
        for xx in 0..w {
            // фон: от светло-серого к тёмному, как на примере
            let bg = if cfg.halo {
                // пёстрый фон для проверки ореола: крупная чёрно-белая шахматка
                if (xx / 90 + yy / 70) % 2 == 0 { 12 } else { 238 }
            } else {
                (190.0 - 150.0 * (xx as f32 / w as f32 * 0.6 + yy as f32 / h as f32 * 0.4)) as u32
            };
            let sx = xx - pad + fr.all.left;
            let sy = yy - pad + fr.all.top;
            let (mut r, mut g, mut b) = (bg, bg, bg);
            if sx >= 0 && sy >= 0 && sx < fr.w && sy < fr.h {
                let p = src[(sy * fr.w + sx) as usize];
                let a = p >> 24;
                let op = cfg.opacity;
                let a2 = a * op / 100;
                let pr = ((p >> 16) & 0xFF) * op / 100;
                let pg = ((p >> 8) & 0xFF) * op / 100;
                let pb = (p & 0xFF) * op / 100;
                r = pr + bg * (255 - a2) / 255;
                g = pg + bg * (255 - a2) / 255;
                b = pb + bg * (255 - a2) / 255;
            }
            data.push(b.min(255) as u8);
            data.push(g.min(255) as u8);
            data.push(r.min(255) as u8);
        }
        data.extend(std::iter::repeat(0u8).take(row_pad as usize));
    }
    let mut f = Vec::new();
    let file_size = 54 + data.len() as u32;
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&file_size.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24u16.to_le_bytes());
    f.extend_from_slice(&[0u8; 24]);
    f.extend_from_slice(&data);
    std::fs::write(out, f).is_ok()
}
