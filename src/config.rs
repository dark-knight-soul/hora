// Настройки: простой INI-файл config.ini рядом с exe.
// Файл можно править руками — программа сама заметит изменения и применит их.
use std::path::Path;

/// Особое значение цвета: подбирать светлый/тёмный текст по яркости обоев под часами.
pub const COLOR_AUTO: u32 = 0xFF00_0000;

#[derive(Clone, PartialEq)]
pub struct Config {
    // --- обои ---
    pub source: String,     // wallhaven | folder | both
    pub interval_min: u32,  // 0 = не менять автоматически
    pub paused: bool,
    pub wall_mode: String,  // same (одинаковые) | per_monitor (разные) | span (единые на все экраны)
    pub folder: String,     // пусто = папка wallpapers рядом с программой
    pub topics: String,     // выбранные категории через запятую (nature,space…); пусто = query
    pub schedule: bool,          // свои категории на каждое время суток
    pub schedule_hours: String,  // когда начинаются утро, день, вечер и ночь: 6,12,18,23
    pub topics_slot: [String; 4], // категории утра, дня, вечера и ночи (пусто — как основные)
    pub query: String,      // теги для поиска на Wallhaven
    pub sorting: String,    // toplist | random
    pub top_range: String,  // 1d 3d 1w 1M 3M 6M 1y
    pub categories: String, // general/anime/people, например 100
    pub atleast: String,    // минимальное разрешение, auto = как у монитора
    pub ratios: String,     // пропорции, auto = как у монитора
    pub fit: String,        // fill | fit | stretch | center | tile | span
    pub cache_keep: u32,
    pub favorites_mix: u32, // как часто показывать избранное среди новых обоев, % (0 — никогда)
    pub anim: String,  // none | fade | slide | wipe | circle | blinds | random
    pub anim_ms: u32,  // длительность анимации, мс
    // --- часы ---
    pub clock: bool,
    pub clock_monitor: String, // primary | second | all
    pub style: String,    // hero | photo | minimal | pixel | block | split | vertical | words | calendar | outline | poster | line | techno | analog
    pub font: String,     // auto | имя шрифта для крупного текста
    pub font2: String,    // auto | имя шрифта для мелкого текста
    pub color: u32,       // 0xRRGGBB или COLOR_AUTO
    pub opacity: u32,     // 10..100
    pub scale: u32,       // проценты, 100 = обычный размер
    pub position: String, // center | top | bottom | left | right | top-left | ...
    pub offset_x: i32,    // сдвиг в процентах ширины экрана
    pub offset_y: i32,    // сдвиг в процентах высоты экрана
    pub seconds: bool,
    pub lang: String,        // ru | en
    pub time_format: String, // auto | 24 | 12
    pub shadow: bool,
    pub backdrop: String, // подложка под часами: off | on | auto (только на пёстром фоне)
    pub backdrop_opacity: u32, // непрозрачность подложки, % (100 — сплошная)
    pub ui_theme: String, // тема окна настроек: auto | light | dark
    pub ui_lang: String,  // язык окна и меню: auto (как в Windows) | ru | en
    pub update_check: bool, // раз в день проверять, нет ли новой версии
    /// Не настройка, а подсказка при рисовании: фон под текстом пёстрый — нужен плотный ореол.
    pub halo: bool,
    // --- погода ---
    pub weather: bool,
    pub weather_city: String,
    pub weather_lat: f64,
    pub weather_lon: f64,
    pub weather_position: String, // как position у часов
    pub weather_monitor: String,  // primary | second | all
    pub weather_scale: u32,       // проценты
    // --- живые обои ---
    pub live: String,         // off | effects | video
    pub live_effect: String,  // snow | rain | stars | leaves | fireflies | weather
    pub live_density: u32,    // 1 мало, 2 средне, 3 много
    pub live_battery: bool,   // замирать при работе от батареи
    pub video_source: String, // both | folder | we (Wallpaper Engine)
    pub video_folder: String, // своя папка с видео (пусто = videos рядом с программой)
    pub video: String,        // какое видео сейчас
    pub video_rotate: bool,   // менять видео с тем же интервалом, что и обои
}

