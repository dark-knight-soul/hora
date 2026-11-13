// Установка и удаление Hora без сторонних программ.
//
// Установщик — это та же программа, только файл называется HoraSetup.exe: увидев «setup» в имени,
// она спрашивает, для кого ставить, и копирует себя как Hora.exe:
//   • «только для меня» — в %LOCALAPPDATA%\Programs\Hora, права администратора не нужны;
//   • «для всех пользователей» — в Program Files (Windows попросит подтвердить права).
// Создаёт ярлык в меню «Пуск» и запись в «Параметры → Приложения», откуда программу можно удалить
// (Hora.exe --uninstall). Настройки установленной версии лежат в %LOCALAPPDATA%\Hora у каждого пользователя.

use crate::util::{log, wide};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const TITLE: &str = "Установка Hora";
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Hora";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const ID_ME: i32 = 101;
const ID_ALL: i32 = 102;

/// Это установщик? (файл называется HoraSetup.exe или похоже)
pub fn is_setup_exe() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .map(|s| s.contains("setup"))
        .unwrap_or(false)
}

// ---------------- окна-диалоги (стандартный TaskDialog Windows) ----------------

type TaskDialogFn = unsafe extern "system" fn(*const windows_sys::Win32::UI::Controls::TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> i32;

/// TaskDialogIndirect берём из comctl32 версии 6 (её даёт манифест) — подгружаем по требованию,
/// чтобы сама программа запускалась, даже если с диалогами что-то не так.
fn task_dialog_fn() -> Option<TaskDialogFn> {
    unsafe {
        let lib = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide("comctl32.dll").as_ptr());
        if lib.is_null() {
            return None;
        }
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(lib, b"TaskDialogIndirect\0".as_ptr()).map(|f| std::mem::transmute::<_, TaskDialogFn>(f))
    }
}

/// Кнопке «для всех» — значок щита (понадобятся права администратора).
unsafe extern "system" fn shield_cb(hwnd: HWND, msg: i32, _wp: WPARAM, _lp: LPARAM, _data: isize) -> i32 {
    if msg == 0 {
        // TDN_CREATED → TDM_SET_BUTTON_ELEVATION_REQUIRED_STATE
        SendMessageW(hwnd, WM_USER + 115, ID_ALL as usize, 1);
    }
    0
}

struct Dialog<'a> {
    main: &'a str,
    content: &'a str,
    links: &'a [(i32, &'a str)], // большие кнопки-ссылки: (id, «заголовок\nпояснение»)
    common: i32,                 // обычные кнопки: TDCBF_*
    check: Option<(&'a str, bool)>,
    shield: bool,
}

/// Показать диалог. Возвращает (нажатая кнопка, отмечен ли флажок).
fn dialog(d: &Dialog) -> (i32, bool) {
    use windows_sys::Win32::UI::Controls::*;
    let Some(td) = task_dialog_fn() else {
        // запасной вариант — простое окно сообщения
        let text = format!("{}\n\n{}", d.main, d.content);
        let yes_no = d.common & windows_sys::Win32::UI::Controls::TDCBF_YES_BUTTON != 0;
        let style = if yes_no { MB_YESNO } else { MB_OKCANCEL };
        let r = unsafe { MessageBoxW(std::ptr::null_mut(), wide(&text).as_ptr(), wide(TITLE).as_ptr(), style | MB_ICONINFORMATION) };
        let b = match r {
            IDYES => IDYES,
            IDOK => d.links.first().map(|l| l.0).unwrap_or(IDOK),
            _ => IDCANCEL,
        };
        return (b, d.check.map(|c| c.1).unwrap_or(false));
    };
    let title = wide(TITLE);
    let main = wide(d.main);
    let content = wide(d.content);
    let link_text: Vec<Vec<u16>> = d.links.iter().map(|l| wide(l.1)).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = d.links.iter().zip(&link_text).map(|(l, t)| TASKDIALOG_BUTTON { nButtonID: l.0, pszButtonText: t.as_ptr() }).collect();
    let check = d.check.map(|c| wide(c.0));
    unsafe {
        let mut c: TASKDIALOGCONFIG = std::mem::zeroed();
        c.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
        c.hInstance = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        c.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | if buttons.is_empty() { 0 } else { TDF_USE_COMMAND_LINKS } | if d.check.map(|c| c.1).unwrap_or(false) { TDF_VERIFICATION_FLAG_CHECKED } else { 0 };
        c.dwCommonButtons = d.common;
        c.pszWindowTitle = title.as_ptr();
        c.Anonymous1.pszMainIcon = 1 as _; // значок программы (ресурс 1)
        c.pszMainInstruction = main.as_ptr();
        c.pszContent = content.as_ptr();
        c.cButtons = buttons.len() as u32;
        c.pButtons = if buttons.is_empty() { std::ptr::null() } else { buttons.as_ptr() };
        if let Some(t) = &check {
            c.pszVerificationText = t.as_ptr();
        }
        if d.shield {
            c.pfCallback = Some(shield_cb);
        }
        let (mut button, mut checked) = (0i32, 0 as BOOL);
        if td(&c, &mut button, std::ptr::null_mut(), &mut checked) < 0 {
            return (IDCANCEL, false);
        }
        (button, checked != 0)
    }
}

