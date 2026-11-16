// Смена обоев: выбор картинки (Wallhaven / своя папка / кэш), скачивание и установка.
// Всё это выполняется в отдельном коротком потоке, чтобы не тормозить часы и меню.
use crate::i18n::tr;
use crate::config::Config;
use crate::util::{is_image, wide_os, Rng};
use crate::http;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

/// Категории обоев: (id для config.ini, название в меню, запрос Wallhaven, категории general/anime/people)
/// Номера пунктов меню: M_TOPIC + индекс (до 40 штук) и M_SCHED + время суток·50 + индекс (до 49).
pub const TOPICS: [(&str, &str, &str, &str); 34] = [
    ("nature", "Природа", "nature", "100"),
    ("mountains", "Горы", "mountains", "100"),
    ("sea", "Море и пляжи", "sea", "100"),
    ("forest", "Лес", "forest", "100"),
    ("sunset", "Закаты", "sunset", "100"),
    ("winter", "Зима", "winter", "100"),
    ("space", "Космос", "space", "100"),
    ("city", "Город", "city", "100"),
    ("architecture", "Архитектура", "architecture", "100"),
    ("abstract", "Абстракция", "abstract", "100"),
    ("minimalism", "Минимализм", "minimalism", "100"),
    ("dark", "Тёмные", "dark", "100"),
    ("animals", "Животные", "animals", "100"),
    ("cars", "Машины", "car", "100"),
    ("fantasy", "Фэнтези", "fantasy art", "100"),
    ("digital", "Цифровое искусство", "digital art", "100"),
    ("games", "Игры", "video games", "100"),
    ("anime", "Аниме", "", "010"),
    ("ascii", "ASCII-арт", "ascii", "100"),
    ("pixel", "Пиксель-арт", "pixel art", "100"),
    ("cyberpunk", "Киберпанк", "cyberpunk", "100"),
    ("synthwave", "Ретро и синтвейв", "synthwave", "100"),
    ("lowpoly", "Low poly", "low poly", "100"),
    ("flowers", "Цветы", "flowers", "100"),
    ("lake", "Озёра и реки", "lake", "100"),
    ("waterfall", "Водопады", "waterfall", "100"),
    ("desert", "Пустыня", "desert", "100"),
    ("autumn", "Осень", "autumn", "100"),
    ("aurora", "Северное сияние", "aurora", "100"),
    ("clouds", "Небо и облака", "clouds", "100"),
    ("planets", "Планеты", "planet", "100"),
    ("japan", "Япония", "Japan", "100"),
    ("cats", "Кошки", "cats", "100"),
    ("macro", "Макро", "macro", "100"),
];

/// «Фото»-категории: на Wallhaven поиск по слову («mountains») в общем разделе находит и игровой,
/// и нарисованный арт (Firewatch, пиксель-арт). Для них тег обязателен («+»), а игры и цифровой
/// арт исключены (проверено на живых запросах: остаются фотографии).
const PHOTO_TOPICS: [&str; 20] = [
    "nature", "mountains", "sea", "forest", "sunset", "winter", "city", "architecture", "animals", "cars",
    "flowers", "lake", "waterfall", "desert", "autumn", "aurora", "clouds", "japan", "cats", "macro",
];
const NOT_ART: &str = r#" -"video games" -"video game art" -"pixel art" -"digital art""#;

/// Запрос Wallhaven для категории.
pub fn topic_query(t: &(&str, &str, &str, &str)) -> String {
    if PHOTO_TOPICS.contains(&t.0) {
        format!("+{}{}", t.2, NOT_ART)
    } else {
        t.2.to_string()
    }
}

/// Какая категория у картинок в кэше: cache\topics.txt, строки «имя_файла<TAB>категория».
/// Нужно запасному пути: без сети берём из кэша только картинки выбранных категорий.
fn topics_file(cache: &Path) -> PathBuf {
    cache.join("topics.txt")
}

fn read_topics(cache: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(topics_file(cache))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_lowercase(), b.to_string())))
        .collect()
}

fn remember_topic(cache: &Path, name: &str, topic: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(topics_file(cache)) {
        let _ = writeln!(f, "{}\t{}", name.to_lowercase(), topic);
    }
}

