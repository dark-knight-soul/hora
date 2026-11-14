// Перевод интерфейса (окно настроек, меню трея, установщик): русский текст в коде — ключ,
// английский берётся из таблицы ниже. Язык: ui_lang = auto (как в Windows) | ru | en.
// Надписи с подстановкой значений переведены прямо в коде через en().

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// 0 — ещё не выбран (берём язык Windows), 1 — русский, 2 — английский.
static LANG: AtomicU8 = AtomicU8::new(0);

/// Windows на русском или на языке, где русский обычно понятнее английского.
pub fn system_ru() -> bool {
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3FF;
    // ru, uk, be, kk, uz, ky, tg, hy, az, ka
    matches!(id, 0x19 | 0x22 | 0x23 | 0x3F | 0x43 | 0x40 | 0x28 | 0x2B | 0x2C | 0x37)
}

/// Выбрать язык интерфейса по настройке (auto | ru | en).
pub fn set_lang(setting: &str) {
    let v = match setting {
        "ru" => 1,
        "en" => 2,
        _ => {
            // HORA_UI_LANG=ru|en — как будто Windows на этом языке (проверка установщика и меню
            // на «чужой» Windows, где настроек ещё нет)
            let forced = std::env::var("HORA_UI_LANG").ok();
            if let Some(v @ ("ru" | "en")) = forced.as_deref() {
                LANG.store(if v == "en" { 2 } else { 1 }, Ordering::Relaxed);
                return;
            }
            if system_ru() {
                1
            } else {
                2
            }
        }
    };
    LANG.store(v, Ordering::Relaxed);
}

/// Интерфейс на английском?
pub fn en() -> bool {
    let v = LANG.load(Ordering::Relaxed);
    if v == 0 {
        set_lang("auto");
        return LANG.load(Ordering::Relaxed) == 2;
    }
    v == 2
}

/// Надпись на языке интерфейса (русская строка — ключ; если перевода нет — как есть).
pub fn tr(s: &str) -> &str {
    if !en() {
        return s;
    }
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| PAIRS.iter().copied().collect()).get(s).copied().unwrap_or(s)
}