impl Default for Config {
    fn default() -> Self {
        Config {
            source: "wallhaven".into(),
            interval_min: 30,
            paused: false,
            wall_mode: "same".into(),
            folder: String::new(),
            topics: String::new(),
            schedule: false,
            schedule_hours: "6,12,18,23".into(),
            // утром — светлая природа, вечером — закаты, ночью — тёмное и космос; днём — основные
            topics_slot: ["nature,forest,mountains".into(), String::new(), "sunset,sea,city".into(), "space,dark,minimalism".into()],
            query: String::new(),
            sorting: "toplist".into(),
            top_range: "1M".into(),
            categories: "100".into(),
            atleast: "auto".into(),
            ratios: "auto".into(),
            fit: "fill".into(),
            cache_keep: 30,
            favorites_mix: 20,
            anim: "fade".into(),
            anim_ms: 900,
            clock: true,
            clock_monitor: "primary".into(),
            style: "hero".into(),
            font: "auto".into(),
            font2: "auto".into(),
            color: 0xFFFFFF,
            opacity: 100,
            scale: 100,
            position: "center".into(),
            offset_x: 0,
            offset_y: -4,
            seconds: false,
            lang: if crate::i18n::system_ru() { "ru".into() } else { "en".into() },
            time_format: "auto".into(),
            shadow: true,
            backdrop: "off".into(),
            backdrop_opacity: 80,
            ui_theme: "auto".into(),
            ui_lang: "auto".into(),
            update_check: true,
            halo: false,
            weather: true,
            weather_city: String::new(),
            weather_lat: 0.0,
            weather_lon: 0.0,
            weather_position: "top-right".into(),
            weather_monitor: "primary".into(),
            weather_scale: 100,
            live: "off".into(),
            live_effect: "snow".into(),
            live_density: 2,
            live_battery: true,
            video_source: "both".into(),
            video_folder: String::new(),
            video: String::new(),
            video_rotate: true,
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "да" => Some(true),
        "0" | "false" | "no" | "off" | "нет" => Some(false),
        _ => None,
    }
}

fn b(v: bool) -> &'static str {
    if v { "1" } else { "0" }
}

/// Время суток для расписания тем.
pub const SLOTS: [&str; 4] = ["Утро", "День", "Вечер", "Ночь"];

impl Config {
    /// Часы начала утра, дня, вечера и ночи. Если в файле что-то не так — 6, 12, 18, 23.
    pub fn slot_hours(&self) -> [u32; 4] {
        let v: Vec<u32> = self.schedule_hours.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if v.len() == 4 {
            // ночь с полуночи можно записать как 0 или 24
            let h = [v[0], v[1], v[2], if v[3] == 0 { 24 } else { v[3] }];
            if h[0] < h[1] && h[1] < h[2] && h[2] < h[3] && h[3] <= 24 {
                return h;
            }
        }
        [6, 12, 18, 23]
    }

    /// Какое сейчас время суток (0 утро, 1 день, 2 вечер, 3 ночь) для часа 0..23.
    pub fn slot_at(&self, hour: u32) -> usize {
        let h = self.slot_hours();
        if hour >= h[3] || hour < h[0] {
            3
        } else if hour >= h[2] {
            2
        } else if hour >= h[1] {
            1
        } else {
            0
        }
    }

    /// Категории, которые действуют в этот час (с учётом расписания).
    pub fn topics_at(&self, hour: u32) -> &str {
        if self.schedule {
            let t = &self.topics_slot[self.slot_at(hour)];
            if !t.is_empty() {
                return t;
            }
        }
        &self.topics
    }

    /// Настройки из файла; если файла нет или он не читается — по умолчанию.
    pub fn load(path: &Path) -> Config {
        Self::try_load(path).unwrap_or_default()
    }