fn message(main: &str, content: &str) {
    dialog(&Dialog { main, content, links: &[], common: windows_sys::Win32::UI::Controls::TDCBF_OK_BUTTON, check: None, shield: false });
}

// ---------------- папки ----------------

fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    unsafe {
        let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
        // KF_FLAG_CREATE: папки «Programs» в профиле может ещё не быть
        let hr = SHGetKnownFolderPath(id, 0x8000, std::ptr::null_mut(), &mut p);
        let out = if hr >= 0 && !p.is_null() {
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            Some(PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(p, n))))
        } else {
            None
        };
        if !p.is_null() {
            windows_sys::Win32::System::Com::CoTaskMemFree(p as *const _);
        }
        out
    }
}

/// Куда ставить: для себя или для всех.
fn install_dir(all: bool) -> Option<PathBuf> {
    known_folder(if all { &FOLDERID_ProgramFiles } else { &FOLDERID_UserProgramFiles }).map(|p| p.join("Hora"))
}

/// Ярлык в меню «Пуск».
fn shortcut_path(all: bool) -> Option<PathBuf> {
    known_folder(if all { &FOLDERID_CommonPrograms } else { &FOLDERID_Programs }).map(|p| p.join("Hora.lnk"))
}

// ---------------- установка ----------------

/// Точка входа установщика (HoraSetup.exe).
pub fn run() {
    let me_dir = install_dir(false).map(|p| p.display().to_string()).unwrap_or_default();
    let all_dir = install_dir(true).map(|p| p.display().to_string()).unwrap_or_default();
    let me = format!("Только для меня\nБез прав администратора. Папка: {me_dir}");
    let all = format!("Для всех пользователей этого компьютера\nНужны права администратора. Папка: {all_dir}");
    loop {
        let (choice, _) = dialog(&Dialog {
            main: "Для кого установить Hora?",
            content: "Hora меняет обои, показывает часы и погоду на рабочем столе. Настройки у каждого пользователя свои.",
            links: &[(ID_ME, &me), (ID_ALL, &all)],
            common: windows_sys::Win32::UI::Controls::TDCBF_CANCEL_BUTTON,
            check: None,
            shield: true,
        });
        let ok = match choice {
            ID_ME => install(false),
            ID_ALL => match run_elevated("--install-all") {
                Some(0) => true,
                Some(_) => false, // ошибку уже показал процесс с правами администратора
                None => continue, // отказались подтверждать права — спросим ещё раз
            },
            _ => return,
        };
        if ok {
            finish(choice == ID_ALL);
        }
        return;
    }
}

/// Вызывается с правами администратора (HoraSetup.exe --install-all). Код выхода: 0 — успех.
pub fn install_elevated() -> i32 {
    if install(true) {
        0
    } else {
        1
    }
}

fn install(all: bool) -> bool {
    match try_install(all) {
        Ok(()) => true,
        Err(e) => {
            log(&format!("установка не удалась: {e}"));
            message("Не удалось установить Hora", &e);
            false
        }
    }
}

