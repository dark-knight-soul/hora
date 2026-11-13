// Погода: данные с open-meteo.com (бесплатно, без ключа). Поиск города — их же геокодер.
// Запросы идут в фоновом потоке; разбор JSON — простой, без сторонних библиотек.
use crate::http;

#[derive(Clone, Debug)]
pub struct Day {
    pub dow: usize, // день недели: 0 = воскресенье
    pub tmax: f64,
    pub tmin: f64,
}

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: f64,
    pub feels: f64,
    pub humidity: f64,
    pub wind: f64, // м/с
    pub code: i32,
    pub is_day: bool,
    pub days: Vec<Day>, // сегодня и следующие дни
    pub fetched: (u16, u16), // во сколько обновили: (часы, минуты)
}

#[derive(Clone, Debug)]
pub struct City {
    pub name: String,
    pub region: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

impl City {
    /// «Берлин, Германия» (с областью, если она отличается от названия)
    pub fn label(&self) -> String {
        let mut parts = vec![self.name.clone()];
        if !self.region.is_empty() && self.region != self.name {
            parts.push(self.region.clone());
        }
        if !self.country.is_empty() {
            parts.push(self.country.clone());
        }
        parts.join(", ")
    }
}

// ---------------- простой разбор JSON ----------------

/// Число после "ключ": в куске JSON.
fn num(json: &str, key: &str) -> Option<f64> {
    let pat = format!("\"{key}\":");
    let i = json.find(&pat)? + pat.len();
    let s: String = json[i..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.' || *c == 'e' || *c == 'E' || *c == '+').collect();
    s.parse().ok()
}

/// Строка после "ключ": (с раскодированием \uXXXX и \").
fn text(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let i = json.find(&pat)? + pat.len();
    let mut out = String::new();
    let mut chars = json[i..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out.chars().filter(|c| !c.is_control()).collect()),
            '\\' => match chars.next() {
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(x) => out.push(x),
                None => break,
            },
            _ => out.push(c),
        }
    }
    None
}

/// Массив чисел после "ключ":[ ... ].
fn nums(json: &str, key: &str) -> Vec<f64> {
    let pat = format!("\"{key}\":[");
    let Some(i) = json.find(&pat) else { return Vec::new() };
    let rest = &json[i + pat.len()..];
    let end = rest.find(']').unwrap_or(rest.len());
    rest[..end].split(',').filter_map(|s| s.trim().parse().ok()).collect()
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Ищет города по названию (до 6 вариантов).
pub fn search(query: &str, lang: &str) -> Result<Vec<City>, String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=6&language={}&format=json",
        url_encode(query),
        if lang == "en" { "en" } else { "ru" }
    );
    let json = http::get_text(&url)?;
    let Some(start) = json.find("\"results\"") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for part in json[start..].split("{\"id\":").skip(1) {
        let (Some(name), Some(lat), Some(lon)) = (text(part, "name"), num(part, "latitude"), num(part, "longitude")) else { continue };
        // координаты из ответа — только настоящие (не бесконечность и в пределах Земли)
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
            continue;
        }
        out.push(City {
            name,
            region: text(part, "admin1").unwrap_or_default(),
            country: text(part, "country").unwrap_or_default(),
            lat,
            lon,
        });
    }
    Ok(out)
}

/// Загружает текущую погоду и прогноз на 4 дня.
pub fn fetch(lat: f64, lon: f64) -> Result<Weather, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,is_day\
&daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=4&wind_speed_unit=ms"
    );
    let json = http::get_text(&url)?;
    let cur_at = json.find("\"current\":{").ok_or("в ответе нет текущей погоды")?;
    let cur = &json[cur_at..];
    let cur = &cur[..cur.find('}').unwrap_or(cur.len())];
    let daily_at = json.find("\"daily\":{").unwrap_or(json.len());
    let daily = &json[daily_at..];
    let tmax = nums(daily, "temperature_2m_max");
    let tmin = nums(daily, "temperature_2m_min");
    let mut st: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    let today = st.wDayOfWeek as usize;
    let days = (0..tmax.len().min(tmin.len()))
        .map(|i| Day { dow: (today + i) % 7, tmax: tmax[i], tmin: tmin[i] })
        .collect();
    Ok(Weather {
        temp: num(cur, "temperature_2m").ok_or("нет температуры")?,
        feels: num(cur, "apparent_temperature").unwrap_or(f64::NAN),
        humidity: num(cur, "relative_humidity_2m").unwrap_or(f64::NAN),
        wind: num(cur, "wind_speed_10m").unwrap_or(f64::NAN),
        code: num(cur, "weather_code").unwrap_or(0.0) as i32,
        is_day: num(cur, "is_day").unwrap_or(1.0) != 0.0,
        days,
        fetched: (st.wHour, st.wMinute),
    })
}

// ---------------- описание погоды ----------------

/// Значок погоды (символ из шрифта Segoe UI Symbol, он есть в каждой Windows).
pub fn icon(code: i32, is_day: bool) -> &'static str {
    match code {
        0 | 1 => {
            if is_day { "☀" } else { "☾" }
        }
        2 => {
            if is_day { "⛅" } else { "☁" }
        }
        3 | 45 | 48 => "☁",
        51..=67 | 80..=82 => "☂",
        71..=77 | 85 | 86 => "❄",
        95..=99 => "⚡",
        _ => "☁",
    }
}

/// Словесное описание по коду погоды (стандарт WMO).
pub fn describe(code: i32, en: bool) -> &'static str {
    let (ru, eng) = match code {
        0 => ("Ясно", "Clear"),
        1 => ("Преимущественно ясно", "Mostly clear"),
        2 => ("Переменная облачность", "Partly cloudy"),
        3 => ("Пасмурно", "Overcast"),
        45 | 48 => ("Туман", "Fog"),
        51..=55 => ("Морось", "Drizzle"),
        56 | 57 => ("Ледяная морось", "Freezing drizzle"),
        61..=65 => ("Дождь", "Rain"),
        66 | 67 => ("Ледяной дождь", "Freezing rain"),
        71..=75 => ("Снег", "Snow"),
        77 => ("Снежная крупа", "Snow grains"),
        80..=82 => ("Ливень", "Showers"),
        85 | 86 => ("Снегопад", "Snow showers"),
        95 => ("Гроза", "Thunderstorm"),
        96 | 99 => ("Гроза с градом", "Thunderstorm, hail"),
        _ => ("—", "—"),
    };
    if en { eng } else { ru }
}

/// «12°» / «-3°» — без «+», с обычным дефисом (он есть во всех шрифтах).
pub fn deg(t: f64) -> String {
    let v = t.round() as i32;
    format!("{}°", if v == 0 { 0 } else { v })
}
