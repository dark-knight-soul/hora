// Падения программы.
//
// Если Hora упала (ошибка в коде Rust — «паника» — или сбой уровня системы, например обращение
// к чужой памяти), то:
//   1) причина записывается в crash.txt рядом с настройками (версия, Windows, где упала);
//   2) главная программа один раз перезапускается сама (чтобы часы и обои не пропали с экрана),
//      но не чаще раза в 2 минуты — иначе падение по кругу съело бы процессор;
//   3) при следующем запуске значок в трее предлагает сообщить об ошибке: щелчок открывает
//      на GitHub готовую форму с текстом отчёта. Само ничего не отправляется — человек видит
//      текст и сам решает, нажимать ли «Submit».

use crate::util::{app_dir, wide};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static RESTART: AtomicBool = AtomicBool::new(false);
static ROLE_LIVE: AtomicBool = AtomicBool::new(false);
static FIRED: AtomicBool = AtomicBool::new(false);

const ISSUES: &str = "https://github.com/dark-knight-soul/hora/issues/new";

fn report_path() -> PathBuf {
    app_dir().join("crash.txt")
}

/// Метка «о падении ещё не сообщали» (её снимает следующий запуск).
fn marker_path() -> PathBuf {
    app_dir().join("crash.new")
}

/// Включить перехват падений. restart — перезапускать ли программу (главный процесс — да;
/// процесс живых обоев — нет, его и так перезапустит главная программа).
pub fn install(restart: bool) {
    RESTART.store(restart, Ordering::Relaxed);
    ROLE_LIVE.store(!restart, Ordering::Relaxed);
    std::panic::set_hook(Box::new(|info| {
        let p = info.payload();
        let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "?".into());
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        handle(&format!("panic: {msg}\nwhere: {place}\nthread: {thread}"));
    }));
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(on_exception));
    }
}

/// Сбой уровня системы (исключение, которое никто не обработал).
unsafe extern "system" fn on_exception(ep: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    if !ep.is_null() && !(*ep).ExceptionRecord.is_null() {
        let rec = &*(*ep).ExceptionRecord;
        let addr = rec.ExceptionAddress as usize;
        handle(&format!("exception: 0x{:08X}\nat: {}", rec.ExceptionCode as u32, module_offset(addr)));
    }
    1 // EXCEPTION_EXECUTE_HANDLER — завершить процесс
}

/// «Hora.exe+0x1A2B3» — по смещению внутри модуля место находится в той же сборке.
fn module_offset(addr: usize) -> String {
    use windows_sys::Win32::System::LibraryLoader::*;
    unsafe {
        let mut m: windows_sys::Win32::Foundation::HMODULE = std::ptr::null_mut();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        if GetModuleHandleExW(flags, addr as *const u16, &mut m) == 0 || m.is_null() {
            return format!("0x{addr:X}");
        }
        let mut buf = [0u16; 260];
        let n = GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
        let path = String::from_utf16_lossy(&buf[..n]);
        // только имя файла — без папок (в них может быть имя пользователя)
        let name = path.rsplit(['\\', '/']).next().unwrap_or("?").to_string();
        format!("{name}+0x{:X}", addr - m as usize)
    }
}

fn handle(details: &str) {
    // второе падение во время записи первого — не повторяемся
    if FIRED.swap(true, Ordering::SeqCst) {
        return;
    }
    // журнал (util::log) здесь не трогаем: он под замком, а упасть могли как раз внутри него
    let prev = std::fs::metadata(report_path()).and_then(|m| m.modified()).ok();
    let mut st = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st) };
    let role = if ROLE_LIVE.load(Ordering::Relaxed) { "live wallpaper process" } else { "main" };
    let text = format!(
        "Hora {} ({role})\ntime: {:04}-{:02}-{:02} {:02}:{:02}:{:02}\nwindows: {}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        st.wYear,
        st.wMonth,
        st.wDay,
        st.wHour,
        st.wMinute,
        st.wSecond,
        windows_version(),
        sanitize(details)
    );
    let _ = std::fs::write(report_path(), &text);
    let _ = std::fs::write(marker_path(), "");
    let recent = prev.and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() < 120).unwrap_or(false);
    if RESTART.load(Ordering::Relaxed) && !recent {
        if let Ok(exe) = std::env::current_exe() {
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg("--after-crash").arg(std::process::id().to_string());
            if let Some(dir) = exe.parent() {
                cmd.current_dir(dir);
            }
            let _ = cmd.spawn();
        }
    }
}

