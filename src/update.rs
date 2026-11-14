// Обновления Hora: проверка последнего выпуска на GitHub, скачивание, проверка подписи, замена программы.
//
// Безопасность: обновление ставится, только если подпись файла (ECDSA P-256 над SHA-256) сходится с
// открытым ключом, встроенным в программу. Закрытый ключ есть только у автора (tools\sign-release.ps1),
// так что даже подменённый на GitHub файл установлен не будет.
//
// Как меняется программа (её файл занят, пока она работает):
//   1) работающая Hora скачивает новую версию в <папка данных>\update и запускает её с
//      --update-coordinator <путь к Hora.exe> <номер процесса>, после чего закрывается;
//   2) новая версия ждёт, пока старая закроется, копирует себя на её место (для установки «для всех»
//      это делает отдельный процесс с правами администратора: --update-apply-admin) и запускает её.

use crate::i18n::tr;
use crate::util::log;
use std::path::{Path, PathBuf};

const API_LATEST: &str = "https://api.github.com/repos/dark-knight-soul/hora/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/dark-knight-soul/hora/releases/download/";
const ASSET: &str = "HoraSetup.exe";

/// Открытый ключ подписи выпусков (BCRYPT_ECCPUBLIC_BLOB: «ECS1», 32, X, Y) — см. tools\new-signing-key.ps1.
const PUBKEY: [u8; 72] = [
    0x45, 0x43, 0x53, 0x31, 0x20, 0x00, 0x00, 0x00, 0x60, 0x3B, 0xA4, 0x79, 0x7A, 0xA1, 0x39, 0xA7, 0x43, 0x48, 0xC4, 0x13, 0xFD, 0xAB, 0x3B, 0x21, 0x2E, 0x1E, 0x15, 0x96, 0xCA, 0xA3, 0x34, 0xA8,
    0x52, 0x32, 0xD3, 0x84, 0xD5, 0xE1, 0x89, 0x81, 0xBF, 0xB4, 0xD3, 0x24, 0x8D, 0xBF, 0x09, 0x17, 0x1C, 0xF9, 0xD2, 0xF3, 0x40, 0x88, 0x12, 0x27, 0xA1, 0x69, 0xC2, 0x32, 0x3F, 0x95, 0xE4, 0x66,
    0xF7, 0x6E, 0x6A, 0xAB, 0x96, 0xC5, 0xF8, 0x27,
];

/// Найденный выпуск новее текущей версии.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub exe_url: String,
    pub sig_url: String,
    pub page: String,
}

/// Текущая версия программы (HORA_PRETEND_VERSION — только для проверки обновления на тестовой копии).
pub fn current() -> String {
    std::env::var("HORA_PRETEND_VERSION").ok().filter(|v| parse_version(v).is_some()).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

fn parse_version(v: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = v.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty() && p.len() <= 4)
}

/// Найденная версия могла устареть (проверка раз в сутки, а окно могли не закрывать днями):
/// перед установкой ещё раз спрашиваем GitHub о самой последней и ставим её — иначе
/// обновление шло бы по ступенькам 0.4 → 0.5 → 0.6. Нет связи — ставим найденную раньше.
/// (Спрашиваем всегда настоящий releases/latest, без тестовой подмены HORA_UPDATE_API.)
pub fn freshest(found: Release) -> Release {
    let latest = crate::http::get_text(API_LATEST).ok().and_then(|json| release_from_json(&json));
    match latest {
        Some(l) if newer(&l.version, &found.version) && newer(&l.version, &current()) => {
            crate::util::log(&format!("обновления: пока ждали, вышла {} — ставлю её вместо {}", l.version, found.version));
            l
        }
        _ => found,
    }
}

/// a новее b?
pub fn newer(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(mut x), Some(mut y)) => {
            let n = x.len().max(y.len());
            x.resize(n, 0);
            y.resize(n, 0);
            x > y
        }
        _ => false,
    }
}

/// Строковые значения ключа в JSON (простой разбор "ключ" : "значение").
fn json_values(json: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(&pat) {
        rest = &rest[i + pat.len()..];
        let r = rest.trim_start();
        let Some(r) = r.strip_prefix(':') else { continue };
        let Some(r) = r.trim_start().strip_prefix('"') else { continue };
        if let Some(end) = r.find('"') {
            out.push(r[..end].replace("\\/", "/"));
        }
    }
    out
}

/// Спросить GitHub о последнем выпуске. Ok(None) — уже последняя версия.
pub fn check() -> Result<Option<Release>, String> {
    // HORA_UPDATE_API — только для проверки обновления на тестовой копии (и только этот репозиторий)
    let api = std::env::var("HORA_UPDATE_API").ok().filter(|u| u.starts_with("https://api.github.com/repos/dark-knight-soul/hora/releases/")).unwrap_or_else(|| API_LATEST.to_string());
    let json = crate::http::get_text(&api)?;
    let version = release_version(&json)?;
    if !newer(&version, &current()) {
        return Ok(None);
    }
    release_files(&json, version).map(Some)
}