/// Список выбранных категорий из строки настроек.
pub fn selected_topics(topics: &str) -> Vec<&'static (&'static str, &'static str, &'static str, &'static str)> {
    let ids: Vec<&str> = topics.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    TOPICS.iter().filter(|t| ids.contains(&t.0)).collect()
}

pub struct Dirs {
    pub cache: PathBuf,
    pub folder: PathBuf,
    pub favorites: PathBuf,  // избранное (копии понравившихся обоев)
    pub banned: Vec<String>, // чёрный список: имена картинок без расширения, строчными
}

/// «Имя» картинки для избранного и чёрного списка: имя файла без расширения, строчными
/// (у Wallhaven это wallhaven-xxxxxx — одно и то же у любой копии).
pub fn key_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Имена всех картинок в папке.
pub fn list_keys(dir: &Path) -> Vec<String> {
    list_images(dir).iter().map(|p| key_of(p)).collect()
}

/// Файлы в папке с этим «именем».
pub fn find_by_key(dir: &Path, key: &str) -> Vec<PathBuf> {
    list_images(dir).into_iter().filter(|p| key_of(p) == key).collect()
}

/// Чёрный список из файла: по имени на строку, строки с «;» и «#» — комментарии.
/// Ошибка — только если файл есть, но не прочитался (тогда его нельзя перезаписывать).
pub fn load_blacklist(path: &Path) -> std::io::Result<Vec<String>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // Блокнот при «Юникоде» сохраняет в UTF-16
    let text = if bytes.starts_with(&[0xFF, 0xFE]) {
        let w: Vec<u16> = bytes[2..].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
        String::from_utf16_lossy(&w)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(text
        .lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}').to_lowercase())
        .filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('#'))
        .collect())
}

/// Сохранить чёрный список (через временный файл — сбой не оставит пустой список).
pub fn save_blacklist(path: &Path, list: &[String]) -> std::io::Result<()> {
    let mut s = String::from("; Чёрный список WallClock: эти обои больше не показываются.\r\n; По имени файла без расширения на строку. Удалите строку — картинка снова сможет попасться.\r\n");
    for k in list {
        // в имени не должно быть переводов строки и прочих управляющих символов
        let k: String = k.chars().filter(|c| !c.is_control()).collect();
        if !k.is_empty() {
            s += &k;
            s += "\r\n";
        }
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, s)?;
    std::fs::rename(&tmp, path)
}

pub struct Outcome {
    pub paths: Vec<PathBuf>, // поставленные обои (по одной на монитор или одна на все)
    pub error: Option<String>,
    pub partial: bool, // меняли обои только на одном мониторе — остальные остались прежними
}

