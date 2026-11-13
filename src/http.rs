// HTTP-запросы через встроенный в Windows WinHTTP: без сторонних библиотек,
// с системным прокси и системными сертификатами.
use crate::util::wide;
use std::ffi::c_void;
use std::io::Write;
use std::path::Path;
use windows_sys::Win32::Networking::WinHttp::*;

struct H(*mut c_void);
impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// Открытый запрос: держим все три хэндла, чтобы закрылись в правильном порядке.
struct Request {
    req: H,
    _conn: H,
    _sess: H,
}

fn open(url: &str) -> Result<Request, String> {
    // только защищённое соединение: подменить ответ по дороге нельзя
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(format!("непонятный адрес: {url}"));
    };
    let secure = true;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(if secure { 443 } else { 80 })),
        None => (hostport, if secure { 443u16 } else { 80 }),
    };
    unsafe {
        let sess = H(WinHttpOpen(
            wide("Hora/0.2 (+Windows)").as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ));
        if sess.0.is_null() {
            return Err("WinHttpOpen не сработал".into());
        }
        WinHttpSetTimeouts(sess.0, 10_000, 10_000, 20_000, 30_000);
        let conn = H(WinHttpConnect(sess.0, wide(host).as_ptr(), port, 0));
        if conn.0.is_null() {
            return Err(format!("не удалось подключиться к {host}"));
        }
        let req = H(WinHttpOpenRequest(
            conn.0,
            wide("GET").as_ptr(),
            wide(path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if secure { WINHTTP_FLAG_SECURE } else { 0 },
        ));
        if req.0.is_null() {
            return Err("WinHttpOpenRequest не сработал".into());
        }
        // Вход на корпоративный прокси работает и с уровнем по умолчанию (MEDIUM);
        // уровень LOW не ставим: он отдавал бы учётные данные Windows любому серверу.
        if WinHttpSendRequest(req.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(req.0, std::ptr::null_mut()) == 0
        {
            return Err(format!(
                "нет ответа от {host} (код {})",
                windows_sys::Win32::Foundation::GetLastError()
            ));
        }
        let mut status: u32 = 0;
        let mut len: u32 = 4;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status as *mut u32 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
        );
        if status != 200 {
            return Err(format!("{host} ответил HTTP {status}"));
        }
        Ok(Request { req, _conn: conn, _sess: sess })
    }
}

/// Читает тело ответа кусками и отдаёт каждый кусок в `sink`.
fn read_body(r: &Request, limit: usize, mut sink: impl FnMut(&[u8]) -> Result<(), String>) -> Result<usize, String> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0usize;
    // сервер, отдающий по байту раз в полминуты, не должен держать загрузку вечно
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > std::time::Duration::from_secs(180) {
            return Err("скачивание идёт слишком долго".into());
        }
        let mut got: u32 = 0;
        let ok = unsafe { WinHttpReadData(r.req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut got) };
        if ok == 0 {
            return Err("обрыв при скачивании".into());
        }
        if got == 0 {
            return Ok(total);
        }
        total += got as usize;
        if total > limit {
            return Err("файл слишком большой".into());
        }
        sink(&buf[..got as usize])?;
    }
}

/// Небольшой ответ целиком в память (например, JSON от API).
pub fn get_text(url: &str) -> Result<String, String> {
    let r = open(url)?;
    let mut out = Vec::new();
    read_body(&r, 4 * 1024 * 1024, |c| {
        out.extend_from_slice(c);
        Ok(())
    })?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Скачивает файл сразу на диск, не держа его целиком в памяти.
pub fn download(url: &str, dst: &Path) -> Result<(), String> {
    let r = open(url)?;
    let tmp = dst.with_extension("part");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let res = read_body(&r, 80 * 1024 * 1024, |c| f.write_all(c).map_err(|e| e.to_string()));
    drop(f);
    // это действительно картинка? (JPEG начинается с FF D8 FF, PNG — с 89 50 4E 47)
    let image = || -> bool {
        let mut head = [0u8; 4];
        std::fs::File::open(&tmp).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok()
            && (head[..3] == [0xFF, 0xD8, 0xFF] || head == [0x89, b'P', b'N', b'G'])
    };
    match res {
        Ok(n) if n > 10 * 1024 && image() => std::fs::rename(&tmp, dst).map_err(|e| e.to_string()),
        Ok(n) if n > 10 * 1024 => {
            let _ = std::fs::remove_file(&tmp);
            Err("скачался не JPEG/PNG — пропускаю".into())
        }
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp);
            Err("скачался пустой файл".into())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