/// Версия выпуска из ответа GitHub.
fn release_version(json: &str) -> Result<String, String> {
    let tag = json_values(json, "tag_name").into_iter().next().ok_or_else(|| tr("не удалось прочитать ответ GitHub").to_string())?;
    let version = tag.trim_start_matches('v').to_string();
    if parse_version(&version).is_none() {
        return Err(tr("не удалось прочитать ответ GitHub").to_string());
    }
    Ok(version)
}

/// Ссылки на установщик и подпись — только из выпусков этого репозитория.
fn release_files(json: &str, version: String) -> Result<Release, String> {
    let urls: Vec<String> = json_values(json, "browser_download_url").into_iter().filter(|u| u.starts_with(DOWNLOAD_PREFIX)).collect();
    let exe_url = urls.iter().find(|u| u.ends_with(&format!("/{ASSET}"))).cloned();
    let sig_url = urls.iter().find(|u| u.ends_with(&format!("/{ASSET}.sig"))).cloned();
    match (exe_url, sig_url) {
        (Some(exe_url), Some(sig_url)) => Ok(Release {
            page: format!("https://github.com/dark-knight-soul/hora/releases/tag/v{version}"),
            version,
            exe_url,
            sig_url,
        }),
        _ => Err(tr("в новом выпуске нет подписанного установщика").to_string()),
    }
}

fn release_from_json(json: &str) -> Option<Release> {
    release_files(json, release_version(json).ok()?).ok()
}

/// Подпись файла сходится с открытым ключом выпусков?
pub fn verify(data: &[u8], sig: &[u8]) -> bool {
    use windows_sys::Win32::Security::Cryptography::*;
    if sig.len() != 64 {
        return false;
    }
    unsafe {
        let mut hash = [0u8; 32];
        if BCryptHash(BCRYPT_SHA256_ALG_HANDLE, std::ptr::null(), 0, data.as_ptr(), data.len() as u32, hash.as_mut_ptr(), 32) != 0 {
            return false;
        }
        let mut key: BCRYPT_KEY_HANDLE = std::ptr::null_mut();
        if BCryptImportKeyPair(BCRYPT_ECDSA_P256_ALG_HANDLE, std::ptr::null_mut(), BCRYPT_ECCPUBLIC_BLOB, &mut key, PUBKEY.as_ptr(), PUBKEY.len() as u32, 0) != 0 {
            return false;
        }
        let ok = BCryptVerifySignature(key, std::ptr::null(), hash.as_ptr(), 32, sig.as_ptr(), 64, 0) == 0;
        BCryptDestroyKey(key);
        ok
    }
}

fn update_dir() -> PathBuf {
    crate::util::app_dir().join("update")
}

/// Скачать новую версию и проверить подпись. Возвращает путь к проверенному exe.
pub fn download(r: &Release) -> Result<PathBuf, String> {
    let dir = update_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = dir.join(format!("Hora-{}.exe", r.version));
    let sig = dir.join(format!("Hora-{}.exe.sig", r.version));
    crate::http::download_file(&r.exe_url, &exe, 64 << 20)?;
    crate::http::download_file(&r.sig_url, &sig, 1024)?;
    let ok = match (std::fs::read(&exe), std::fs::read(&sig)) {
        (Ok(d), Ok(s)) => verify(&d, &s),
        _ => false,
    };
    let _ = std::fs::remove_file(&sig);
    if !ok {
        let _ = std::fs::remove_file(&exe);
        log("обновление: подпись не сходится — файл удалён, ничего не установлено");
        return Err(tr("подпись обновления не сходится — установка отменена").to_string());
    }
    Ok(exe)
}