/// Что сейчас стоит на каждом мониторе — по данным самой Windows:
/// (идентификатор монитора, прямоугольник, файл обоев).
pub fn on_monitors() -> Vec<(Vec<u16>, windows_sys::Win32::Foundation::RECT, Option<PathBuf>)> {
    let _com = ComGuard::new();
    let Some(d) = DesktopWallpaper::new() else { return Vec::new() };
    d.monitors()
        .into_iter()
        .map(|(id, r)| {
            let p = d.wallpaper_of(&id);
            (id, r, p)
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Src {
    Wallhaven,
    Folder,
    Cache,
    Favorites,
}

/// Главная функция потока: подобрать картинки и поставить их на рабочий стол
/// (одну на все мониторы или, в режиме «разные обои», свою на каждый).
/// only — сменить обои только на этом мониторе (идентификатор из on_monitors), остальные не трогать.
pub fn change(cfg: &Config, dirs: &Dirs, current: &[PathBuf], primary: (i32, i32), only: Option<&[u16]>) -> Outcome {
    let _com = ComGuard::new();
    let mut rng = Rng::new();
    let dw = DesktopWallpaper::new();
    let mons = dw.as_ref().map(|d| d.monitors()).unwrap_or_default();
    // единые обои: одна картинка на весь рабочий стол (все мониторы вместе)
    let span = cfg.wall_mode == "span" && mons.len() > 1;
    // цели: (монитор или все, размер экрана для подбора картинки)
    let per_monitor = cfg.wall_mode == "per_monitor" && mons.len() > 1;
    let partial = per_monitor && only.is_some();
    let targets: Vec<(Option<Vec<u16>>, (i32, i32))> = if per_monitor {
        mons.iter()
            .filter(|(id, _)| only.map(|o| id.as_slice() == o).unwrap_or(true))
            .map(|(id, r)| (Some(id.clone()), (r.right - r.left, r.bottom - r.top)))
            .collect()
    } else if span {
        let (l, t, r, b) = mons.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |a, (_, m)| (a.0.min(m.left), a.1.min(m.top), a.2.max(m.right), a.3.max(m.bottom)));
        vec![(None, (r - l, b - t))]
    } else {
        vec![(None, primary)]
    };
    if let Some(d) = &dw {
        // для единых обоев Windows растягивает картинку на все экраны сразу (режим «расширение»)
        d.set_position(if span { 5 } else { fit_code(&cfg.fit) });
    }
    let mut got: Vec<PathBuf> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (id, screen) in targets {
        // не повторяем ни текущие, ни только что выбранные для соседнего монитора
        let mut exclude: Vec<PathBuf> = current.to_vec();
        exclude.extend(got.iter().cloned());
        match pick_one(cfg, dirs, &exclude, screen, span, &mut rng) {
            Ok(p) => {
                let ok = match &dw {
                    Some(d) => d.set_wallpaper(id.as_deref(), &p),
                    None => id.is_none() && set_classic(&p),
                };
                if ok {
                    got.push(p);
                } else {
                    errors.push(tr("Windows не принял картинку как обои").into());
                }
            }
            Err(e) => errors.push(e),
        }
    }
    if !got.is_empty() {
        let mut keep: Vec<PathBuf> = got.clone();
        keep.extend(current.iter().cloned());
        prune_cache(&dirs.cache, cfg.cache_keep as usize, &keep);
    }
    let error = if errors.is_empty() { None } else { Some(errors.join("; ")) };
    Outcome { paths: got, error, partial }
}

/// Подбирает одну картинку: из выбранного источника, при неудаче — из запасных.
fn pick_one(cfg: &Config, dirs: &Dirs, exclude: &[PathBuf], screen: (i32, i32), wide: bool, rng: &mut Rng) -> Result<PathBuf, String> {
    let only_fav = cfg.source == "favorites";
    let mut order: Vec<Src> = match cfg.source.as_str() {
        // только избранное; если оно пустое — обычные источники, чтобы обои всё же сменились
        "favorites" => vec![Src::Favorites, Src::Wallhaven, Src::Folder, Src::Cache],
        "folder" => vec![Src::Folder, Src::Cache],
        "both" => {
            if rng.below(2) == 0 {
                vec![Src::Wallhaven, Src::Folder, Src::Cache]
            } else {
                vec![Src::Folder, Src::Wallhaven, Src::Cache]
            }
        }
        // источник — только Wallhaven: в свою папку не лезем (там может быть что угодно)
        _ => vec![Src::Wallhaven, Src::Cache],
    };
    // иногда (favorites_mix %) вместо новой картинки показываем что-то из избранного
    if !only_fav && cfg.favorites_mix > 0 && rng.below(100) < cfg.favorites_mix as usize && !list_images(&dirs.favorites).is_empty() {
        order.insert(0, Src::Favorites);
    }
    let mut errors = Vec::new();
    for src in order {
        let got = match src {
            Src::Wallhaven => from_wallhaven(cfg, dirs, screen, wide, rng),
            Src::Folder => pick_local(&dirs.folder, exclude, &dirs.banned, true, rng).ok_or_else(|| tr("в своей папке нет картинок").to_string()),
            Src::Cache => pick_cache(cfg, dirs, exclude, rng).ok_or_else(|| tr("в кэше нет картинок выбранных категорий").to_string()),
            // из избранного повтор текущей допустим, только когда кроме избранного ничего не нужно
            Src::Favorites => pick_local(&dirs.favorites, exclude, &dirs.banned, only_fav, rng).ok_or_else(|| tr("в избранном нет других картинок").to_string()),
        };
        match got {
            Ok(p) => {
                if !errors.is_empty() {
                    crate::util::log(&format!("обои взяты из запасного источника: {}", errors.join("; ")));
                }
                return Ok(p);
            }
            Err(e) => errors.push(e),
        }
    }
    Err(errors.join("; "))
}

/// Картинка из кэша — только из выбранных категорий (если категории не выбраны — любая).
/// Без этого при недоступном Wallhaven вместо «Природы» вставали игры, скачанные раньше.
fn pick_cache(cfg: &Config, dirs: &Dirs, exclude: &[PathBuf], rng: &mut Rng) -> Option<PathBuf> {
    let topics = selected_topics(&cfg.topics);
    if topics.is_empty() {
        return pick_local(&dirs.cache, exclude, &dirs.banned, true, rng);
    }
    let tags = read_topics(&dirs.cache);
    let ok_names: Vec<&str> = tags.iter().filter(|(_, t)| topics.iter().any(|x| x.0 == t)).map(|(n, _)| n.as_str()).collect();
    let taken: Vec<String> = exclude.iter().map(|p| key_of(p)).collect();
    let mut all: Vec<PathBuf> = list_images(&dirs.cache)
        .into_iter()
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            ok_names.contains(&name.as_str()) && !dirs.banned.contains(&key_of(p))
        })
        .collect();
    let rest: Vec<PathBuf> = all.iter().filter(|p| !taken.contains(&key_of(p))).cloned().collect();
    if !rest.is_empty() {
        all = rest;
    }
    if all.is_empty() {
        None
    } else {
        let i = rng.below(all.len());
        Some(all.swap_remove(i))
    }
}