/// Убрать из текста папку профиля (там имя пользователя Windows).
fn sanitize(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(home) = std::env::var_os("USERPROFILE").map(|h| h.to_string_lossy().to_string()).filter(|h| h.len() > 3) {
        out = out.replace(&home, "%USERPROFILE%");
        out = out.replace(&home.replace('\\', "/"), "%USERPROFILE%");
    }
    out
}

/// «Windows 11 25H2 (26200.7171)» — из реестра.
pub fn windows_version() -> String {
    use windows_sys::Win32::System::Registry::*;
    const KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let get_str = |name: &str| -> Option<String> {
        let mut buf = [0u16; 64];
        let mut size = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, wide(KEY).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size) };
        if r != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    };
    let mut ubr = 0u32;
    let mut size = 4u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, wide(KEY).as_ptr(), wide("UBR").as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut ubr as *mut u32 as *mut _, &mut size) };
    let build = get_str("CurrentBuild").unwrap_or_default();
    // в реестре Windows 11 до сих пор пишет «Windows 10» — различаем по номеру сборки
    let name = if build.parse::<u32>().unwrap_or(0) >= 22000 { "Windows 11" } else { "Windows 10" };
    let disp = get_str("DisplayVersion").unwrap_or_default();
    format!("{name} {disp} ({build}.{ubr})").replace("  ", " ")
}

/// Перезапуск после падения: дождаться, пока упавший процесс завершится (иначе он ещё держит
/// защиту «только одна копия программы»).
pub fn wait_for(pid: u32) {
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 10_000);
            windows_sys::Win32::Foundation::CloseHandle(h);
        }
    }
}

/// Было ли падение, о котором ещё не сообщили (метку снимаем: предлагаем один раз).
pub fn take_new() -> bool {
    let p = marker_path();
    if p.exists() {
        let _ = std::fs::remove_file(p);
        report_path().exists()
    } else {
        false
    }
}

/// Ссылка на форму «Ошибка» на GitHub с уже заполненными полями (версия, Windows, отчёт о падении).
pub fn issue_url() -> String {
    let report = std::fs::read_to_string(report_path()).unwrap_or_default();
    let mut url = format!("{ISSUES}?template=bug_report.yml&version={}&windows={}", enc(env!("CARGO_PKG_VERSION")), enc(&windows_version()));
    if !report.is_empty() {
        let title = if crate::i18n::en() { "Hora crashed" } else { "Hora закрылась с ошибкой" };
        let what = if crate::i18n::en() { "Hora closed unexpectedly. What I was doing at that moment: " } else { "Hora неожиданно закрылась. Что я делал в этот момент: " };
        // отчёт длинный не бывает, но ссылку всё равно ограничиваем
        let report: String = sanitize(&report).chars().take(1500).collect();
        url += &format!("&title={}&what={}&log={}", enc(title), enc(what), enc(&report));
    }
    url
}

/// Кодирование для адреса (всё, кроме букв, цифр и -_.~ — в %XX по байтам UTF-8).
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding() {
        assert_eq!(enc("a b/ё"), "a%20b%2F%D1%91");
    }

    #[test]
    fn url_has_fields() {
        let u = issue_url();
        assert!(u.starts_with("https://github.com/dark-knight-soul/hora/issues/new?template=bug_report.yml&version="));
        assert!(u.contains("&windows=Windows"));
    }
}