const PAIRS: &[(&str, &str)] = &[
    ("Проверить обновления", "Check for updates"),
    ("Что нового", "What's new"),
    ("Проверять обновления автоматически", "Check for updates automatically"),
    ("Раз в день спрашивать GitHub, нет ли новой версии", "Once a day, ask GitHub whether a new version is out"),
    ("Обновления ещё не проверялись", "Updates haven't been checked yet"),
    ("Проверяю обновления…", "Checking for updates…"),
    ("У вас последняя версия", "You have the latest version"),
    ("Доступна новая версия Hora", "A new version of Hora is available"),
    ("не удалось прочитать ответ GitHub", "could not read GitHub's reply"),
    ("в новом выпуске нет подписанного установщика", "the new release has no signed installer"),
    ("подпись обновления не сходится — установка отменена", "the update's signature does not match — installation canceled"),
    ("Выключены", "Off"),
    ("Эффекты поверх обоев", "Effects over wallpaper"),
    ("Видео", "Video"),
    ("Снег", "Snow"),
    ("Дождь", "Rain"),
    ("Звёзды", "Stars"),
    ("Листопад", "Falling leaves"),
    ("Светлячки", "Fireflies"),
    ("По погоде", "Match the weather"),
    ("Мало", "Few"),
    ("Средне", "Medium"),
    ("Много", "Many"),
    ("Всё", "All"),
    ("Моя папка", "My folder"),
    ("Нет", "No"),
    ("Иногда", "Sometimes"),
    ("Часто", "Often"),
    ("Одинаковые", "Same"),
    ("Разные на каждом", "Different on each"),
    ("Единые (одна картинка на все экраны)", "Spanned (one picture across all screens)"),
    ("Всегда", "Always"),
    ("Авто (только на пёстром фоне)", "Auto (busy backgrounds only)"),
    ("Как в Windows", "Same as Windows"),
    ("Светлая", "Light"),
    ("Тёмная", "Dark"),
    ("Без анимации", "No animation"),
    ("Растворение", "Fade"),
    ("Сдвиг", "Slide"),
    ("Шторка", "Wipe"),
    ("Круг из центра", "Circle from the center"),
    ("Жалюзи", "Blinds"),
    ("Случайная", "Random"),
    ("Быстро", "Fast"),
    ("Обычно", "Normal"),
    ("Медленно", "Slow"),
    ("Основной", "Primary"),
    ("Второй", "Second"),
    ("На всех", "All"),
    ("Только вручную", "Manual only"),
    ("5 минут", "5 minutes"),
    ("15 минут", "15 minutes"),
    ("30 минут", "30 minutes"),
    ("1 час", "1 hour"),
    ("3 часа", "3 hours"),
    ("12 часов", "12 hours"),
    ("1 день", "1 day"),
    ("Wallhaven (интернет)", "Wallhaven (online)"),
    ("Wallhaven + моя папка", "Wallhaven + my folder"),
    ("Только избранное", "Favorites only"),
    ("День недели", "Weekday"),
    ("Буквы с разрывами", "Split letters"),
    ("Крупное время", "Big time"),
    ("Пиксельное табло", "Pixel scoreboard"),
    ("Блочный пиксель", "Block pixels"),
    ("Время | дата", "Time | date"),
    ("Вертикально", "Vertical"),
    ("Словами", "In words"),
    ("Календарь", "Calendar"),
    ("Контур", "Outline"),
    ("Постер", "Poster"),
    ("Одной строкой", "Single line"),
    ("Техно", "Techno"),
    ("Стрелки", "Analog"),
    ("Русский", "Русский"),
    ("По центру", "Center"),
    ("Сверху", "Top"),
    ("Снизу", "Bottom"),
    ("Слева", "Left"),
    ("Справа", "Right"),
    ("Слева сверху", "Top left"),
    ("Справа сверху", "Top right"),
    ("Слева снизу", "Bottom left"),
    ("Справа снизу", "Bottom right"),
    ("Авто (по яркости обоев)", "Auto (by wallpaper brightness)"),
    ("Белый", "White"),
    ("Тёплый белый", "Warm white"),
    ("Светло-серый", "Light gray"),
    ("Графит", "Graphite"),
    ("Чёрный", "Black"),
    ("Голубой", "Sky blue"),
    ("Синий", "Blue"),
    ("Мятный", "Mint"),
    ("Лавандовый", "Lavender"),
    ("Розовый", "Pink"),
    ("Золотой", "Gold"),
    ("Оранжевый", "Orange"),
    ("Красный", "Red"),
    ("Авто (под стиль)", "Auto (matches the style)"),
    ("Michroma — как на фото (только латиница)", "Michroma — wide techno (Latin only)"),
    ("WallClock Split — свой, с разрывами (рус/англ)", "WallClock Split — custom split letters (Latin/Cyrillic)"),
    ("Megrim — буквы с разрывами (только латиница)", "Megrim — split letters (Latin only)"),
    ("Unbounded — широкий", "Unbounded — wide"),
    ("Jura — тонкий техно", "Jura — thin techno"),
    ("Russo One — жирный спортивный", "Russo One — bold sporty"),
    ("Oswald — узкий", "Oswald — narrow"),
    ("Comfortaa — округлый", "Comfortaa — rounded"),
    ("Tektur — угловатый", "Tektur — angular"),
    ("Exo 2 — футуристичный", "Exo 2 — futuristic"),
    ("Montserrat Alternates — лёгкий", "Montserrat Alternates — light"),
    ("Cormorant — с засечками, изящный", "Cormorant — elegant serif"),
    ("Press Start 2P — пиксельный", "Press Start 2P — pixel"),
    ("Hora Pixel — точки, как на табло (рус/англ)", "Hora Pixel — LED dots, like a scoreboard (Latin/Cyrillic)"),
    ("Hora Block — блочные пиксели, как в играх-песочницах (рус/англ)", "Hora Block — chunky block pixels, sandbox-game style (Latin/Cyrillic)"),
    ("Авто (Bahnschrift)", "Auto (Bahnschrift)"),
    ("нет видео: положите mp4 в папку videos или подпишитесь на видеообои в Wallpaper Engine", "no videos: put mp4 files into the videos folder or subscribe to video wallpapers in Wallpaper Engine"),
    ("Hora — подбираю обои…", "Hora — picking a wallpaper…"),
    ("неизвестная ошибка", "unknown error"),
    ("Открыть Hora", "Open Hora"),
    ("Следующие обои", "Next wallpaper"),
    ("В избранном (нажмите, чтобы убрать)", "In favorites (click to remove)"),
    ("В избранное", "Add to favorites"),
    ("Больше не показывать эти обои", "Never show this wallpaper again"),
    ("Вернуть последние скрытые", "Restore last hidden"),
    ("Пауза автосмены", "Pause auto-change"),
    ("Менять каждые", "Change every"),
    ("Откуда брать обои", "Wallpaper source"),
    ("Открыть папку избранного", "Open favorites folder"),
    ("Открыть список скрытых", "Open hidden list"),
    ("Показывать избранное", "Show favorites"),
    ("Следующее видео", "Next video"),
    ("Открыть папку с видео", "Open video folder"),
    ("Живые обои", "Live wallpaper"),
    ("Любые (популярное)", "Any (popular)"),
    ("По времени суток (расписание)", "By time of day (schedule)"),
    ("Категории обоев", "Wallpaper categories"),
    ("Обои на мониторах", "Wallpaper on monitors"),
    ("Анимация смены", "Transition animation"),
    ("Сохранить текущие в мою папку", "Save current to my folder"),
    ("Показывать часы", "Show clock"),
    ("На каком мониторе", "On which monitor"),
    ("Секунды", "Seconds"),
    ("24-часовой формат", "24-hour format"),
    ("Тень под текстом", "Shadow under text"),
    ("Подложка (матовое стекло)", "Backdrop (frosted glass)"),
    ("Положение", "Position"),
    ("Размер", "Size"),
    ("Цвет", "Color"),
    ("Непрозрачность", "Opacity"),
    ("Шрифт времени", "Time font"),
    ("Шрифт подписей", "Caption font"),
    ("Часы", "Clock"),
    ("Показывать погоду", "Show weather"),
    ("Обновить сейчас", "Refresh now"),
    ("Выбрать город…", "Choose a city…"),
    ("Погода", "Weather"),
    ("Все настройки (config.ini)", "All settings (config.ini)"),
    ("Открыть мою папку с обоями", "Open my wallpaper folder"),
    ("Запускать вместе с Windows", "Start with Windows"),
    ("Выход", "Exit"),
    ("Язык интерфейса", "Interface language"),
    ("Обои", "Wallpaper"),
    ("Категории", "Categories"),
    ("Оформление", "Appearance"),
    ("Мониторы", "Monitors"),
    ("Прочее", "Other"),
    ("Что ставить на рабочий стол и как часто", "What to put on the desktop and how often"),
    ("Темы картинок — можно отметить несколько", "Picture topics — pick as many as you like"),
    ("Стиль и поведение часов на рабочем столе", "Style and behavior of the desktop clock"),
    ("Цвет, размер, положение и шрифты", "Color, size, position and fonts"),
    ("Виджет погоды на рабочем столе", "Weather widget on the desktop"),
    ("Где показывать часы и как раскладывать обои", "Where to show the clock and how to lay out wallpaper"),
    ("Снег, дождь, звёзды поверх обоев или видео вместо картинки", "Snow, rain, stars over your wallpaper — or video instead of a picture"),
    ("Автозапуск, тема окна и файлы программы", "Autostart, window theme and program files"),
    ("Ищу…", "Searching…"),
    ("Не удалось начать поиск", "Could not start the search"),
    ("Ничего не нашлось — попробуйте написать иначе", "Nothing found — try spelling it differently"),
    ("Выберите город:", "Choose a city:"),
    ("Сейчас на рабочем столе", "On the desktop now"),
    ("Автосмена на паузе", "Auto-change is paused"),
    ("Автосмена выключена — только вручную", "Auto-change is off — manual only"),
    ("ошибка", "error"),
    ("В избранном", "In favorites"),
    ("Не показывать", "Never show"),
    ("Обои не будут меняться сами, пока пауза включена", "Wallpaper won't change by itself while paused"),
    ("Избранное и скрытые", "Favorites and hidden"),
    ("Показывать избранное среди новых обоев", "Show favorites among new wallpapers"),
    ("Открыть избранное", "Open favorites"),
    ("Список скрытых", "Hidden list"),
    ("Эффекты делает сама Windows — процессор почти не тратится", "Windows does the effects itself — almost no CPU used"),
    ("Скорость", "Speed"),
    ("Темы", "Topics"),
    ("Каждый раз берётся случайная из отмеченных. Картинки — wallhaven.cc", "A random one of the selected is used each time. Pictures — wallhaven.cc"),
    ("Сейчас обои берутся из вашей папки — выбор темы переключит источник на Wallhaven.", "Wallpapers now come from your folder — picking a topic switches the source to Wallhaven."),
    ("Менять темы по времени суток", "Change topics by time of day"),
    ("Утром природа, вечером закаты, ночью космос — обои меняются сами", "Nature in the morning, sunsets in the evening, space at night — changes by itself"),
    ("Как основные", "Same as main"),
    (" — сейчас", " — now"),
    ("Берутся основные темы, отмеченные выше", "The main topics selected above are used"),
    ("Часы начала утра, дня, вечера и ночи меняются в config.ini (schedule_hours).", "Start hours of morning, day, evening and night are set in config.ini (schedule_hours)."),
    ("Большие часы на рабочем столе, под иконками", "A big clock on the desktop, under the icons"),
    ("Стиль", "Style"),
    ("Часы обновляются раз в секунду вместо раза в минуту", "The clock updates every second instead of every minute"),
    ("Помогает читать часы на светлых обоях", "Helps reading the clock on light wallpapers"),
    ("Язык", "Clock language"),
    ("свой цвет из config.ini", "custom color from config.ini"),
    ("Подложка под часами", "Backdrop behind the clock"),
    ("Матовое стекло: размытые обои под текстом — читается на любом фоне", "Frosted glass: blurred wallpaper behind the text — readable on any background"),
    ("Прозрачность подложки", "Backdrop transparency"),
    ("Положение на экране", "Position on screen"),
    ("Michroma — только латиница; для русского выбирайте любой другой", "Michroma has Latin letters only; pick another font for Cyrillic"),
    ("Дата и день недели", "Date and weekday"),
    ("Данные open-meteo.com, обновление раз в 30 минут", "Data from open-meteo.com, updated every 30 minutes"),
    ("Город не выбран — найдите его ниже", "No city selected — find it below"),
    ("Город", "City"),
    ("Найти", "Search"),
    ("Напишите город на русском или английском и нажмите Enter", "Type a city in English or Russian and press Enter"),
    ("Загружаю…", "Loading…"),
    ("Сначала выберите город", "Choose a city first"),
    ("Нет данных", "No data"),
    ("Обновить", "Refresh"),
    ("Можно поставить в другой угол, чем часы", "Can be placed in a different corner than the clock"),
    ("На мониторе", "On monitor"),
    ("Выключены — программа занимает минимум памяти", "Off — the program uses minimal memory"),
    ("Работают. Замирают, когда рабочий стол закрыт окнами или экран заблокирован", "Running. They pause when the desktop is covered by windows or the screen is locked"),
    ("Запускаются…", "Starting…"),
    ("По погоде: в снегопад — снег, в дождь — дождь, ясной ночью — звёзды", "Match the weather: snow when it snows, rain when it rains, stars on clear nights"),
    ("Рисуется поверх обычных обоев и часов", "Drawn over the regular wallpaper and clock"),
    ("Эффект", "Effect"),
    ("Сколько", "Amount"),
    ("Откуда брать видео", "Video source"),
    ("Из Wallpaper Engine берутся только видеообои — сцены и веб-обои он показывает сам", "Only video wallpapers are taken from Wallpaper Engine — scenes and web wallpapers only it can show"),
    ("Пока ничего: положите mp4 или webm в папку с видео либо подпишитесь на видеообои в Wallpaper Engine", "Nothing yet: put mp4 or webm files into the video folder or subscribe to video wallpapers in Wallpaper Engine"),
    ("Папка с видео", "Video folder"),
    ("Менять видео вместе с обоями", "Change video along with wallpaper"),
    ("С тем же интервалом, что и обычные обои (раздел «Обои»)", "At the same interval as regular wallpaper (Wallpaper section)"),
    ("Замирать от батареи", "Pause on battery"),
    ("На ноутбуке без зарядки живые обои не тратят заряд", "On a laptop without a charger, live wallpaper won't drain the battery"),
    (" (основной)", " (primary)"),
    ("Часы на мониторе", "Clock on monitor"),
    ("Единые — одна широкая картинка продолжается с экрана на экран", "Spanned — one wide picture continues from screen to screen"),
    ("Подключён один монитор — дополнительные настройки появятся, когда подключите второй.", "One monitor is connected — more settings will appear when you connect a second one."),
    ("Hora будет стартовать сама при входе в систему", "Hora will start by itself when you sign in"),
    ("Тема окна", "Window theme"),
    ("«Как в Windows» — следует за светлым или тёмным режимом системы", "\"Same as Windows\" follows the system light or dark mode"),
    ("Файлы", "Files"),
    ("Все настройки", "All settings"),
    ("Журнал", "Log"),
    ("Сообщить о проблеме", "Report a problem"),
    ("Свечение точек", "Glowing dots"),
    ("Мягкий ореол вокруг точек — как у настоящего табло", "A soft glow around the dots, like a real LED board"),
    ("Мигающее двоеточие", "Blinking colon"),
    ("Часы перерисовываются раз в секунду", "The clock is redrawn once a second"),
    ("Горячие клавиши", "Hotkeys"),
    (" (занято)", " (taken)"),
    ("Hora закрылась из-за ошибки", "Hora closed because of an error"),
    ("Программа перезапущена. Нажмите, чтобы сообщить об ошибке на GitHub — это поможет её исправить.", "Hora has been restarted. Click to report the error on GitHub — it helps to fix it."),
    ("Папка программы", "Program folder"),
    ("Обои: wallhaven.cc · Погода: open-meteo.com · Шрифты: Google Fonts (OFL)", "Wallpapers: wallhaven.cc · Weather: open-meteo.com · Fonts: Google Fonts (OFL)"),
    ("Выйти из Hora", "Quit Hora"),
    ("Загрузка…", "Loading…"),
    ("Язык окна и меню (часы — в разделе «Часы»)", "Language of the window and menu (the clock's language is in Clock)"),
    ("Установка Hora", "Hora Setup"),
    ("Для кого установить Hora?", "Who should Hora be installed for?"),
    ("Установка Hora", "Hora Setup"),
    ("Hora меняет обои, показывает часы и погоду на рабочем столе. Настройки у каждого пользователя свои.", "Hora changes your wallpaper and shows a clock and weather on the desktop. Each user has their own settings."),
    ("Не удалось установить Hora", "Could not install Hora"),
    ("не нашлась папка для программ", "the programs folder was not found"),
    ("Hora установлена", "Hora is installed"),
    ("Ярлык — в меню «Пуск». Удалить программу можно в «Параметры → Приложения».", "A shortcut is in the Start menu. You can uninstall it in Settings → Apps."),
    ("Запустить Hora", "Launch Hora"),
    ("Обои, часы и погода на рабочем столе", "Wallpaper, clock and weather on your desktop"),
    ("Это переносная версия Hora", "This is the portable version of Hora"),
    ("Её не нужно удалять через «Приложения» — просто удалите папку с программой.", "It doesn't need to be uninstalled via Apps — just delete the program folder."),
    ("Удалить Hora?", "Uninstall Hora?"),
    ("Будут удалены программа, ярлык в меню «Пуск» и автозапуск.", "The program, its Start menu shortcut and autostart will be removed."),
    ("Удалить также мои настройки, избранное и скачанные обои", "Also delete my settings, favorites and downloaded wallpapers"),
    ("Hora не удалена", "Hora was not uninstalled"),
    ("Без прав администратора программу «для всех пользователей» удалить нельзя.", "An installation \"for all users\" can't be removed without administrator rights."),
    ("Hora удалена", "Hora is uninstalled"),
    ("Спасибо, что пользовались!", "Thanks for using it!"),
    ("Природа", "Nature"),
    ("Горы", "Mountains"),
    ("Море и пляжи", "Sea and beaches"),
    ("Лес", "Forest"),
    ("Закаты", "Sunsets"),
    ("Зима", "Winter"),
    ("Космос", "Space"),
    ("Архитектура", "Architecture"),
    ("Абстракция", "Abstract"),
    ("Минимализм", "Minimalism"),
    ("Тёмные", "Dark"),
    ("Животные", "Animals"),
    ("Машины", "Cars"),
    ("Фэнтези", "Fantasy"),
    ("Цифровое искусство", "Digital art"),
    ("Игры", "Games"),
    ("Аниме", "Anime"),
    ("Windows не принял картинку как обои", "Windows did not accept the picture as wallpaper"),
    ("в своей папке нет картинок", "no pictures in your folder"),
    ("кэш пуст", "the cache is empty"),
    ("в избранном нет других картинок", "no other pictures in favorites"),
    ("на Wallhaven не нашлось новых картинок (все уже были)", "no new pictures on Wallhaven (all were already shown)"),
    ("Wallhaven ничего не нашёл по этим настройкам", "Wallhaven found nothing for these settings"),
    ("скачивание идёт слишком долго", "the download takes too long"),
    ("обрыв при скачивании", "the download was interrupted"),
    ("файл слишком большой", "the file is too large"),
    ("скачался не JPEG/PNG — пропускаю", "the download is not a JPEG/PNG — skipping"),
    ("скачался пустой файл", "the downloaded file is empty"),
    ("Утро", "Morning"),
    ("День", "Day"),
    ("Вечер", "Evening"),
    ("Ночь", "Night"),
    ("Цветная рамка — мониторы, на которых сейчас показываются часы", "Highlighted frame — monitors that show the clock now"),
];