fn ratio_for(w: i32, h: i32) -> &'static str {
    if w <= 0 || h <= 0 {
        return "16x9";
    }
    let r = w as f32 / h as f32;
    let table = [(16.0 / 9.0, "16x9"), (16.0 / 10.0, "16x10"), (21.0 / 9.0, "21x9"), (32.0 / 9.0, "32x9"), (4.0 / 3.0, "4x3"), (3.0 / 2.0, "3x2")];
    let mut best = table[0];
    for t in table {
        if (t.0 - r).abs() < (best.0 - r).abs() {
            best = t;
        }
    }
    best.1
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Адрес картинки — только с сервера картинок Wallhaven по https, имя файла — только буквы,
/// цифры, «-» и «_» и расширение jpg/jpeg/png. Так подменённый ответ не сможет записать файл
/// куда-то за пределы кэша (например, в автозагрузку).
fn safe_image_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://w.wallhaven.cc/") else { return false };
    let name = rest.rsplit('/').next().unwrap_or("");
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    !stem.is_empty()
        && stem.len() <= 64
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png")
}

/// Достаёт все значения "path":"..." из JSON-ответа Wallhaven (полноразмерные картинки).
fn extract_paths(json: &str) -> Vec<String> {
    let mut v = Vec::new();
    let key = "\"path\":\"";
    let mut rest = json;
    while let Some(i) = rest.find(key) {
        rest = &rest[i + key.len()..];
        if let Some(end) = rest.find('"') {
            v.push(rest[..end].replace("\\/", "/"));
            rest = &rest[end..];
        } else {
            break;
        }
    }
    v
}