fn try_install(all: bool) -> Result<(), String> {
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir(all).ok_or("не нашлась папка для программ")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("не удалось создать папку {}: {e}", dir.display()))?;
    let exe = dir.join("Hora.exe");
    close_running();
    // файл может быть ещё занят только что закрытой программой — несколько попыток
    let mut last = String::new();
    for _ in 0..20 {
        match std::fs::copy(&src, &exe) {
            Ok(_) => {
                last.clear();
                break;
            }
            Err(e) => {
                last = e.to_string();
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
    if !last.is_empty() {
        return Err(format!("не удалось скопировать программу: {last}"));
    }
    // копия наследует метку «скачано из интернета» (поток Zone.Identifier) — у установленной
    // программы она не нужна (иначе предупреждения при автозапуске)
    let mut ads = exe.clone().into_os_string();
    ads.push(":Zone.Identifier");
    let _ = std::fs::remove_file(ads);
    if let Some(lnk) = shortcut_path(all) {
        if let Err(e) = make_shortcut(&lnk, &exe, &dir) {
            log(&format!("ярлык не создан: {e}"));
        }
    }
    register_uninstall(all, &exe, &dir).map_err(|e| format!("не удалось записать программу в список установленных: {e}"))?;
    log(&format!("Hora установлена в {}", dir.display()));
    Ok(())
}

/// Последний шаг: «готово» и, по желанию, запуск (всегда без прав администратора).
fn finish(all: bool) {
    let Some(dir) = install_dir(all) else { return };
    let (_, start) = dialog(&Dialog {
        main: "Hora установлена",
        content: "Ярлык — в меню «Пуск». Удалить программу можно в «Параметры → Приложения».",
        links: &[],
        common: windows_sys::Win32::UI::Controls::TDCBF_OK_BUTTON,
        check: Some(("Запустить Hora", true)),
        shield: false,
    });
    if start {
        let _ = std::process::Command::new(dir.join("Hora.exe")).current_dir(&dir).spawn();
    }
}

/// Запустить себя же с правами администратора и дождаться. None — права не подтвердили.
fn run_elevated(args: &str) -> Option<u32> {
    unsafe {
        let exe = std::env::current_exe().ok()?;
        let file = wide(&exe.to_string_lossy());
        let (verb, params) = (wide("runas"), wide(args));
        let mut si: SHELLEXECUTEINFOW = std::mem::zeroed();
        si.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        si.fMask = SEE_MASK_NOCLOSEPROCESS;
        si.lpVerb = verb.as_ptr();
        si.lpFile = file.as_ptr();
        si.lpParameters = params.as_ptr();
        si.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut si) == 0 || si.hProcess.is_null() {
            return None;
        }
        windows_sys::Win32::System::Threading::WaitForSingleObject(si.hProcess, u32::MAX);
        let mut code = 1u32;
        windows_sys::Win32::System::Threading::GetExitCodeProcess(si.hProcess, &mut code);
        CloseHandle(si.hProcess);
        Some(code)
    }
}

/// Закрыть запущенную Hora (на время установки или удаления) и дождаться её выхода.
fn close_running() {
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let h = FindWindowW(wide("WallClockMain").as_ptr(), std::ptr::null());
        if h.is_null() {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid == GetCurrentProcessId() {
            return;
        }
        let ph = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid);
        PostMessageW(h, WM_CLOSE, 0, 0);
        if !ph.is_null() {
            if WaitForSingleObject(ph, 5000) != WAIT_OBJECT_0 {
                TerminateProcess(ph, 0);
            }
            CloseHandle(ph);
        }
        // процесс живых обоев закроется сам, когда оборвётся канал
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

fn make_shortcut(lnk: &Path, exe: &Path, dir: &Path) -> windows::core::Result<()> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        link.SetDescription(&HSTRING::from("Обои, часы и погода на рабочем столе"))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(lnk.as_os_str()), true)?;
    }
    Ok(())
}

fn root(all: bool) -> HKEY {
    if all {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    }
}