    /// None — файл не прочитался или пустой (его, возможно, как раз переписывают).
    pub fn try_load(path: &Path) -> Option<Config> {
        let bytes = std::fs::read(path).ok()?;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return None;
        }
        // Блокнот при «Юникоде» сохраняет в UTF-16 — понимаем и его
        let text = if bytes.starts_with(&[0xFF, 0xFE]) {
            let w: Vec<u16> = bytes[2..].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
            String::from_utf16_lossy(&w)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let mut c = Config::default();
        for raw in text.lines() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let k = k.trim().to_lowercase();
            // комментарий в конце строки после « ;» (кроме путей и текстов — там «;» может быть частью значения)
            let free_text = matches!(k.as_str(), "folder" | "query" | "weather_city" | "font" | "font2" | "video_folder" | "video");
            let v = match v.find(" ;") {
                Some(i) if !free_text => &v[..i],
                _ => v,
            }
            .trim()
            .to_string();
            match k.as_str() {
                "source" => c.source = v.to_lowercase(),
                "interval_min" => c.interval_min = v.parse().unwrap_or(c.interval_min),
                "paused" => c.paused = parse_bool(&v).unwrap_or(c.paused),
                // старый ключ из прошлых версий
                "per_monitor" => {
                    if parse_bool(&v) == Some(true) {
                        c.wall_mode = "per_monitor".into();
                    }
                }
                "wall_mode" => c.wall_mode = v.to_lowercase(),
                "folder" => c.folder = v,
                "topics" => c.topics = v.to_lowercase().replace(' ', ""),
                "schedule" => c.schedule = parse_bool(&v).unwrap_or(c.schedule),
                "schedule_hours" => c.schedule_hours = v.replace(' ', ""),
                "topics_morning" => c.topics_slot[0] = v.to_lowercase().replace(' ', ""),
                "topics_day" => c.topics_slot[1] = v.to_lowercase().replace(' ', ""),
                "topics_evening" => c.topics_slot[2] = v.to_lowercase().replace(' ', ""),
                "topics_night" => c.topics_slot[3] = v.to_lowercase().replace(' ', ""),
                "query" => c.query = v,
                "sorting" => c.sorting = v.to_lowercase(),
                "top_range" => c.top_range = v,
                "categories" => c.categories = v,
                "atleast" => c.atleast = v,
                "ratios" => c.ratios = v,
                "fit" => c.fit = v.to_lowercase(),
                "cache_keep" => c.cache_keep = v.parse().unwrap_or(c.cache_keep),
                "favorites_mix" => c.favorites_mix = v.parse::<u32>().unwrap_or(c.favorites_mix).min(100),
                "anim" => c.anim = v.to_lowercase(),
                "anim_ms" => c.anim_ms = v.parse::<u32>().unwrap_or(c.anim_ms).clamp(150, 5000),
                "clock" => c.clock = parse_bool(&v).unwrap_or(c.clock),
                "clock_monitor" => c.clock_monitor = v.to_lowercase(),
                "style" => c.style = v.to_lowercase(),
                "font" => c.font = v,
                "font2" => c.font2 = v,
                "color" => {
                    if v.eq_ignore_ascii_case("auto") {
                        c.color = COLOR_AUTO;
                    } else if let Ok(x) = u32::from_str_radix(v.trim_start_matches('#'), 16) {
                        c.color = x & 0xFFFFFF;
                    }
                }
                "opacity" => c.opacity = v.parse::<u32>().unwrap_or(c.opacity).clamp(10, 100),
                "scale" => c.scale = v.parse::<u32>().unwrap_or(c.scale).clamp(30, 300),
                "position" => c.position = v.to_lowercase(),
                "offset_x" => c.offset_x = v.parse::<i32>().unwrap_or(c.offset_x).clamp(-100, 100),
                "offset_y" => c.offset_y = v.parse::<i32>().unwrap_or(c.offset_y).clamp(-100, 100),
                "seconds" => c.seconds = parse_bool(&v).unwrap_or(c.seconds),
                "lang" => c.lang = v.to_lowercase(),
                "time_format" => c.time_format = v.to_lowercase(),
                "shadow" => c.shadow = parse_bool(&v).unwrap_or(c.shadow),
                "backdrop" => c.backdrop = v.to_lowercase(),
                "backdrop_opacity" => c.backdrop_opacity = v.parse::<u32>().unwrap_or(c.backdrop_opacity).clamp(10, 100),
                "ui_theme" => c.ui_theme = v.to_lowercase(),
                "update_check" => c.update_check = parse_bool(&v).unwrap_or(c.update_check),
                "ui_lang" => {
                    let v = v.to_lowercase();
                    c.ui_lang = if matches!(v.as_str(), "auto" | "ru" | "en") { v } else { "auto".into() };
                }
                "weather" => c.weather = parse_bool(&v).unwrap_or(c.weather),
                "weather_city" => c.weather_city = v,
                "weather_lat" => c.weather_lat = v.parse::<f64>().ok().filter(|x| x.is_finite() && (-90.0..=90.0).contains(x)).unwrap_or(c.weather_lat),
                "weather_lon" => c.weather_lon = v.parse::<f64>().ok().filter(|x| x.is_finite() && (-180.0..=180.0).contains(x)).unwrap_or(c.weather_lon),
                "weather_position" => c.weather_position = v.to_lowercase(),
                "weather_monitor" => c.weather_monitor = v.to_lowercase(),
                "weather_scale" => c.weather_scale = v.parse::<u32>().unwrap_or(c.weather_scale).clamp(30, 300),
                "live" => {
                    let v = v.to_lowercase();
                    c.live = if matches!(v.as_str(), "off" | "effects" | "video") { v } else { "off".into() };
                }
                "live_effect" => c.live_effect = v.to_lowercase(),
                "live_density" => c.live_density = v.parse::<u32>().unwrap_or(c.live_density).clamp(1, 3),
                "live_battery" => c.live_battery = parse_bool(&v).unwrap_or(c.live_battery),
                "video_source" => c.video_source = v.to_lowercase(),
                "video_folder" => c.video_folder = v,
                "video" => c.video = v,
                "video_rotate" => c.video_rotate = parse_bool(&v).unwrap_or(c.video_rotate),
                _ => {}
            }
        }
        Some(c)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = format!(
"; Настройки WallClock. Можно править прямо здесь — программа подхватит изменения сама.

[wallpaper]
; откуда брать обои: wallhaven (интернет) | folder (своя папка) | both (то и другое)
source = {source}
; как часто менять, в минутах (0 = только вручную)
interval_min = {interval}
; 1 = автосмена на паузе
paused = {paused}
; обои на нескольких мониторах: same (одинаковые) | per_monitor (разные) | span (единые — одна картинка на все экраны)
wall_mode = {wall_mode}
; своя папка с картинками (пусто = папка wallpapers рядом с программой)
folder = {folder}
; категории (можно несколько через запятую, каждый раз берётся случайная из них):
;   nature mountains sea forest space city architecture abstract minimalism dark sunset
;   winter animals cars fantasy digital games anime      (пусто = свой запрос ниже или просто популярное)
topics = {topics}
; расписание: 1 = на каждое время суток свои категории (утро, день, вечер, ночь)
schedule = {schedule}
; когда начинаются утро, день, вечер и ночь (часы по порядку; 0 у ночи = полночь)
schedule_hours = {schedule_hours}
; категории для каждого времени суток (пусто = как основные topics выше)
topics_morning = {topics_morning}
topics_day = {topics_day}
topics_evening = {topics_evening}
topics_night = {topics_night}
; свой запрос для Wallhaven (если категории не выбраны), например: nature mountains   или   abstract dark
query = {query}
; toplist = популярные за период | random = случайные
sorting = {sorting}
; период для toplist: 1d 3d 1w 1M 3M 6M 1y
top_range = {top_range}
; категории Wallhaven (general/anime/people): 100 = только общие
categories = {categories}
; минимальное разрешение (auto = как у монитора), например 2560x1440
atleast = {atleast}
; пропорции (auto = как у монитора), например 16x9 или 21x9
ratios = {ratios}
; как растягивать: fill | fit | stretch | center | tile | span
fit = {fit}
; сколько скачанных обоев хранить в кэше
cache_keep = {cache_keep}
; как часто показывать избранное (папка favorites) среди новых обоев, % (0 — никогда)
favorites_mix = {favorites_mix}
; анимация смены обоев: none | fade (растворение) | slide (сдвиг) | wipe (шторка) | circle (круг) | blinds (жалюзи) | random
anim = {anim}
; длительность анимации, мс
anim_ms = {anim_ms}

[clock]
; 1 = показывать часы на рабочем столе
clock = {clock}
; на каком мониторе часы: primary (основной) | second (второй) | all (на всех)
clock_monitor = {clock_monitor}
; стиль: hero (как на фото) | photo (точно как на фото — свой шрифт с разрывами) | minimal | split (время | дата) | vertical | words (словами)
;        calendar (большое число) | outline (контур) | poster | line (одной строкой) | techno | analog (стрелочные)
;        pixel (пиксельное табло) | block (блочный пиксель)
style = {style}
; шрифт крупного текста: auto | встроенные: Michroma, Unbounded, Jura, Russo One, Oswald, Comfortaa, Tektur,
;   Exo 2, Montserrat Alternates Light, Cormorant Garamond, Press Start 2P | или любой установленный (Bahnschrift, Georgia...)
; Michroma — только латиница, для русского берите любой другой
font = {font}
; шрифт мелкого текста (дата): auto | любое имя шрифта
font2 = {font2}
; цвет текста в HEX (например FFFFFF) или auto — белый/тёмный по яркости обоев
color = {color}
; непрозрачность, %
opacity = {opacity}
; размер, % (100 = обычный)
scale = {scale}
; положение: center | top | bottom | left | right | top-left | top-right | bottom-left | bottom-right
position = {position}
; сдвиг в процентах экрана (минус = влево/вверх)
offset_x = {offset_x}
offset_y = {offset_y}
; 1 = показывать секунды (перерисовка раз в секунду)
seconds = {seconds}
; язык: ru | en
lang = {lang}
; формат времени: auto | 24 | 12
time_format = {time_format}
; 1 = мягкая тень под текстом (читается на светлых обоях)
shadow = {shadow}
; подложка «матовое стекло» под часами и погодой: off (нет) | on (всегда) | auto (только на пёстром фоне)
backdrop = {backdrop}
; непрозрачность подложки, % (100 — сплошная, 10 — почти невидимая)
backdrop_opacity = {backdrop_opacity}

[window]
; тема окна настроек: auto (как в Windows) | light (светлая) | dark (тёмная)
ui_theme = {ui_theme}
; язык окна и меню: auto (как в Windows) | ru | en
ui_lang = {ui_lang}
; 1 = раз в день проверять на GitHub, нет ли новой версии (обновление ставится только с верной подписью)
update_check = {update_check}

[weather]
; 1 = показывать виджет погоды на рабочем столе (данные: open-meteo.com, без ключа)
weather = {weather}
; город и его координаты — проще всего выбрать в окне настроек, раздел «Погода»
weather_city = {weather_city}
weather_lat = {weather_lat}
weather_lon = {weather_lon}
; положение виджета: center | top | bottom | left | right | top-left | top-right | bottom-left | bottom-right
weather_position = {weather_position}
; на каком мониторе: primary | second | all
weather_monitor = {weather_monitor}
; размер виджета, %
weather_scale = {weather_scale}

[live]
; живые обои: off (выключены) | effects (эффекты поверх обоев) | video (видео вместо картинки)
live = {live}
; эффект: snow (снег) | rain (дождь) | stars (звёзды) | leaves (листопад) | fireflies (светлячки) | weather (по погоде)
live_effect = {live_effect}
; сколько частиц: 1 мало | 2 средне | 3 много
live_density = {live_density}
; 1 = замирать при работе от батареи
live_battery = {live_battery}
; откуда брать видео: both (всё) | folder (своя папка) | we (обои Wallpaper Engine типа «видео»)
video_source = {video_source}
; своя папка с видео (пусто = папка videos рядом с программой)
video_folder = {video_folder}
; текущее видео
video = {video}
; 1 = менять видео с тем же интервалом, что и обои
video_rotate = {video_rotate}
",
            source = self.source,
            interval = self.interval_min,
            paused = b(self.paused),
            wall_mode = self.wall_mode,
            folder = Self::clean(&self.folder),
            topics = Self::clean(&self.topics),
            schedule = b(self.schedule),
            schedule_hours = Self::clean(&self.schedule_hours),
            topics_morning = Self::clean(&self.topics_slot[0]),
            topics_day = Self::clean(&self.topics_slot[1]),
            topics_evening = Self::clean(&self.topics_slot[2]),
            topics_night = Self::clean(&self.topics_slot[3]),
            query = Self::clean(&self.query),
            sorting = self.sorting,
            top_range = self.top_range,
            categories = self.categories,
            atleast = self.atleast,
            ratios = self.ratios,
            fit = self.fit,
            cache_keep = self.cache_keep,
            favorites_mix = self.favorites_mix,
            anim = self.anim,
            anim_ms = self.anim_ms,
            clock = b(self.clock),
            clock_monitor = self.clock_monitor,
            style = self.style,
            font = Self::clean(&self.font),
            font2 = Self::clean(&self.font2),
            color = if self.color == COLOR_AUTO { "auto".to_string() } else { format!("{:06X}", self.color) },
            opacity = self.opacity,
            scale = self.scale,
            position = self.position,
            offset_x = self.offset_x,
            offset_y = self.offset_y,
            seconds = b(self.seconds),
            lang = self.lang,
            time_format = self.time_format,
            shadow = b(self.shadow),
            backdrop = self.backdrop,
            backdrop_opacity = self.backdrop_opacity,
            ui_theme = self.ui_theme,
            ui_lang = Self::clean(&self.ui_lang),
            update_check = b(self.update_check),
            weather = b(self.weather),
            weather_city = Self::clean(&self.weather_city),
            weather_lat = self.weather_lat,
            weather_lon = self.weather_lon,
            weather_position = self.weather_position,
            weather_monitor = self.weather_monitor,
            weather_scale = self.weather_scale,
            live = Self::clean(&self.live),
            live_effect = Self::clean(&self.live_effect),
            live_density = self.live_density,
            live_battery = b(self.live_battery),
            video_source = Self::clean(&self.video_source),
            video_folder = Self::clean(&self.video_folder),
            video = Self::clean(&self.video),
            video_rotate = b(self.video_rotate),
        );
        let tmp = path.with_extension("ini.tmp");
        std::fs::write(&tmp, text.replace('\n', "\r\n"))?;
        std::fs::rename(&tmp, path)
    }

    /// Строки без управляющих символов — чтобы значение не «перетекло» на следующую строку файла.
    fn clean(s: &str) -> String {
        s.chars().filter(|c| !c.is_control()).collect()
    }

    pub fn is_24h(&self) -> bool {
        match self.time_format.as_str() {
            "12" => false,
            "24" => true,
            _ => self.lang != "en",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Обещание совместимости (к версии 1.0): любой config.ini, записанный прошлыми версиями,
    // читается и дальше — новые ключи получают значения по умолчанию, лишние пропускаются.
    #[test]
    fn config_compat() {
        let dir = std::env::temp_dir().join(format!("hora-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.ini");

        // 1) всё, что пишет save(), читается обратно без потерь
        let mut c = Config::default();
        c.interval_min = 7;
        c.topics = "space,sea".into();
        c.font = "Hora Pixel".into();
        c.wall_mode = "span".into();
        c.ui_lang = "en".into();
        c.update_check = false;
        c.folder = r"D:\Картинки; отпуск".into();
        c.save(&p).unwrap();
        assert!(Config::try_load(&p).unwrap() == c, "save → load меняет настройки");

        // 2) файл самой первой версии (WallClock): старый ключ per_monitor, CRLF, комментарии,
        //    неизвестный ключ и ни одного из новых ключей
        let old = "; Настройки WallClock
[wallpaper]
source = folder
interval_min = 30
per_monitor = 1
something_removed = 5
[clock]
style = photo ; как на фото
";
        std::fs::write(&p, old).unwrap();
        let o = Config::try_load(&p).unwrap();
        let d = Config::default();
        assert_eq!((o.source.as_str(), o.interval_min, o.wall_mode.as_str(), o.style.as_str()), ("folder", 30, "per_monitor", "photo"));
        assert!(o.font == d.font && o.update_check == d.update_check && o.ui_lang == d.ui_lang && o.schedule == d.schedule);

        // 3) сохранён Блокнотом в «Юникоде» (UTF-16 с меткой FF FE)
        let mut bytes = vec![0xFF, 0xFE];
        for u in "interval_min = 45
".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&p, bytes).unwrap();
        assert_eq!(Config::try_load(&p).unwrap().interval_min, 45);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // время суток по часу: границы, ночь через полночь, «0» у ночи, ошибки в файле
    #[test]
    fn slots() {
        let mut c = Config::default();
        let got: Vec<usize> = (0..24).map(|h| c.slot_at(h)).collect();
        assert_eq!(got, [3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3]);
        c.schedule_hours = "7,11,17,0".into();
        assert_eq!((c.slot_at(23), c.slot_at(0), c.slot_at(6), c.slot_at(7), c.slot_at(17)), (2, 3, 3, 0, 2));
        c.schedule_hours = "12,6,18,23".into(); // не по порядку — берём обычные часы
        assert_eq!(c.slot_hours(), [6, 12, 18, 23]);
        c.schedule_hours = "a,b".into();
        assert_eq!(c.slot_hours(), [6, 12, 18, 23]);
        // расписание выключено — основные темы; у дня пусто — тоже основные
        c.schedule_hours = "6,12,18,23".into();
        c.topics = "cars".into();
        assert_eq!(c.topics_at(20), "cars");
        c.schedule = true;
        assert_eq!(c.topics_at(20), "sunset,sea,city");
        assert_eq!(c.topics_at(14), "cars");
        assert_eq!(c.topics_at(2), "space,dark,minimalism");
    }
}