/// Достаёт число из JSON по ключу, например "last_page":12.
fn json_number(json: &str, key: &str) -> Option<usize> {
    let pat = format!("\"{key}\":");
    let i = json.find(&pat)? + pat.len();
    let digits: String = json[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Подбирает и скачивает картинку с Wallhaven. `wide` — для единых обоев на несколько экранов:
/// сначала ищем сверхширокие картинки, если их нет — просто с большим разрешением.
/// Картинка с Wallhaven. Категорий выбрано несколько — если в одной нет ничего нового,
/// пробуем другую (до трёх), и только потом запасной путь. Ошибку сети не повторяем.
pub fn from_wallhaven(cfg: &Config, dirs: &Dirs, screen: (i32, i32), wide: bool, rng: &mut Rng) -> Result<PathBuf, String> {
    let mut topics = selected_topics(&cfg.topics);
    if topics.is_empty() {
        return wallhaven_one(cfg, dirs, screen, wide, rng, None);
    }
    for i in (1..topics.len()).rev() {
        let j = rng.below(i + 1);
        topics.swap(i, j);
    }
    let nothing = [tr("на Wallhaven не нашлось новых картинок (все уже были)"), tr("Wallhaven ничего не нашёл по этим настройкам")];
    let mut last = String::new();
    for t in topics.into_iter().take(3) {
        match wallhaven_one(cfg, dirs, screen, wide, rng, Some(t)) {
            Ok(p) => return Ok(p),
            Err(e) if nothing.contains(&e.as_str()) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

type Topic = (&'static str, &'static str, &'static str, &'static str);

fn wallhaven_one(cfg: &Config, dirs: &Dirs, screen: (i32, i32), wide: bool, rng: &mut Rng, chosen: Option<&'static Topic>) -> Result<PathBuf, String> {
    let auto_atleast = cfg.atleast.eq_ignore_ascii_case("auto") || cfg.atleast.is_empty();
    // варианты (минимальное разрешение, пропорции) — пробуем по очереди
    let variants: Vec<(String, String)> = if wide {
        let h = screen.1.clamp(720, 2160);
        vec![
            (format!("{}x{}", screen.0.min(3440), h), "21x9,32x9,48x9".to_string()),
            (format!("{}x{}", screen.0.min(3840), h), "16x9,16x10".to_string()),
        ]
    } else {
        let atleast = if auto_atleast { format!("{}x{}", screen.0.max(1280), screen.1.max(720)) } else { cfg.atleast.clone() };
        let ratios = if cfg.ratios.eq_ignore_ascii_case("auto") { ratio_for(screen.0, screen.1).to_string() } else { cfg.ratios.clone() };
        vec![(atleast, ratios)]
    };
    let random = cfg.sorting == "random";
    // категория (выбрал from_wallhaven); без категорий — свой запрос из настроек
    let (query, categories, top_range, topic) = match chosen {
        None => (cfg.query.trim().to_string(), cfg.categories.clone(), cfg.top_range.clone(), ""),
        Some(t) => {
            // в узкой категории за месяц бывает всего несколько картинок — берём популярное за год
            let range = if cfg.top_range == "1M" { "1y".to_string() } else { cfg.top_range.clone() };
            (topic_query(t), t.3.to_string(), range, t.0)
        }
    };
    let mut all_seen = false;
    for (atleast, ratios) in variants {
        let mut base = format!(
            "https://wallhaven.cc/api/v1/search?categories={}&purity=100&atleast={}",
            url_encode(&categories),
            url_encode(&atleast)
        );
        if !ratios.is_empty() {
            base += &format!("&ratios={}", url_encode(&ratios).replace("%2C", ","));
        }
        if !query.is_empty() {
            base += &format!("&q={}", url_encode(&query));
        }
        let mut last_page = 0usize; // сколько всего страниц — узнаём из первого ответа
        let mut page = 1usize;
        // несколько попыток: если на странице всё уже было — берём другую
        for attempt in 0..4 {
            let url = if random {
                let seed: String = (0..6)
                    .map(|_| b"abcdefghijklmnopqrstuvwxyz0123456789"[rng.below(36)] as char)
                    .collect();
                format!("{base}&sorting=random&seed={seed}")
            } else {
                // берём случайную страницу из первых восьми (самые популярные)
                page = if last_page == 0 { 1 + rng.below(3) } else { 1 + rng.below(last_page.min(8)) };
                format!("{base}&sorting=toplist&topRange={}&page={page}", url_encode(&top_range))
            };
            let json = http::get_text(&url)?;
            last_page = json_number(&json, "last_page").unwrap_or(1).max(1);
            let paths: Vec<String> = extract_paths(&json).into_iter().filter(|p| safe_image_url(p)).collect();
            if paths.is_empty() {
                // пусто: либо страниц больше одной, либо попросили страницу за пределами (у узких
                // категорий страница всего одна) — пробуем ещё раз уже в пределах last_page
                if attempt < 2 && !random && (last_page > 1 || page > last_page) {
                    continue;
                }
                break; // в этом варианте ничего нет — пробуем следующий
            }
            let fresh: Vec<&String> = paths
                .iter()
                .filter(|p| {
                    let name = p.rsplit('/').next().unwrap_or("");
                    !name.is_empty() && !dirs.cache.join(name).exists() && !dirs.banned.contains(&key_of(Path::new(name)))
                })
                .collect();
            if fresh.is_empty() {
                all_seen = true;
                continue;
            }
            let pick = fresh[rng.below(fresh.len())];
            let name = pick.rsplit('/').next().unwrap();
            let dst = dirs.cache.join(name);
            http::download(pick, &dst)?;
            remember_topic(&dirs.cache, name, topic);
            return Ok(dst);
        }
    }
    if all_seen {
        Err(tr("на Wallhaven не нашлось новых картинок (все уже были)").into())
    } else {
        Err(tr("Wallhaven ничего не нашёл по этим настройкам").into())
    }
}

fn list_images(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && is_image(p)).collect())
        .unwrap_or_default()
}

/// Случайная картинка из папки: не из чёрного списка и, по возможности, не из уже стоящих.
/// repeat — можно ли вернуть уже стоящую, если других нет (одна картинка в папке).
fn pick_local(dir: &Path, exclude: &[PathBuf], banned: &[String], repeat: bool, rng: &mut Rng) -> Option<PathBuf> {
    let mut all: Vec<PathBuf> = list_images(dir).into_iter().filter(|p| !banned.contains(&key_of(p))).collect();
    let taken: Vec<String> = exclude.iter().map(|p| key_of(p)).collect();
    let rest: Vec<PathBuf> = all.iter().filter(|p| !taken.contains(&key_of(p))).cloned().collect();
    if !rest.is_empty() || !repeat {
        all = rest;
    }
    if all.is_empty() {
        None
    } else {
        let i = rng.below(all.len());
        Some(all.swap_remove(i))
    }
}

/// Оставляет в кэше только `keep` самых свежих файлов.
fn prune_cache(cache: &Path, keep: usize, current: &[PathBuf]) {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(cache)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if name.starts_with("bg-") {
                        return None; // файлы фона под часами — не наши, их ведёт bg.rs
                    }
                    if p.extension().map(|x| x == "part").unwrap_or(false) {
                        let _ = std::fs::remove_file(&p);
                        return None;
                    }
                    if !is_image(&p) {
                        return None;
                    }
                    Some((e.metadata().ok()?.modified().ok()?, p))
                })
                .collect()
        })
        .unwrap_or_default();
    if files.len() <= keep.max(2) {
        return;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in files.into_iter().skip(keep.max(2)) {
        if !current.contains(&p) {
            let _ = std::fs::remove_file(p);
        }
    }
    // записи о категориях — только для файлов, что остались
    let tags = read_topics(cache);
    if !tags.is_empty() {
        let kept: String = tags
            .iter()
            .filter(|(n, _)| cache.join(n).exists())
            .map(|(n, t)| format!("{n}\t{t}\n"))
            .collect();
        let _ = std::fs::write(topics_file(cache), kept);
    }
}