#[cfg(test)]
mod tests {
    use super::PAIRS;
    use std::collections::HashSet;

    /// Все строковые литералы со строки кода (без экранирования — у нас в подписях его нет).
    fn literals(line: &str) -> Vec<&str> {
        line.split('"').skip(1).step_by(2).collect()
    }

    fn cyrillic(s: &str) -> bool {
        s.chars().any(|c| ('а'..='я').contains(&c.to_ascii_lowercase()) || ('А'..='Я').contains(&c) || c == 'ё' || c == 'Ё')
    }

    // Гарантия для английской Windows: у каждой русской подписи интерфейса есть перевод.
    // Проверяем (1) всё, что передаётся в tr("…"), и (2) подписи в списках меню/настроек
    // вида ("значение", "Подпись") в main.rs и ui.rs.
    #[test]
    fn every_label_has_english() {
        let keys: HashSet<&str> = PAIRS.iter().map(|p| p.0).collect();
        let sources = [
            ("main.rs", include_str!("main.rs")),
            ("ui.rs", include_str!("ui.rs")),
            ("setup.rs", include_str!("setup.rs")),
            ("update.rs", include_str!("update.rs")),
            ("crash.rs", include_str!("crash.rs")),
            ("live.rs", include_str!("live.rs")),
            ("wallpaper.rs", include_str!("wallpaper.rs")),
            ("http.rs", include_str!("http.rs")),
            ("clock.rs", include_str!("clock.rs")),
        ];
        let mut missing = Vec::new();
        let (mut calls, mut labels) = (0, 0);
        for (file, text) in sources {
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                // (1) tr("…")
                for part in code.split("tr(\"").skip(1) {
                    if let Some(lit) = part.split('"').next() {
                        calls += cyrillic(lit) as usize;
                        if cyrillic(lit) && !keys.contains(lit) {
                            missing.push(format!("{file}:{} tr(\"{lit}\")", n + 1));
                        }
                    }
                }
                // (2) элементы списков подписей: строка начинается с «(» и заканчивается «),»
                let t = code.trim();
                // (3) текстовые константы интерфейса: const ИМЯ: &str = "…"
                if t.starts_with("const ") && t.contains(": &str = \"") {
                    for lit in literals(t) {
                        labels += cyrillic(lit) as usize;
                        if cyrillic(lit) && !keys.contains(lit) {
                            missing.push(format!("{file}:{} const «{lit}»", n + 1));
                        }
                    }
                }
                if (file == "main.rs" || file == "ui.rs") && t.starts_with('(') && t.ends_with("),") && !t.contains("tr(") {
                    for lit in literals(t) {
                        labels += cyrillic(lit) as usize;
                        if cyrillic(lit) && !keys.contains(lit) {
                            missing.push(format!("{file}:{} «{lit}»", n + 1));
                        }
                    }
                }
            }
        }
        // защита от «тест ничего не нашёл и потому прошёл»
        assert!(calls > 150 && labels > 60, "проверено слишком мало: tr — {calls}, подписей в списках — {labels}");
        assert!(missing.is_empty(), "нет английского перевода ({}):\n{}", missing.len(), missing.join("\n"));
    }
}