fn register_uninstall(all: bool, exe: &Path, dir: &Path) -> Result<(), String> {
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let r = RegCreateKeyExW(root(all), wide(UNINSTALL_KEY).as_ptr(), 0, std::ptr::null(), 0, KEY_WRITE, std::ptr::null(), &mut key, std::ptr::null_mut());
        if r != 0 {
            return Err(format!("код {r}"));
        }
        let set_str = |name: &str, val: &str| {
            let v = wide(val);
            RegSetValueExW(key, wide(name).as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
        };
        let set_dw = |name: &str, val: u32| {
            RegSetValueExW(key, wide(name).as_ptr(), 0, REG_DWORD, &val as *const u32 as *const u8, 4);
        };
        let exe_s = exe.display().to_string();
        set_str("DisplayName", "Hora");
        set_str("DisplayVersion", env!("CARGO_PKG_VERSION"));
        set_str("Publisher", "Hora");
        set_str("DisplayIcon", &format!("{exe_s},0"));
        set_str("InstallLocation", &dir.display().to_string());
        set_str("UninstallString", &format!("\"{exe_s}\" --uninstall"));
        set_dw("NoModify", 1);
        set_dw("NoRepair", 1);
        let kb = std::fs::metadata(exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
        set_dw("EstimatedSize", kb);
        RegCloseKey(key);
    }
    Ok(())
}

// ---------------- удаление ----------------

/// Hora.exe --uninstall (запускается из «Параметры → Приложения»).
pub fn uninstall() {
    let Some(exe) = std::env::current_exe().ok() else { return };
    let Some(dir) = exe.parent().map(|p| p.to_path_buf()) else { return };
    // удаляем только то, что ставил установщик: папку Hora в «Program Files» или в «Programs» профиля
    let all = install_dir(true).map(|d| same_path(&d, &dir)).unwrap_or(false);
    let mine = install_dir(false).map(|d| same_path(&d, &dir)).unwrap_or(false);
    if !all && !mine {
        message("Это переносная версия Hora", "Её не нужно удалять через «Приложения» — просто удалите папку с программой.");
        return;
    }
    let (b, wipe) = dialog(&Dialog {
        main: "Удалить Hora?",
        content: "Будут удалены программа, ярлык в меню «Пуск» и автозапуск.",
        links: &[],
        common: windows_sys::Win32::UI::Controls::TDCBF_YES_BUTTON | windows_sys::Win32::UI::Controls::TDCBF_NO_BUTTON,
        check: Some(("Удалить также мои настройки, избранное и скачанные обои", false)),
        shield: false,
    });
    if b != IDYES {
        return;
    }
    close_running();
    // личное — здесь, в процессе самого пользователя (права администратора может подтвердить
    // другая учётная запись, и тогда её HKCU и её папка данных были бы не теми)
    unsafe {
        // автозапуск убираем, только если он ведёт в удаляемую папку (рядом может жить переносная копия)
        for name in ["Hora", "WallClock"] {
            if run_target(name).map(|p| p.starts_with(&dir)).unwrap_or(false) {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(name).as_ptr());
            }
        }
    }
    if wipe {
        if let Some(data) = known_folder(&FOLDERID_LocalAppData).map(|p| p.join("Hora")) {
            let _ = std::fs::remove_dir_all(data);
        }
    }
    if all && unsafe { IsUserAnAdmin() } == 0 {
        // ярлык для всех, запись в HKLM и папка в Program Files — только с правами администратора
        if run_elevated("--uninstall-system") != Some(0) {
            message("Hora не удалена", "Без прав администратора программу «для всех пользователей» удалить нельзя.");
            return;
        }
    } else {
        remove_system_parts(all, &dir);
    }
    message("Hora удалена", "Спасибо, что пользовались!");
    if !all {
        delete_dir_later(&dir); // уже после последнего окна: сразу выходим, папка освобождается
    }
}

/// Hora.exe --uninstall-system (с правами администратора): системная часть удаления «для всех».
pub fn uninstall_system() -> i32 {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) else { return 1 };
    // только настоящая папка установки «для всех» и только с правами администратора
    if !install_dir(true).map(|d| same_path(&d, &dir)).unwrap_or(false) || unsafe { IsUserAnAdmin() } == 0 {
        return 1;
    }
    remove_system_parts(true, &dir);
    delete_dir_later(&dir);
    0
}

/// Запись в «Приложениях» и ярлык в «Пуске».
fn remove_system_parts(all: bool, _dir: &Path) {
    unsafe {
        RegDeleteKeyW(root(all), wide(UNINSTALL_KEY).as_ptr());
    }
    if let Some(lnk) = shortcut_path(all) {
        let _ = std::fs::remove_file(lnk);
    }
}

/// Удалить папку программы, когда она освободится: командная строка по ПОЛНЫМ путям из System32
/// (не из текущей папки — туда может подложить файл кто угодно), путь передаётся через переменную
/// окружения (без подстановки %…% из самого пути), до минуты повторов, пока exe ещё занят.
fn delete_dir_later(dir: &Path) {
    let Some(sys) = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32")) else { return };
    let cmd = sys.join("cmd.exe");
    let ping = sys.join("PING.EXE");
    use std::os::windows::process::CommandExt;
    let script = format!(
        "/d /c for /l %i in (1,1,60) do @if exist \"%HORA_DIR%\" (rmdir /s /q \"%HORA_DIR%\" 2>nul & if exist \"%HORA_DIR%\" \"{}\" -n 2 127.0.0.1 >nul)",
        ping.display()
    );
    let _ = std::process::Command::new(&cmd)
        .raw_arg(script)
        .env("HORA_DIR", dir)
        .current_dir(&sys)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn();
}

/// Куда ведёт запись автозапуска (путь к exe без кавычек).
fn run_target(name: &str) -> Option<PathBuf> {
    unsafe {
        let mut size: u32 = 0;
        if RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(name).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size) != 0 {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..n]).trim().trim_matches('"').to_string()))
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}