// --- установка обоев через COM-интерфейс IDesktopWallpaper (с плавным переходом, по мониторам) ---

#[repr(C)]
struct IDesktopWallpaperVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_wallpaper: unsafe extern "system" fn(*mut c_void, *const u16, *const u16) -> i32,
    get_wallpaper: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut u16) -> i32,
    get_monitor_device_path_at: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32,
    get_monitor_device_path_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    get_monitor_rect: unsafe extern "system" fn(*mut c_void, *const u16, *mut RECT) -> i32,
    set_background_color: usize,
    get_background_color: usize,
    set_position: unsafe extern "system" fn(*mut c_void, i32) -> i32,
}

use windows_sys::Win32::Foundation::RECT;

/// Инициализация COM на время жизни потока.
pub struct ComGuard(bool);
impl ComGuard {
    pub fn new() -> Self {
        let hr = unsafe { windows_sys::Win32::System::Com::CoInitializeEx(std::ptr::null(), windows_sys::Win32::System::Com::COINIT_MULTITHREADED as _) };
        ComGuard(hr >= 0)
    }
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// Обёртка над системным объектом «обои рабочего стола».
pub struct DesktopWallpaper(*mut c_void);

impl DesktopWallpaper {
    pub fn new() -> Option<Self> {
        use windows_sys::core::GUID;
        use windows_sys::Win32::System::Com::*;
        let clsid = GUID::from_u128(0xC2CF3110_460E_4fc1_B9D0_8A1C0C9CC4BD);
        let iid = GUID::from_u128(0xB92B56A9_8B55_4E14_9A89_0199BBB6F93B);
        let mut obj: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { CoCreateInstance(&clsid, std::ptr::null_mut(), CLSCTX_LOCAL_SERVER | CLSCTX_INPROC_SERVER, &iid, &mut obj) };
        if hr >= 0 && !obj.is_null() {
            Some(DesktopWallpaper(obj))
        } else {
            None
        }
    }