/// Запустить замену программы новой версией (после этого работающую Hora надо закрыть).
pub fn start(new_exe: &Path) -> Result<(), String> {
    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(new_exe)
        .arg("--update-coordinator")
        .arg(&target)
        .arg(std::process::id().to_string())
        .current_dir(new_exe.parent().unwrap_or(Path::new(".")))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Убрать скачанное прошлым обновлением (при запуске уже новой версии).
pub fn cleanup() {
    // свежие файлы (моложе 10 минут) не трогаем: обновление, возможно, ещё идёт —
    // например, ждёт подтверждения прав администратора
    let dir = update_dir();
    let busy = std::fs::read_dir(&dir)
        .map(|it| it.flatten().any(|e| e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() < 600).unwrap_or(true)))
        .unwrap_or(false);
    if !busy {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Скопировать себя на место target (файл может быть ещё занят закрывающейся программой).
fn replace(target: &Path) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut last = String::new();
    for _ in 0..60 {
        match std::fs::copy(&me, target) {
            Ok(_) => {
                let mut ads = target.as_os_str().to_owned();
                ads.push(":Zone.Identifier");
                let _ = std::fs::remove_file(ads);
                return Ok(());
            }
            Err(e) => {
                last = e.to_string();
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
    Err(last)
}

/// Дождаться выхода процесса (не дольше 20 с).
fn wait_exit(pid: u32) {
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 20_000);
            windows_sys::Win32::Foundation::CloseHandle(h);
        }
    }
}

/// Новая версия: --update-coordinator <путь к старой Hora.exe> <номер её процесса>.
pub fn coordinator(target: PathBuf, pid: u32) -> i32 {
    wait_exit(pid);
    // заменять можно только Hora.exe (а не любой файл из командной строки)
    if !target.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("Hora.exe")).unwrap_or(false) {
        return 1;
    }
    let all_users = crate::setup::install_dir(true).map(|d| crate::setup::same_path(&d, target.parent().unwrap_or(Path::new("")))).unwrap_or(false);
    let ok = if all_users {
        // Program Files — только с правами администратора
        match crate::setup::run_elevated("--update-apply-admin") {
            Some(0) => true,
            Some(code) => {
                crate::util::log(&format!("обновление для всех: замена в Program Files не удалась (код {code})"));
                false
            }
            None => {
                crate::util::log("обновление для всех: права администратора не подтвердили — оставлена прежняя версия");
                false
            }
        }
    } else {
        match replace(&target) {
            Ok(()) => {
                if let Some(dir) = target.parent() {
                    if crate::setup::install_dir(false).map(|d| crate::setup::same_path(&d, dir)).unwrap_or(false) {
                        let _ = crate::setup::register_uninstall(false, &target, dir); // новая версия в «Приложениях»
                    }
                }
                true
            }
            Err(_) => false,
        }
    };
    // запускаем (обновлённую или, если не вышло, прежнюю) программу — без прав администратора
    let _ = std::process::Command::new(&target).current_dir(target.parent().unwrap_or(Path::new("."))).spawn();
    if ok {
        0
    } else {
        1
    }
}

/// С правами администратора: заменить Hora.exe в папке установки «для всех» — и только её.
pub fn apply_admin() -> i32 {
    // разные коды — чтобы по журналу помощника было видно, что именно не вышло
    let Some(dir) = crate::setup::install_dir(true) else { return 2 };
    let target = dir.join("Hora.exe");
    if !target.is_file() {
        return 3;
    }
    match replace(&target) {
        Ok(()) => {
            let _ = crate::setup::register_uninstall(true, &target, &dir);
            0
        }
        Err(_) => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("0.4.0", "0.3.0"));
        assert!(newer("v1.0", "0.9.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(!newer("0.3.0", "0.3.0"));
        assert!(!newer("0.3", "0.3.0"));
        assert!(!newer("abc", "0.1.0"));
    }

    #[test]
    fn real_signature() {
        // подписано tools\sign-release.ps1 (.NET ECDsaCng) — программа (BCrypt) должна это принять
        const SIG: [u8; 64] = [0x7E, 0x12, 0x58, 0x44, 0x9A, 0xC2, 0x92, 0x3F, 0x26, 0x53, 0x62, 0x1B, 0xF6, 0x43, 0xE4, 0x70, 0xF3, 0xE4, 0x59, 0x60, 0x0D, 0x18, 0xB8, 0xCA, 0xFA, 0x6F, 0xE4, 0xE3, 0x2B, 0xBE, 0xAC, 0x33, 0xA1, 0x9C, 0xAF, 0xEC, 0x5E, 0x6F, 0x81, 0x14, 0x92, 0x0D, 0xDE, 0x08, 0xE8, 0x0D, 0xA5, 0x0A, 0xE4, 0xF2, 0x05, 0x62, 0x41, 0xCD, 0xE8, 0x33, 0xDA, 0x3E, 0x8D, 0x91, 0x3C, 0xF1, 0xD1, 0xE0];
        assert!(verify(b"hora update test", &SIG));
        assert!(!verify(b"hora update tesT", &SIG), "изменённый файл не проходит");
    }

    #[test]
    fn signature() {
        // подпись чужим ключом или испорченный файл не проходят
        assert!(!verify(b"hello", &[0u8; 64]));
        assert!(!verify(b"hello", &[1u8; 10]));
    }

    #[test]
    fn github_json() {
        let j = r#"{"tag_name":"v0.4.0","assets":[{"browser_download_url":"https://github.com/dark-knight-soul/hora/releases/download/v0.4.0/HoraSetup.exe"},
            {"browser_download_url":"https://evil.example/HoraSetup.exe.sig"}]}"#;
        assert_eq!(json_values(j, "tag_name"), vec!["v0.4.0".to_string()]);
        let urls: Vec<String> = json_values(j, "browser_download_url").into_iter().filter(|u| u.starts_with(DOWNLOAD_PREFIX)).collect();
        assert_eq!(urls.len(), 1, "ссылки не с GitHub-выпусков этого репозитория отбрасываются");
    }
}