    fn vt(&self) -> &IDesktopWallpaperVtbl {
        unsafe { &**(self.0 as *mut *const IDesktopWallpaperVtbl) }
    }

    unsafe fn take_string(p: *mut u16) -> Option<String> {
        if p.is_null() {
            return None;
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        windows_sys::Win32::System::Com::CoTaskMemFree(p as *const c_void);
        Some(s)
    }

    /// Подключённые мониторы: (идентификатор для Windows, прямоугольник на экране).
    pub fn monitors(&self) -> Vec<(Vec<u16>, RECT)> {
        let mut v = Vec::new();
        unsafe {
            let mut n = 0u32;
            if (self.vt().get_monitor_device_path_count)(self.0, &mut n) < 0 {
                return v;
            }
            for i in 0..n {
                let mut p: *mut u16 = std::ptr::null_mut();
                if (self.vt().get_monitor_device_path_at)(self.0, i, &mut p) < 0 {
                    continue;
                }
                let Some(id) = Self::take_string(p) else { continue };
                let wid = crate::util::wide(&id);
                let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                // для отключённых мониторов Windows не отдаёт прямоугольник — пропускаем их
                if (self.vt().get_monitor_rect)(self.0, wid.as_ptr(), &mut r) == 0 && r.right > r.left && r.bottom > r.top {
                    v.push((wid, r));
                }
            }
        }
        v
    }

    /// Какие обои сейчас стоят на мониторе.
    pub fn wallpaper_of(&self, id: &[u16]) -> Option<PathBuf> {
        unsafe {
            let mut p: *mut u16 = std::ptr::null_mut();
            if (self.vt().get_wallpaper)(self.0, id.as_ptr(), &mut p) < 0 {
                return None;
            }
            Self::take_string(p).filter(|s| !s.is_empty()).map(PathBuf::from)
        }
    }

    /// Ставит обои на монитор (None — на все сразу).
    pub fn set_wallpaper(&self, id: Option<&[u16]>, path: &Path) -> bool {
        let wpath = wide_os(path.as_os_str());
        unsafe { (self.vt().set_wallpaper)(self.0, id.map(|s| s.as_ptr()).unwrap_or(std::ptr::null()), wpath.as_ptr()) >= 0 }
    }

    pub fn set_position(&self, pos: i32) {
        unsafe { (self.vt().set_position)(self.0, pos) };
    }
}

impl Drop for DesktopWallpaper {
    fn drop(&mut self) {
        unsafe { (self.vt().release)(self.0) };
    }
}

fn fit_code(fit: &str) -> i32 {
    match fit {
        "center" => 0,
        "tile" => 1,
        "stretch" => 2,
        "fit" => 3,
        "span" => 5,
        _ => 4, // fill
    }
}

/// Запасной путь — старый системный вызов (одна картинка на все мониторы).
fn set_classic(path: &Path) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let wpath = wide_os(path.as_os_str());
    unsafe { SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, wpath.as_ptr() as *mut c_void, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE) != 0 }
}

/// Меняет только способ растяжения обоев (заполнить/вписать/...), картинки остаются.
pub fn set_fit(fit: &str) {
    let _com = ComGuard::new();
    if let Some(d) = DesktopWallpaper::new() {
        d.set_position(fit_code(fit));
    }
}
