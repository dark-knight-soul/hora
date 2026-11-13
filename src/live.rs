// Живые обои: эффекты (снег, дождь, звёзды, листопад, светлячки) и видео вместо картинки.
//
// Рисует видеокарта, и всё это — в отдельном процессе (WallClock.exe --live), который живёт, только пока
// режим включён. Процесс рисует кадры в общие «поверхности композиции» (по одной на монитор) и передаёт
// главной программе их ключи. Главная программа кладёт поверхность слоем поверх своего окна часов:
// в режиме «эффекты» слой прозрачный — под ним видны обычные обои и часы; в режиме «видео» непрозрачный:
// кадр видео + часы и погода поверх. (Подключиться к чужому окну напрямую Windows не разрешает.)
// Кадры не рисуются, когда рабочего стола не видно (окно на весь экран, заблокированный экран, батарея).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{implement, Interface, BSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, HANDLE, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::*;

use crate::util::{log, Rng};

// ---------------- что показывать (приходит из главного потока) ----------------

#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u8)]
pub enum Effect {
    None = 0,
    Snow,
    Rain,
    Stars,
    Leaves,
    Fireflies,
}

#[derive(Clone, PartialEq)]
pub enum Mode {
    Off,
    Effects(Effect),
    Video(PathBuf),
}

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub density: u32,       // 1 мало, 2 средне, 3 много
    pub span: bool,         // одно видео на все мониторы (как «единые» обои)
    pub pause_battery: bool, // не рисовать от батареи
}

/// Картинка поверх видео (часы или погода): премультиплицированные пиксели и место внутри окна.
#[derive(Clone)]
pub struct Layer {
    pub px: Arc<Vec<u32>>,
    pub rect: (i32, i32, i32, i32), // left, top, right, bottom
    pub opacity: f32,
}

/// Окно часов на одном мониторе.
#[derive(Clone)]
pub struct Face {
    pub hwnd: isize,
    pub rect: (i32, i32, i32, i32), // монитор на экране
    pub layers: Vec<Layer>,         // только в режиме видео
}

struct Shared {
    settings: Settings,
    faces: Vec<Face>,
    ver: u64, // растёт при каждом обновлении — поток узнаёт, что пора перечитать
    stop: bool,
}

// ---------------- отдельный процесс ----------------
// Живые обои рисует вторая копия программы: Hora.exe --live. Главная программа передаёт ей
// настройки и картинки часов через канал (стандартный ввод). Живые обои выключили — процесс
// закрывается, и вся память видеокарты и видеопроигрывателя возвращается системе. Главная программа
// закрылась или упала — канал оборвался, и процесс живых обоев завершается сам.

/// Что процесс живых обоев сообщает главной программе.
enum Note {
    Surface(isize, isize), // окно монитора, ключ поверхности (уже в нашем процессе)
    Drop(isize),           // окно монитора: слой больше не показывать
    Exit,                  // процесс закрылся
}

/// Слой поверх окна часов (в главной программе).
struct Shown {
    face: isize,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual2,
    handle: Option<HANDLE>,
}

/// Управление процессом живых обоев (из главной программы).
pub struct Live {
    notify: isize, // окно главной программы: ему приходит WM_LIVE, когда есть новости от процесса
    child: Option<std::process::Child>,
    tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    notes: Arc<Mutex<Vec<(u64, Note)>>>, // (номер запуска процесса, новость)
    dcomp: Option<IDCompositionDesktopDevice>,
    shown: Vec<Shown>,
    gen: u64,                               // номер текущего запуска процесса
    started: Option<std::time::Instant>,    // когда запустили текущий процесс
    fails: u32,                             // сколько раз подряд процесс закрылся вскоре после запуска
    gave_up: bool,                          // закрывался слишком часто — не перезапускаем до смены режима
    faces: Vec<isize>,                      // наши окна часов: слои от процесса принимаем только для них
}

/// Сообщение главному окну: у процесса живых обоев новости (см. Live::pump).
pub const WM_LIVE: u32 = 0x8000 + 40;

impl Live {
    pub fn new(notify: isize) -> Self {
        Live { notify, child: None, tx: None, notes: Arc::new(Mutex::new(Vec::new())), dcomp: None, shown: Vec::new(), gen: 0, started: None, fails: 0, gave_up: false, faces: Vec::new() }
    }

    /// Разобрать новости процесса: показать новые поверхности поверх окон часов, убрать ненужные.
    /// Возвращает true, если текущий процесс закрылся (тогда его стоит запустить заново).
    pub fn pump(&mut self) -> bool {
        let notes: Vec<(u64, Note)> = match self.notes.lock() {
            Ok(mut n) => std::mem::take(&mut *n),
            Err(_) => return false,
        };
        let mut exited = false;
        for (gen, n) in notes {
            if gen != self.gen {
                // новость от прошлого процесса: его слои уже убраны, ключ просто закрываем
                if let Note::Surface(_, h) = n {
                    unsafe {
                        let _ = windows::Win32::Foundation::CloseHandle(HANDLE(h as *mut _));
                    }
                }
                continue;
            }
            match n {
                Note::Surface(face, h) if !self.faces.contains(&face) => unsafe {
                    // окно не наше — слой не кладём, ключ (он уже в нашем процессе) закрываем
                    let _ = windows::Win32::Foundation::CloseHandle(HANDLE(h as *mut _));
                },
                Note::Surface(face, h) => {
                    if let Err(e) = self.show(face, HANDLE(h as *mut _)) {
                        log(&format!("живые обои: не удалось показать слой: {e}"));
                        unsafe {
                            let _ = windows::Win32::Foundation::CloseHandle(HANDLE(h as *mut _));
                        }
                    }
                }
                Note::Drop(face) => self.hide(face),
                Note::Exit => {
                    self.clear();
                    exited = true;
                }
            }
        }
        exited
    }

    fn show(&mut self, face: isize, h: HANDLE) -> windows::core::Result<()> {
        unsafe {
            if self.dcomp.is_none() {
                // композиция без видеокарты: только раскладка слоёв, рисует другой процесс
                self.dcomp = Some(DCompositionCreateDevice2(None)?);
            }
            let dc = self.dcomp.as_ref().ok_or_else(windows::core::Error::empty)?;
            if !self.shown.iter().any(|s| s.face == face) {
                let target = dc.CreateTargetForHwnd(HWND(face as *mut _), true)?;
                let visual = dc.CreateVisual()?;
                target.SetRoot(&visual)?;
                self.shown.push(Shown { face, _target: target, visual, handle: None });
            }
            let surface = dc.CreateSurfaceFromHandle(h)?;
            let s = self.shown.iter_mut().find(|s| s.face == face).ok_or_else(windows::core::Error::empty)?;
            s.visual.SetContent(&surface)?;
            if let Some(old) = s.handle.replace(h) {
                let _ = windows::Win32::Foundation::CloseHandle(old);
            }
            // с этого места ключ принадлежит слою (закроется в hide) — ошибку Commit только записываем
            if let Err(e) = dc.Commit() {
                log(&format!("живые обои: композиция не обновилась: {e}"));
            }
        }
        Ok(())
    }

    fn hide(&mut self, face: isize) {
        if let Some(i) = self.shown.iter().position(|s| s.face == face) {
            let s = self.shown.remove(i);
            unsafe {
                let _ = s.visual.SetContent(None);
                if let Some(h) = s.handle {
                    let _ = windows::Win32::Foundation::CloseHandle(h);
                }
            }
            drop(s);
            if let Some(dc) = &self.dcomp {
                unsafe {
                    let _ = dc.Commit();
                }
            }
        }
        if self.shown.is_empty() {
            self.dcomp = None;
        }
    }

    /// Убрать все слои (процесс закрылся или режим выключен).
    fn clear(&mut self) {
        let faces: Vec<isize> = self.shown.iter().map(|s| s.face).collect();
        for f in faces {
            self.hide(f);
        }
        self.dcomp = None;
    }

    /// Передать настройки и окна. Процесс запускается, когда есть что показывать, и закрывается,
    /// когда режим выключен.
    pub fn update(&mut self, settings: Settings, faces: Vec<Face>) {
        if settings.mode == Mode::Off || faces.is_empty() {
            self.stop();
            return;
        }
        self.faces = faces.iter().map(|f| f.hwnd).collect();
        // окна часов пересоздали — со старых окон слои убираем
        let gone: Vec<isize> = self.shown.iter().map(|s| s.face).filter(|f| !faces.iter().any(|x| x.hwnd == *f)).collect();
        for f in gone {
            self.hide(f);
        }
        if !self.running() {
            self.spawn();
        }
        if let Some(tx) = &self.tx {
            if tx.send(encode(&settings, &faces)).is_err() {
                self.tx = None;
            }
        }
    }

    /// Закрыть процесс живых обоев: обрываем канал, он сам убирает слои и выходит.
    pub fn stop(&mut self) {
        self.tx = None;
        self.clear();
        if let Some(mut c) = self.child.take() {
            // ждём в стороне, чтобы не держать главную программу; не вышел за 3 с — закрываем силой
            let _ = std::thread::Builder::new().stack_size(64 * 1024).spawn(move || {
                for _ in 0..30 {
                    if !matches!(c.try_wait(), Ok(None)) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = c.kill();
                let _ = c.wait();
            });
        }
        self.gave_up = false;
        self.fails = 0;
        self.started = None;
    }

    /// Процесс живых обоев работает.
    pub fn running(&mut self) -> bool {
        match &mut self.child {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn spawn(&mut self) {
        if self.gave_up {
            return;
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
            // прошлый процесс прожил меньше 5 минут — это падение; три подряд — больше не пробуем
            let short = self.started.map(|t| t.elapsed() < std::time::Duration::from_secs(300)).unwrap_or(false);
            self.fails = if short { self.fails + 1 } else { 0 };
            if self.fails >= 3 {
                self.gave_up = true;
                log("живые обои: процесс несколько раз подряд закрылся с ошибкой — не перезапускаю, пока режим не переключат (подробности выше в журнале)");
                return;
            }
        }
        self.tx = None;
        self.gen += 1;
        self.started = Some(std::time::Instant::now());
        let Ok(exe) = std::env::current_exe() else { return };
        use std::os::windows::process::CommandExt;
        let child = std::process::Command::new(exe)
            .arg("--live")
            .arg(std::process::id().to_string()) // чтобы процесс мог передать нам ключи поверхностей
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                log(&format!("живые обои: не удалось запустить процесс: {e}"));
                return;
            }
        };
        let Some(mut stdin) = child.stdin.take() else { return };
        // читатель: новости процесса (строки SURF/DROP) → очередь, и будим главное окно
        if let Some(out) = child.stdout.take() {
            let (notes, hwnd, gen) = (self.notes.clone(), self.notify, self.gen);
            let _ = std::thread::Builder::new().stack_size(64 * 1024).spawn(move || {
                use std::io::BufRead;
                let push = |n: Note| {
                    if let Ok(mut q) = notes.lock() {
                        q.push((gen, n));
                    }
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd as _, WM_LIVE, 0, 0);
                    }
                };
                let mut rd = std::io::BufReader::new(out);
                loop {
                    // строка новостей короткая; длиннее 256 байт — что-то не так, дальше не читаем
                    let mut line = String::new();
                    use std::io::Read;
                    match (&mut rd).take(256).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if !line.ends_with('\n') => break,
                        Ok(_) => {}
                    }
                    let mut it = line.split_whitespace();
                    let (cmd, a, b) = (it.next(), it.next().and_then(|v| v.parse::<isize>().ok()), it.next().and_then(|v| v.parse::<isize>().ok()));
                    match (cmd, a, b) {
                        (Some("SURF"), Some(f), Some(h)) => push(Note::Surface(f, h)),
                        (Some("DROP"), Some(f), _) => push(Note::Drop(f)),
                        _ => {}
                    }
                }
                push(Note::Exit);
            });
        }
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        // писатель: отправляет только самое свежее сообщение (промежуточные устарели)
        let ok = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                use std::io::Write;
                while let Ok(mut m) = rx.recv() {
                    while let Ok(n) = rx.try_recv() {
                        m = n;
                    }
                    if stdin.write_all(&m).and_then(|_| stdin.flush()).is_err() {
                        break; // процесс закрылся
                    }
                }
            })
            .is_ok();
        if !ok {
            let _ = child.kill();
            return;
        }
        self.child = Some(child);
        self.tx = Some(tx);
    }
}

/// Главная программа (процесс), которой передаём ключи поверхностей.
static PARENT: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

/// Строка новостей для главной программы.
fn tell(line: &str) {
    use std::io::Write;
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(line.as_bytes());
    let _ = o.write_all(b"\n");
    let _ = o.flush();
}

/// Точка входа процесса живых обоев (Hora.exe --live <номер процесса главной программы>).
pub fn child_main(parent_pid: u32) {
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_DUP_HANDLE};
    let h = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, parent_pid) };
    if h.is_null() {
        log("живые обои: нет связи с главной программой");
        return;
    }
    let _ = PARENT.set(h as isize);
    let shared = Arc::new(Mutex::new(Shared {
        settings: Settings { mode: Mode::Off, density: 2, span: false, pause_battery: true },
        faces: Vec::new(),
        ver: 0,
        stop: false,
    }));
    // читатель: сообщения из канала → общие настройки; канал оборвался — главная программа закрылась
    let sh = shared.clone();
    let _ = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
        use std::io::Read;
        let mut inp = std::io::stdin().lock();
        loop {
            let mut len = [0u8; 4];
            if inp.read_exact(&mut len).is_err() {
                break;
            }
            let n = u32::from_le_bytes(len) as usize;
            if n > 256 << 20 {
                break; // не наше сообщение
            }
            let mut buf = vec![0u8; n];
            if inp.read_exact(&mut buf).is_err() {
                break;
            }
            let Some((s, f)) = decode(&buf) else { break };
            if let Ok(mut g) = sh.lock() {
                g.settings = s;
                g.faces = f;
                g.ver += 1;
            }
        }
        if let Ok(mut g) = sh.lock() {
            g.stop = true;
            g.ver += 1;
        }
        // цикл рисования мог зависнуть в драйвере — не оставляем процесс висеть без главной программы
        std::thread::sleep(std::time::Duration::from_secs(5));
        std::process::exit(0);
    });
    run(&shared);
}

// ---- сообщение для процесса живых обоев: длина, затем поля по порядку (little-endian) ----

fn encode(s: &Settings, faces: &[Face]) -> Vec<u8> {
    let mut b: Vec<u8> = vec![0; 4];
    let (mode, effect, path) = match &s.mode {
        Mode::Off => (0u8, Effect::None, String::new()),
        Mode::Effects(e) => (1, *e, String::new()),
        Mode::Video(p) => (2, Effect::None, p.to_string_lossy().into_owned()),
    };
    b.push(mode);
    b.push(effect as u8);
    b.extend_from_slice(&s.density.to_le_bytes());
    b.push(s.span as u8);
    b.push(s.pause_battery as u8);
    b.extend_from_slice(&(path.len() as u32).to_le_bytes());
    b.extend_from_slice(path.as_bytes());
    b.extend_from_slice(&(faces.len() as u32).to_le_bytes());
    let rect = |b: &mut Vec<u8>, r: (i32, i32, i32, i32)| {
        for v in [r.0, r.1, r.2, r.3] {
            b.extend_from_slice(&v.to_le_bytes());
        }
    };
    for f in faces {
        b.extend_from_slice(&(f.hwnd as i64).to_le_bytes());
        rect(&mut b, f.rect);
        b.extend_from_slice(&(f.layers.len() as u32).to_le_bytes());
        for l in &f.layers {
            rect(&mut b, l.rect);
            b.extend_from_slice(&l.opacity.to_le_bytes());
            b.extend_from_slice(&(l.px.len() as u32).to_le_bytes());
            for p in l.px.iter() {
                b.extend_from_slice(&p.to_le_bytes());
            }
        }
    }
    let n = (b.len() - 4) as u32;
    b[..4].copy_from_slice(&n.to_le_bytes());
    b
}

fn decode(b: &[u8]) -> Option<(Settings, Vec<Face>)> {
    let mut pos = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = b.get(pos..pos.checked_add(n)?)?;
        pos += n;
        Some(s)
    };
    let mode = take(1)?[0];
    let effect = match take(1)?[0] {
        1 => Effect::Snow,
        2 => Effect::Rain,
        3 => Effect::Stars,
        4 => Effect::Leaves,
        5 => Effect::Fireflies,
        _ => Effect::None,
    };
    let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap_or([0; 4]));
    let i32_ = |s: &[u8]| i32::from_le_bytes(s.try_into().unwrap_or([0; 4]));
    let density = u32_(take(4)?).clamp(1, 3);
    let span = take(1)?[0] != 0;
    let pause_battery = take(1)?[0] != 0;
    let plen = u32_(take(4)?) as usize;
    let path = String::from_utf8(take(plen)?.to_vec()).ok()?;
    let mode = match mode {
        1 => Mode::Effects(effect),
        2 => Mode::Video(PathBuf::from(path)),
        _ => Mode::Off,
    };
    let nf = u32_(take(4)?) as usize;
    let mut faces = Vec::new();
    for _ in 0..nf.min(16) {
        let hwnd = i64::from_le_bytes(take(8)?.try_into().ok()?) as isize;
        let mut r = [0i32; 4];
        for v in r.iter_mut() {
            *v = i32_(take(4)?);
        }
        let nl = u32_(take(4)?) as usize;
        let mut layers = Vec::new();
        for _ in 0..nl.min(8) {
            let mut lr = [0i32; 4];
            for v in lr.iter_mut() {
                *v = i32_(take(4)?);
            }
            let opacity = f32::from_le_bytes(take(4)?.try_into().ok()?).clamp(0.0, 1.0);
            let n = u32_(take(4)?) as usize;
            let raw = take(n.checked_mul(4)?)?;
            let px: Vec<u32> = raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            layers.push(Layer { px: Arc::new(px), rect: (lr[0], lr[1], lr[2], lr[3]), opacity });
        }
        faces.push(Face { hwnd, rect: (r[0], r[1], r[2], r[3]), layers });
    }
    Some((Settings { mode, density, span, pause_battery }, faces))
}

// ---------------- поток рисования ----------------

fn run(shared: &Arc<Mutex<Shared>>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    // начинаем «во сне»: видеокарту берём, только когда рабочий стол действительно виден
    let mut r = Renderer { dormant: true, ..Default::default() };
    let mut seen = u64::MAX;
    let mut gpu_fail = 0u32;
    let mut errors = 0u32; // ошибок вывода подряд
    loop {
        // свежие настройки
        let (settings, faces, ver, stop) = match shared.lock() {
            Ok(s) => (s.settings.clone(), s.faces.clone(), s.ver, s.stop),
            Err(_) => break,
        };
        if stop {
            break;
        }
        if ver != seen {
            r.rects = faces.iter().map(|f| (f.hwnd, f.rect)).collect();
        }
        // «глубокий сон»: видеокарта отпущена, только следим, не открылся ли рабочий стол
        if r.dormant {
            seen = ver;
            let _ = r.frame(&settings);
            if !r.all_paused {
                r.dormant = false;
                seen = u64::MAX; // проснулись — всё собрать заново (видео продолжит с того же места)
                log("живые обои: рабочий стол снова виден — продолжаю");
            } else {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            continue;
        }
        if r.gpu.is_none() {
            match Gpu::new() {
                Ok(g) => {
                    r.gpu = Some(g);
                    gpu_fail = 0;
                    seen = u64::MAX; // всё пересоздать на новом устройстве
                }
                Err(e) => {
                    gpu_fail += 1;
                    if gpu_fail == 1 {
                        log(&format!("живые обои: видеокарта недоступна ({e}) — пробую снова"));
                    }
                    std::thread::sleep(std::time::Duration::from_secs(if gpu_fail < 5 { 2 } else { 15 }));
                    continue;
                }
            }
        }
        if ver != seen {
            seen = ver;
            if let Err(e) = r.apply(&settings, &faces) {
                log(&format!("живые обои: не удалось подготовить вывод: {e}"));
                r.reset();
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        }
        match r.frame(&settings) {
            Ok(true) => {
                errors = 0;
                // ждём следующего обновления экрана (кадры в такт монитору)
                let t = std::time::Instant::now();
                let hr = unsafe { windows_sys::Win32::Graphics::Dwm::DwmFlush() };
                if hr != 0 || t.elapsed() < std::time::Duration::from_millis(2) {
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            Ok(false) => std::thread::sleep(std::time::Duration::from_millis(250)), // пауза: рабочего стола не видно
            Err(e) => {
                // устройство потеряно (обновили драйвер, сменилась видеокарта) — собираем всё заново,
                // но если ошибка повторяется, ждём всё дольше (не мигаем и не забиваем журнал)
                errors += 1;
                let wait = 1u64 << errors.min(6).saturating_sub(1); // 1, 2, 4, 8, 16, 32 с
                if errors <= 3 || errors % 10 == 0 {
                    log(&format!("живые обои: ошибка вывода ({e}) — пересоздаю через {wait} с"));
                }
                r.reset();
                seen = u64::MAX;
                std::thread::sleep(std::time::Duration::from_secs(wait.min(60)));
            }
        }
    }
    r.reset();
    unsafe { CoUninitialize() };
}

/// Устройства видеокарты: Direct3D (для видео), Direct2D (рисование), DirectComposition (слои окон).
struct Gpu {
    d3d: ID3D11Device,
    d2d: ID2D1DeviceContext,
    media: IDXGIFactoryMedia, // цепочки кадров на общих поверхностях
    brush: ID2D1SolidColorBrush,
}

impl Gpu {
    fn new() -> windows::core::Result<Gpu> {
        unsafe {
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
            let mut dev: Option<ID3D11Device> = None;
            // сначала настоящая видеокарта; если её нет (удалённый сеанс) — программная
            let mut res = D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut dev), None, None);
            if res.is_err() {
                res = D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut dev), None, None);
            }
            res?;
            let d3d = dev.ok_or_else(windows::core::Error::empty)?;
            // видео (Media Foundation) пользуется устройством из своих потоков — включаем защиту
            if let Ok(mt) = d3d.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_dev = factory.CreateDevice(&dxgi)?;
            let d2d = d2d_dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let factory2: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let media: IDXGIFactoryMedia = factory2.cast()?;
            let brush = d2d.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None)?;
            Ok(Gpu { d3d, d2d, media, brush })
        }
    }
}

/// Вывод на один монитор: слой окна, цепочка кадров, картинки часов и частицы.
struct Out {
    hwnd: isize,
    rect: (i32, i32, i32, i32),
    w: u32,
    h: u32,
    surface: HANDLE, // наш ключ общей поверхности
    chain: IDXGISwapChain1,
    canvas: ID2D1Bitmap1,
    layers: Vec<(ID2D1Bitmap1, D2D_RECT_F, f32)>,
    parts: Particles,
    paused: bool,
    cleared: bool, // в эффектах «ничего» уже показан прозрачный кадр
}

impl Drop for Out {
    fn drop(&mut self) {
        // главная программа убирает слой; поверхность закрываем у себя
        tell(&format!("DROP {}", self.hwnd));
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.surface);
        }
    }
}

#[derive(Default)]
struct Renderer {
    gpu: Option<Gpu>,
    outs: Vec<Out>,
    video: Option<Video>,
    last_check: Option<std::time::Instant>,
    last_effect: Option<std::time::Instant>,
    all_paused: bool,
    pause_reason: String,
    started: Option<std::time::Instant>,
    rects: Vec<(isize, (i32, i32, i32, i32))>, // окна мониторов (для проверки, видно ли стол, даже во сне)
    built: Vec<(isize, (i32, i32, i32, i32))>, // для каких окон уже созданы выводы
    locked_n: u32,                    // сколько проверок подряд «нет активного окна»
    paused_since: Option<std::time::Instant>,
    dormant: bool,                   // видеокарта и видео отпущены, пока стол закрыт
    resume: Option<(PathBuf, f64)>,  // с какого места продолжить видео после сна
}

impl Renderer {
    fn reset(&mut self) {
        if let Some(v) = self.video.take() {
            v.shutdown();
        }
        self.drop_gpu();
    }

    /// Отпустить слои окон и видеокарту.
    fn drop_gpu(&mut self) {
        self.outs.clear();
        self.gpu = None;
    }

    /// Рабочего стола давно не видно — отпускаем видео и видеокарту, чтобы не держать память.
    fn sleep(&mut self) {
        if let Some(v) = self.video.take() {
            let pos = unsafe { v.engine.GetCurrentTime() };
            self.resume = Some((v.path.clone(), if pos.is_finite() { pos } else { 0.0 }));
            v.shutdown();
        }
        self.started = None; // время анимации — заново после сна
        self.drop_gpu();
        self.dormant = true;
        crate::util::trim_memory();
        log("живые обои: стол давно закрыт — отпускаю видеокарту до его появления");
    }

    /// Новые настройки или окна: пересоздать выводы и видео, где нужно.
    fn apply(&mut self, s: &Settings, faces: &[Face]) -> windows::core::Result<()> {
        let g = self.gpu.as_ref().ok_or_else(windows::core::Error::empty)?;
        let video = matches!(s.mode, Mode::Video(_));
        // выводы: те же окна и размеры оставляем, остальные пересоздаём (по списку окон, а не по
        // удачно созданным выводам — иначе один неудачный монитор пересоздавал бы всё на каждом сообщении)
        let want: Vec<(isize, (i32, i32, i32, i32))> = faces.iter().map(|f| (f.hwnd, f.rect)).collect();
        if want != self.built || self.outs.is_empty() {
            self.outs.clear();
            for f in faces {
                match make_out(g, f) {
                    Ok(o) => self.outs.push(o),
                    Err(e) => log(&format!("живые обои: не удалось подготовить слой для монитора: {e}")),
                }
            }
            self.built = want;
        }
        // часы и погода поверх видео (вывод ↔ монитор — по окну)
        for o in self.outs.iter_mut() {
            let Some(f) = faces.iter().find(|f| f.hwnd == o.hwnd) else { continue };
            o.layers.clear();
            o.cleared = false;
            if video {
                for l in &f.layers {
                    let (w, h) = ((l.rect.2 - l.rect.0).max(0) as u32, (l.rect.3 - l.rect.1).max(0) as u32);
                    if w == 0 || h == 0 || w > 16384 || h > 16384 || l.px.len() < w as usize * h as usize {
                        continue;
                    }
                    let props = bitmap_props(D2D1_BITMAP_OPTIONS_NONE);
                    let bmp = unsafe { g.d2d.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(l.px.as_ptr() as *const _), w * 4, &props)? };
                    let dst = D2D_RECT_F { left: l.rect.0 as f32, top: l.rect.1 as f32, right: l.rect.2 as f32, bottom: l.rect.3 as f32 };
                    o.layers.push((bmp, dst, l.opacity));
                }
            }
            let effect = match s.mode {
                Mode::Effects(e) => e,
                _ => Effect::None,
            };
            o.parts.setup(effect, s.density, o.w as f32, o.h as f32);
        }
        // видео: открыть новое, закрыть ненужное
        match &s.mode {
            Mode::Video(p) => {
                if self.video.as_ref().map(|v| &v.path != p).unwrap_or(true) {
                    if let Some(v) = self.video.take() {
                        v.shutdown();
                    }
                    match Video::open(g, p) {
                        Ok(v) => {
                            if let Some((rp, pos)) = self.resume.take() {
                                if &rp == p {
                                    v.seek.set(Some(pos));
                                }
                            }
                            v.set_playing(!self.all_paused);
                            self.video = Some(v);
                        }
                        Err(e) => log(&format!("живые обои: видео не открылось ({e}): {}", p.display())),
                    }
                }
            }
            _ => {
                if let Some(v) = self.video.take() {
                    v.shutdown();
                }
            }
        }
        self.last_check = None;
        Ok(())
    }

    /// Один шаг: проверить, видно ли рабочий стол, и нарисовать кадр там, где нужно.
    /// Ok(false) — всё на паузе.
    fn frame(&mut self, s: &Settings) -> windows::core::Result<bool> {
        let now = std::time::Instant::now();
        let started = *self.started.get_or_insert(now);
        // раз в полсекунды: какие мониторы закрыты окнами, не заблокирован ли экран, нет ли батареи
        if self.last_check.map(|t| now - t >= std::time::Duration::from_millis(500)).unwrap_or(true) {
            self.last_check = Some(now);
            // нет активного окна — это блокировка экрана, но мимолётно так бывает и при переключении окон
            self.locked_n = if screen_locked() { self.locked_n + 1 } else { 0 };
            let (locked, battery) = (self.locked_n >= 2, s.pause_battery && on_battery());
            let covered = covered_monitors(&self.rects);
            let why = covered.iter().flatten().next().map(|c| format!("рабочий стол закрыт окном «{c}»")).unwrap_or_default();
            let paused: Vec<bool> = covered.iter().map(|c| locked || battery || c.is_some()).collect();
            for o in self.outs.iter_mut() {
                // по окну монитора находим его состояние
                let i = self.rects.iter().position(|r| r.0 == o.hwnd);
                o.paused = i.map(|i| paused[i]).unwrap_or(true);
            }
            let all = paused.iter().all(|&p| p);
            // в журнал — почему замерли (помогает, если эффекты «не видны»); пишем только при перемене
            let reason = if !all { String::new() } else if locked { "экран заблокирован".to_string() } else if battery { "работа от батареи".to_string() } else { why };
            // пока от главной программы не пришли окна мониторов — писать нечего
            if reason != self.pause_reason && !self.rects.is_empty() {
                log(&if all { format!("живые обои: пауза — {reason}") } else { "живые обои: рисую".to_string() });
                self.pause_reason = reason;
            }
            if all != self.all_paused {
                self.all_paused = all;
                self.paused_since = if all { Some(now) } else { None };
                if let Some(v) = &self.video {
                    v.set_playing(!all);
                }
            }
        }
        if self.dormant {
            return Ok(false);
        }
        if self.all_paused {
            if self.paused_since.map(|t| now - t >= std::time::Duration::from_secs(15)).unwrap_or(false) {
                self.sleep();
            }
            return Ok(false);
        }
        if self.outs.is_empty() {
            return Ok(false);
        }
        let Some(g) = &self.gpu else { return Ok(false) };
        // время для покачиваний и мерцания: по модулю часа, чтобы через дни работы не терялась точность
        let t = ((now - started).as_secs_f64() % 3600.0) as f32;
        match &s.mode {
            Mode::Video(_) => {
                let Some(v) = &self.video else { return Ok(false) };
                if v.failed() {
                    return Ok(false);
                }
                // новый кадр видео есть? (иначе ничего не рисуем — экономим)
                if !v.new_frame() {
                    return Ok(true);
                }
                let (vw, vh) = v.size();
                if vw == 0 || vh == 0 {
                    return Ok(true);
                }
                // общая область: монитор или все мониторы вместе (единые обои)
                let union = if s.span {
                    self.outs.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |a, o| (a.0.min(o.rect.0), a.1.min(o.rect.1), a.2.max(o.rect.2), a.3.max(o.rect.3)))
                } else {
                    (0, 0, 0, 0)
                };
                for o in self.outs.iter_mut().filter(|o| !o.paused) {
                    let area = if s.span { union } else { o.rect };
                    let src = fill_crop(vw, vh, area, o.rect);
                    unsafe {
                        let tex: ID3D11Texture2D = o.chain.GetBuffer(0)?;
                        let dst = RECT { left: 0, top: 0, right: o.w as i32, bottom: o.h as i32 };
                        let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
                        v.engine.TransferVideoFrame(&tex, Some(&src), &dst, Some(&black))?;
                        if !o.layers.is_empty() {
                            g.d2d.SetTarget(&o.canvas);
                            g.d2d.BeginDraw();
                            for (bmp, dst, op) in &o.layers {
                                g.d2d.DrawBitmap(bmp, Some(dst), *op, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
                            }
                            g.d2d.EndDraw(None, None)?;
                        }
                        o.chain.Present(0, DXGI_PRESENT(0)).ok()?;
                    }
                }
            }
            Mode::Effects(_) => {
                // эффекты — 30 кадров в секунду: глазу хватает, а работы вдвое меньше
                let due = self.last_effect.map(|l| now - l >= std::time::Duration::from_millis(31)).unwrap_or(true);
                if !due {
                    return Ok(true);
                }
                let dt = self.last_effect.map(|l| (now - l).as_secs_f32()).unwrap_or(0.0).min(0.1);
                self.last_effect = Some(now);
                for o in self.outs.iter_mut().filter(|o| !o.paused) {
                    let empty = o.parts.is_empty();
                    if empty && o.cleared {
                        continue; // показывать нечего, прозрачный кадр уже стоит
                    }
                    o.parts.step(dt, t);
                    unsafe {
                        g.d2d.SetTarget(&o.canvas);
                        g.d2d.BeginDraw();
                        g.d2d.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
                        o.parts.draw(&g.d2d, &g.brush, t);
                        g.d2d.SetTransform(&Matrix3x2::identity());
                        g.d2d.EndDraw(None, None)?;
                        o.chain.Present(0, DXGI_PRESENT(0)).ok()?;
                    }
                    o.cleared = empty;
                }
            }
            Mode::Off => return Ok(false),
        }
        Ok(true)
    }
}

fn bitmap_props(options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        colorContext: std::mem::ManuallyDrop::new(None),
    }
}

/// Ключ поверхности, который закроется сам, если вывод так и не создан (ошибка на полпути).
struct SurfaceGuard(Option<HANDLE>);

impl SurfaceGuard {
    fn take(mut self) -> HANDLE {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for SurfaceGuard {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
        }
    }
}

/// Вывод на монитор: общая поверхность + цепочка кадров размером с монитор. Ключ поверхности
/// передаём главной программе — она покажет её слоем поверх окна часов.
fn make_out(g: &Gpu, f: &Face) -> windows::core::Result<Out> {
    let (w, h) = ((f.rect.2 - f.rect.0).max(1) as u32, (f.rect.3 - f.rect.1).max(1) as u32);
    unsafe {
        // 3 = чтение и запись (COMPOSITIONOBJECT_ALL_ACCESS)
        let guard = SurfaceGuard(Some(DCompositionCreateSurfaceHandle(3, None)?));
        let surface = guard.0.unwrap_or_default();
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w,
            Height: h,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let chain = g.media.CreateSwapChainForCompositionSurfaceHandle(&g.d3d, surface, &desc, None)?;
        let buf: IDXGISurface = chain.GetBuffer(0)?;
        let props = bitmap_props(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW);
        let canvas = g.d2d.CreateBitmapFromDxgiSurface(&buf, Some(&props))?;
        // первый кадр — прозрачный, чтобы до первого рисования не мелькнул чёрный
        g.d2d.SetTarget(&canvas);
        g.d2d.BeginDraw();
        g.d2d.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        g.d2d.EndDraw(None, None)?;
        chain.Present(0, DXGI_PRESENT(0)).ok()?;
        // ключ — в процесс главной программы и номер ключа ей
        let parent = *PARENT.get().ok_or_else(windows::core::Error::empty)?;
        let mut theirs = HANDLE::default();
        let me = windows::Win32::System::Threading::GetCurrentProcess();
        windows::Win32::Foundation::DuplicateHandle(me, surface, HANDLE(parent as *mut _), &mut theirs, 0, false, windows::Win32::Foundation::DUPLICATE_SAME_ACCESS)?;
        tell(&format!("SURF {} {}", f.hwnd, theirs.0 as isize));
        Ok(Out {
            hwnd: f.hwnd,
            rect: f.rect,
            w,
            h,
            surface: guard.take(),
            chain,
            canvas,
            layers: Vec::new(),
            parts: Particles::default(),
            paused: false,
            cleared: false,
        })
    }
}

/// Какую часть кадра видео взять, чтобы заполнить область (как «заполнение» у обоев),
/// и какой кусок этой области приходится на монитор `part`.
fn fill_crop(vw: u32, vh: u32, area: (i32, i32, i32, i32), part: (i32, i32, i32, i32)) -> MFVideoNormalizedRect {
    let (aw, ah) = ((area.2 - area.0).max(1) as f32, (area.3 - area.1).max(1) as f32);
    let (va, fa) = (vw as f32 / vh as f32, aw / ah);
    // видимая доля кадра по ширине и высоте
    let (fw, fh) = if va > fa { (fa / va, 1.0) } else { (1.0, va / fa) };
    let (x0, y0) = ((1.0 - fw) / 2.0, (1.0 - fh) / 2.0);
    let nx = |x: i32| x0 + fw * ((x - area.0) as f32 / aw);
    let ny = |y: i32| y0 + fh * ((y - area.1) as f32 / ah);
    MFVideoNormalizedRect { left: nx(part.0), top: ny(part.1), right: nx(part.2), bottom: ny(part.3) }
}

// ---------------- видео (Media Foundation) ----------------

#[implement(IMFMediaEngineNotify)]
struct Notify {
    state: Arc<AtomicU32>, // 0 — грузится, 1 — готово, 2 — ошибка
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 as u32 || event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 as u32 {
            let _ = self.state.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
        } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.state.store(2, Ordering::SeqCst);
        }
        Ok(())
    }
}

struct Video {
    path: PathBuf,
    engine: IMFMediaEngine,
    state: Arc<AtomicU32>,
    _manager: IMFDXGIDeviceManager,
    reported: std::cell::Cell<bool>,
    seek: std::cell::Cell<Option<f64>>, // перемотать, как только видео загрузится
}

impl Video {
    fn open(g: &Gpu, path: &Path) -> windows::core::Result<Video> {
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_LITE)?;
            let r = Self::open_started(g, path);
            if r.is_err() {
                let _ = MFShutdown(); // парный к MFStartup
            }
            r
        }
    }

    /// Открыть видео (Media Foundation уже запущен).
    unsafe fn open_started(g: &Gpu, path: &Path) -> windows::core::Result<Video> {
        {
            let mut token = 0u32;
            let mut mgr: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut token, &mut mgr)?;
            let mgr = mgr.ok_or_else(windows::core::Error::empty)?;
            mgr.ResetDevice(&g.d3d, token)?;
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 3)?;
            let attrs = attrs.ok_or_else(windows::core::Error::empty)?;
            let state = Arc::new(AtomicU32::new(0));
            let notify: IMFMediaEngineNotify = Notify { state: state.clone() }.into();
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &mgr)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attrs.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
            let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let engine = factory.CreateInstance(0, &attrs)?;
            let setup = (|| -> windows::core::Result<()> {
                engine.SetMuted(true)?; // обои — без звука
                engine.SetLoop(true)?;
                engine.SetSource(&BSTR::from(path.to_string_lossy().as_ref()))?;
                engine.Play()
            })();
            if let Err(e) = setup {
                let _ = engine.Shutdown();
                return Err(e);
            }
            log(&format!("живые обои: видео {}", path.display()));
            Ok(Video { path: path.to_path_buf(), engine, state, _manager: mgr, reported: std::cell::Cell::new(false), seek: std::cell::Cell::new(None) })
        }
    }

    fn failed(&self) -> bool {
        let bad = self.state.load(Ordering::SeqCst) == 2;
        if bad && !self.reported.replace(true) {
            let code = unsafe { self.engine.GetError().map(|e| e.GetErrorCode()).unwrap_or(0) };
            log(&format!("живые обои: видео не воспроизводится (код {code}) — возможно, нужен кодек: {}", self.path.display()));
        }
        bad
    }

    /// Готов новый кадр? (S_OK — да, S_FALSE — тот же, что был)
    fn new_frame(&self) -> bool {
        if self.state.load(Ordering::SeqCst) != 1 {
            return false;
        }
        if let Some(pos) = self.seek.take() {
            unsafe {
                let _ = self.engine.SetCurrentTime(pos);
            }
        }
        let mut pts = 0i64;
        let hr = unsafe { (Interface::vtable(&self.engine).OnVideoStreamTick)(Interface::as_raw(&self.engine), &mut pts) };
        hr.0 == 0
    }

    fn size(&self) -> (u32, u32) {
        let (mut w, mut h) = (0u32, 0u32);
        unsafe {
            let _ = self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
        }
        (w, h)
    }

    fn set_playing(&self, on: bool) {
        unsafe {
            let _ = if on { self.engine.Play() } else { self.engine.Pause() };
        }
    }

    fn shutdown(self) {
        unsafe {
            let _ = self.engine.Shutdown();
            let _ = MFShutdown();
        }
    }
}

// ---------------- частицы ----------------

#[derive(Clone, Copy, Default)]
struct P {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    r: f32,
    a: f32,
    ph: f32, // фаза (покачивание, мерцание, поворот)
    f: f32,  // частота
    c: u8,   // номер цвета
}

#[derive(Default)]
struct Particles {
    kind: Option<Effect>,
    w: f32,
    h: f32,
    s: f32, // масштаб от высоты экрана (1.0 = 1080 px)
    list: Vec<P>,
    rng: Option<Rng>,
    shoot: Option<(f32, f32, f32, f32, f32)>, // падающая звезда: x, y, vx, vy, оставшееся время
    next_shoot: f32,
}

impl Particles {
    fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn rnd(&mut self, a: f32, b: f32) -> f32 {
        let r = self.rng.get_or_insert_with(Rng::new);
        a + (b - a) * (r.below(10_000) as f32 / 10_000.0)
    }

    fn setup(&mut self, kind: Effect, density: u32, w: f32, h: f32) {
        if self.kind == Some(kind) && self.w == w && self.h == h && self.list.len() == Self::count(kind, density, w, h) {
            return;
        }
        self.kind = Some(kind);
        self.w = w;
        self.h = h;
        self.s = (h / 1080.0).max(0.5);
        self.list.clear();
        self.shoot = None;
        self.next_shoot = 4.0;
        for _ in 0..Self::count(kind, density, w, h) {
            let p = self.spawn(true);
            self.list.push(p);
        }
    }

    /// Сколько частиц: от вида эффекта, «мало/средне/много» и площади экрана.
    fn count(kind: Effect, density: u32, w: f32, h: f32) -> usize {
        let base = match kind {
            Effect::None => 0.0,
            Effect::Snow => 110.0,
            Effect::Rain => 150.0,
            Effect::Stars => 110.0,
            Effect::Leaves => 22.0,
            Effect::Fireflies => 32.0,
        };
        let mul = [0.5, 1.0, 1.9][(density.clamp(1, 3) - 1) as usize];
        (base * mul * (w * h / (1920.0 * 1080.0)).sqrt()).round() as usize
    }

    /// Новая частица; `anywhere` — в случайном месте экрана (при запуске), иначе — над экраном.
    fn spawn(&mut self, anywhere: bool) -> P {
        let (w, h, s) = (self.w, self.h, self.s);
        let x = self.rnd(0.0, w);
        let top = if anywhere { self.rnd(0.0, h) } else { self.rnd(-0.1 * h, -0.01 * h) };
        match self.kind.unwrap_or(Effect::None) {
            Effect::Snow => {
                let r = self.rnd(1.3, 4.2) * s;
                P { x, y: top, vx: 0.0, vy: (18.0 + r / s * 13.0) * s, r, a: self.rnd(0.45, 0.9), ph: self.rnd(0.0, 6.28), f: self.rnd(0.3, 0.9), c: 0 }
            }
            Effect::Rain => {
                let vy = self.rnd(950.0, 1400.0) * s;
                P { x: x - 0.12 * h, y: top, vx: vy * 0.12, vy, r: self.rnd(16.0, 32.0) * s, a: self.rnd(0.18, 0.4), ph: 0.0, f: 0.0, c: 0 }
            }
            Effect::Stars => P {
                x,
                y: self.rnd(0.0, h),
                vx: 0.0,
                vy: 0.0,
                r: self.rnd(0.6, 1.9) * s,
                a: self.rnd(0.35, 0.95),
                ph: self.rnd(0.0, 6.28),
                f: self.rnd(0.4, 1.6),
                c: 0,
            },
            Effect::Leaves => P {
                x,
                y: top,
                vx: self.rnd(-15.0, 25.0) * s,
                vy: self.rnd(45.0, 95.0) * s,
                r: self.rnd(6.0, 10.0) * s,
                a: self.rnd(0.75, 0.95),
                ph: self.rnd(0.0, 6.28),
                f: self.rnd(0.4, 1.1),
                c: self.rnd(0.0, 4.0) as u8,
            },
            Effect::Fireflies => P {
                x,
                y: self.rnd(0.25 * h, h),
                vx: self.rnd(-18.0, 18.0) * s,
                vy: self.rnd(-12.0, 12.0) * s,
                r: self.rnd(1.6, 2.8) * s,
                a: 1.0,
                ph: self.rnd(0.0, 6.28),
                f: self.rnd(0.3, 0.8),
                c: 0,
            },
            Effect::None => P::default(),
        }
    }

    fn step(&mut self, dt: f32, t: f32) {
        let (w, h) = (self.w, self.h);
        let kind = self.kind.unwrap_or(Effect::None);
        for i in 0..self.list.len() {
            let mut p = self.list[i];
            match kind {
                Effect::Snow => {
                    p.y += p.vy * dt;
                    p.x += (p.ph + t * p.f).sin() * 18.0 * self.s * dt;
                }
                Effect::Rain | Effect::Leaves => {
                    p.y += p.vy * dt;
                    p.x += p.vx * dt + if kind == Effect::Leaves { (p.ph + t * p.f).sin() * 40.0 * self.s * dt } else { 0.0 };
                }
                Effect::Fireflies => {
                    // медленно блуждают и плавно меняют направление
                    let turn = (p.ph + t * 0.37).sin() * 0.9;
                    let (c, s) = (turn.cos(), turn.sin());
                    p.x += (p.vx * c - p.vy * s) * dt;
                    p.y += (p.vx * s + p.vy * c) * dt;
                    if p.x < -20.0 || p.x > w + 20.0 || p.y < 0.15 * h || p.y > h + 20.0 {
                        p.vx = -p.vx;
                        p.vy = -p.vy;
                    }
                }
                _ => {}
            }
            // улетела за экран — новая сверху
            if matches!(kind, Effect::Snow | Effect::Rain | Effect::Leaves) && (p.y > h + 40.0 || p.x > w + 0.2 * h || p.x < -0.2 * h) {
                p = self.spawn(false);
            }
            self.list[i] = p;
        }
        // звёзды: иногда падающая звезда
        if kind == Effect::Stars {
            if let Some(sh) = &mut self.shoot {
                sh.0 += sh.2 * dt;
                sh.1 += sh.3 * dt;
                sh.4 -= dt;
                if sh.4 <= 0.0 {
                    self.shoot = None;
                }
            } else {
                self.next_shoot -= dt;
                if self.next_shoot <= 0.0 {
                    let (x, y) = (self.rnd(0.1 * w, 0.8 * w), self.rnd(0.03 * h, 0.35 * h));
                    let v = self.rnd(700.0, 1000.0) * self.s;
                    self.shoot = Some((x, y, v * 0.85, v * 0.45, 0.7));
                    self.next_shoot = self.rnd(6.0, 14.0);
                }
            }
        }
    }

    fn draw(&self, dc: &ID2D1DeviceContext, brush: &ID2D1SolidColorBrush, t: f32) {
        let color = |r: f32, g: f32, b: f32, a: f32| D2D1_COLOR_F { r, g, b, a };
        let ell = |x: f32, y: f32, rx: f32, ry: f32| D2D1_ELLIPSE { point: D2D_POINT_2F { x, y }, radiusX: rx, radiusY: ry };
        unsafe {
            match self.kind.unwrap_or(Effect::None) {
                Effect::Snow => {
                    for p in &self.list {
                        brush.SetColor(&color(1.0, 1.0, 1.0, p.a));
                        dc.FillEllipse(&ell(p.x, p.y, p.r, p.r), brush);
                    }
                }
                Effect::Rain => {
                    for p in &self.list {
                        brush.SetColor(&color(0.78, 0.85, 0.95, p.a));
                        let k = p.r / p.vy.max(1.0);
                        dc.DrawLine(D2D_POINT_2F { x: p.x, y: p.y }, D2D_POINT_2F { x: p.x - p.vx * k, y: p.y - p.vy * k }, brush, 1.3 * self.s, None);
                    }
                }
                Effect::Stars => {
                    for p in &self.list {
                        let tw = 0.55 + 0.45 * (p.ph + t * p.f * 2.0).sin();
                        brush.SetColor(&color(1.0, 0.97, 0.9, p.a * tw));
                        dc.FillEllipse(&ell(p.x, p.y, p.r, p.r), brush);
                    }
                    if let Some((x, y, vx, vy, left)) = self.shoot {
                        // хвост из нескольких отрезков, к концу тоньше и прозрачнее
                        let len = 0.12;
                        for k in 0..6 {
                            let (a0, a1) = (k as f32 / 6.0, (k + 1) as f32 / 6.0);
                            let fade = (1.0 - a0) * (left / 0.7).clamp(0.0, 1.0);
                            brush.SetColor(&color(1.0, 1.0, 1.0, 0.85 * fade));
                            dc.DrawLine(
                                D2D_POINT_2F { x: x - vx * len * a0, y: y - vy * len * a0 },
                                D2D_POINT_2F { x: x - vx * len * a1, y: y - vy * len * a1 },
                                brush,
                                (2.2 - a0 * 1.6) * self.s,
                                None,
                            );
                        }
                    }
                }
                Effect::Leaves => {
                    const PAL: [(f32, f32, f32); 4] = [(0.86, 0.42, 0.12), (0.78, 0.24, 0.12), (0.93, 0.68, 0.18), (0.62, 0.45, 0.16)];
                    for p in &self.list {
                        let (r, g, b) = PAL[(p.c as usize) % 4];
                        brush.SetColor(&color(r, g, b, p.a));
                        // лист кружится: поворот вокруг центра
                        let ang = p.ph + t * p.f * 2.2;
                        let (c, s) = (ang.cos(), ang.sin());
                        let m = Matrix3x2 { M11: c, M12: s, M21: -s, M22: c, M31: p.x - c * p.x + s * p.y, M32: p.y - s * p.x - c * p.y };
                        dc.SetTransform(&m);
                        dc.FillEllipse(&ell(p.x, p.y, p.r, p.r * 0.45 * (0.35 + 0.65 * (t * p.f * 3.0 + p.ph).cos().abs())), brush);
                    }
                    dc.SetTransform(&Matrix3x2::identity());
                }
                Effect::Fireflies => {
                    for p in &self.list {
                        let blink = (0.5 + 0.5 * (p.ph + t * p.f * 3.0).sin()).powf(2.0);
                        // мягкое свечение вокруг и яркая точка
                        brush.SetColor(&color(0.85, 1.0, 0.45, 0.10 * blink));
                        dc.FillEllipse(&ell(p.x, p.y, p.r * 5.0, p.r * 5.0), brush);
                        brush.SetColor(&color(0.85, 1.0, 0.45, 0.22 * blink));
                        dc.FillEllipse(&ell(p.x, p.y, p.r * 2.4, p.r * 2.4), brush);
                        brush.SetColor(&color(1.0, 1.0, 0.75, 0.95 * blink));
                        dc.FillEllipse(&ell(p.x, p.y, p.r, p.r), brush);
                    }
                }
                Effect::None => {}
            }
        }
    }
}

// ---------------- видно ли рабочий стол ----------------

/// Экран заблокирован или открыт защищённый рабочий стол (UAC) — у нас нет активного окна.
fn screen_locked() -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow().is_null() }
}

fn on_battery() -> bool {
    unsafe {
        let mut s: windows_sys::Win32::System::Power::SYSTEM_POWER_STATUS = std::mem::zeroed();
        windows_sys::Win32::System::Power::GetSystemPowerStatus(&mut s) != 0 && s.ACLineStatus == 0
    }
}

/// Для каждого монитора: закрыт ли он почти целиком каким-то обычным окном
/// (развёрнутым, на весь экран, игрой). Тогда рисовать там кадры незачем. Возвращает класс окна.
fn covered_monitors(mons: &[(isize, (i32, i32, i32, i32))]) -> Vec<Option<String>> {
    use windows_sys::Win32::Foundation::{BOOL as SBOOL, HWND as SHWND, LPARAM, RECT as SRECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    struct Ctx<'a> {
        mons: &'a [(isize, (i32, i32, i32, i32))],
        out: Vec<Option<String>>,
    }
    unsafe extern "system" fn each(h: SHWND, lp: LPARAM) -> SBOOL {
        let c = &mut *(lp as *mut Ctx);
        if IsWindowVisible(h) == 0 || IsIconic(h) != 0 {
            return 1;
        }
        // невидимые «призрачные» окна Windows (спрятанные приложения из магазина) пропускаем
        let mut cloaked = 0u32;
        windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute(h, 14, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return 1;
        }
        // прозрачные для мыши накладки (оверлеи игр, записи экрана) не закрывают стол
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TRANSPARENT != 0 || (ex & WS_EX_TOOLWINDOW != 0 && ex & WS_EX_LAYERED != 0) {
            return 1;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(h, cls.as_mut_ptr(), 64).max(0) as usize;
        let name = String::from_utf16_lossy(&cls[..n]);
        if matches!(name.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return 1;
        }
        let mut r: SRECT = std::mem::zeroed();
        if GetWindowRect(h, &mut r) == 0 {
            return 1;
        }
        for (i, (_, m)) in c.mons.iter().enumerate() {
            let iw = (r.right.min(m.2) - r.left.max(m.0)).max(0) as i64;
            let ih = (r.bottom.min(m.3) - r.top.max(m.1)).max(0) as i64;
            let area = ((m.2 - m.0) as i64 * (m.3 - m.1) as i64).max(1);
            // 85% монитора — с запасом на панель задач у развёрнутых окон
            if iw * ih * 100 >= area * 85 && c.out[i].is_none() {
                c.out[i] = Some(name.clone());
            }
        }
        1
    }
    let mut c = Ctx { mons, out: vec![None; mons.len()] };
    unsafe {
        EnumWindows(Some(each), &mut c as *mut Ctx as LPARAM);
    }
    c.out
}

// ---------------- список видео: своя папка и Wallpaper Engine ----------------

#[derive(Clone, PartialEq)]
pub struct VideoItem {
    pub title: String,
    pub path: PathBuf,
    pub we: bool, // из Wallpaper Engine
}

const VIDEO_EXT: [&str; 7] = ["mp4", "webm", "mkv", "mov", "avi", "wmv", "m4v"];

pub fn is_video(p: &Path) -> bool {
    p.extension().map(|e| VIDEO_EXT.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

/// Все видео: из своей папки и (по желанию) обои Wallpaper Engine типа «видео».
pub fn scan(folder: &Path, own: bool, we: bool) -> Vec<VideoItem> {
    let mut v = Vec::new();
    if own {
        if let Ok(rd) = std::fs::read_dir(folder) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() && is_video(&p) {
                    let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    v.push(VideoItem { title, path: p, we: false });
                }
            }
        }
    }
    if we {
        for dir in we_dirs() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                if let Some(item) = we_item(&e.path()) {
                    v.push(item);
                }
            }
        }
    }
    v.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    v.dedup_by(|a, b| a.path == b.path);
    v
}

/// Папки, где Wallpaper Engine хранит обои: подписки из мастерской Steam (431960) и свои проекты.
fn we_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for lib in steam_libraries() {
        out.push(lib.join("steamapps").join("workshop").join("content").join("431960"));
        let we = lib.join("steamapps").join("common").join("wallpaper_engine").join("projects");
        out.push(we.join("myprojects"));
        out.push(we.join("defaultprojects"));
    }
    out.retain(|p| p.is_dir());
    out
}

/// Библиотеки Steam: основная папка (из реестра) и дополнительные (из libraryfolders.vdf).
fn steam_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    let Some(steam) = reg_string("Software\\Valve\\Steam", "SteamPath") else { return libs };
    let steam = PathBuf::from(steam.replace('/', "\\"));
    libs.push(steam.clone());
    if let Ok(t) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for line in t.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("\"path\"") {
                let val = rest.trim().trim_matches('"').replace("\\\\", "\\");
                let p = PathBuf::from(val);
                if !libs.contains(&p) {
                    libs.push(p);
                }
            }
        }
    }
    libs
}

fn reg_string(key: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::*;
    unsafe {
        let (k, n) = (crate::util::wide(key), crate::util::wide(name));
        let mut size: u32 = 0;
        if RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize / 2) + 1];
        if RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size) != 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// Обои Wallpaper Engine из папки проекта — только тип «видео» (сцены и веб-страницы
/// показывает лишь сам Wallpaper Engine).
fn we_item(dir: &Path) -> Option<VideoItem> {
    let pj = dir.join("project.json");
    if std::fs::metadata(&pj).ok()?.len() > 1 << 20 {
        return None;
    }
    let json = std::fs::read_to_string(pj).ok()?;
    let types = json_values(&json, "type");
    if !types.iter().any(|t| t.eq_ignore_ascii_case("video")) {
        return None;
    }
    let file = json_values(&json, "file").into_iter().next()?;
    // имя файла — только внутри папки проекта (без «..» и полных путей)
    let rel = Path::new(&file);
    if rel.is_absolute() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    let path = dir.join(rel);
    if !path.is_file() || !is_video(&path) {
        return None;
    }
    let title = json_values(&json, "title").into_iter().next().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Some(VideoItem { title: title.chars().filter(|c| !c.is_control()).take(80).collect(), path, we: true })
}

/// Все строковые значения ключа в JSON (простой разбор: "ключ" : "значение").
fn json_values(json: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\"");
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(&pat) {
        rest = &rest[i + pat.len()..];
        let r = rest.trim_start();
        let Some(r) = r.strip_prefix(':') else { continue };
        let r = r.trim_start();
        let Some(r) = r.strip_prefix('"') else { continue };
        let mut s = String::new();
        let mut chars = r.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => s.push(' '),
                    Some('t') => s.push(' '),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            s.push(ch);
                        }
                    }
                    Some(o) => s.push(o),
                    None => break,
                },
                _ => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json() {
        let j = r#"{ "file" : "clip.mp4", "general" : { "properties" : { "c" : { "type" : "color", "text" : "Цвет" } } },
            "title" : "Ночной \"город\" Д", "type" : "Video" }"#;
        assert_eq!(json_values(j, "type"), vec!["color".to_string(), "Video".to_string()]);
        assert_eq!(json_values(j, "title"), vec!["Ночной \"город\" Д".to_string()]);
        assert_eq!(json_values(j, "file"), vec!["clip.mp4".to_string()]);
    }

    #[test]
    fn we_project() {
        let base = std::env::temp_dir().join(format!("wc-we-test-{}", std::process::id()));
        let ok = base.join("1");
        let scene = base.join("2");
        let evil = base.join("3");
        for d in [&ok, &scene, &evil] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(ok.join("v.mp4"), b"x").unwrap();
        std::fs::write(ok.join("project.json"), r#"{"file":"v.mp4","title":"Море","type":"video"}"#).unwrap();
        std::fs::write(scene.join("project.json"), r#"{"file":"scene.json","title":"Сцена","type":"scene"}"#).unwrap();
        std::fs::write(evil.join("project.json"), r#"{"file":"..\1\v.mp4","title":"Хитрый","type":"video"}"#).unwrap();
        let a = we_item(&ok).expect("видео-обои должны найтись");
        assert_eq!((a.title.as_str(), a.we), ("Море", true));
        assert!(we_item(&scene).is_none(), "сцены не поддерживаем");
        assert!(we_item(&evil).is_none(), "путь за пределы папки проекта запрещён");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn message() {
        let s = Settings { mode: Mode::Video(PathBuf::from("C:\\Видео\\море.mp4")), density: 3, span: true, pause_battery: false };
        let f = vec![Face { hwnd: 0x1234, rect: (-1920, 0, 0, 1080), layers: vec![Layer { px: Arc::new(vec![0xFF00_00FF, 7]), rect: (10, 20, 12, 21), opacity: 0.5 }] }];
        let b = encode(&s, &f);
        assert_eq!(u32::from_le_bytes(b[..4].try_into().unwrap()) as usize, b.len() - 4);
        let (s2, f2) = decode(&b[4..]).unwrap();
        assert!(s2 == s);
        assert_eq!((f2[0].hwnd, f2[0].rect, f2[0].layers[0].rect, f2[0].layers[0].opacity), (0x1234, (-1920, 0, 0, 1080), (10, 20, 12, 21), 0.5));
        assert_eq!(*f2[0].layers[0].px, vec![0xFF00_00FF, 7]);
        // обрезанное сообщение не разбирается
        assert!(decode(&b[4..b.len() - 3]).is_none());
        let fx = Settings { mode: Mode::Effects(Effect::Fireflies), density: 1, span: false, pause_battery: true };
        assert!(decode(&encode(&fx, &[])[4..]).unwrap().0 == fx);
    }

    #[test]
    fn crop() {
        // видео 16:9 на экране 16:10 — обрезаются края по ширине
        let r = fill_crop(1920, 1080, (0, 0, 1920, 1200), (0, 0, 1920, 1200));
        assert!((r.top - 0.0).abs() < 1e-4 && (r.bottom - 1.0).abs() < 1e-4);
        // видно 1.6/1.778 = 90% ширины — по 5% с каждого края
        assert!((r.left - 0.05).abs() < 1e-3 && (r.right - 0.95).abs() < 1e-3);
        // два монитора рядом, видео на оба: левый получает левую половину кадра
        let l = fill_crop(3840, 1080, (0, 0, 3840, 1080), (0, 0, 1920, 1080));
        assert!((l.left - 0.0).abs() < 1e-4 && (l.right - 0.5).abs() < 1e-4);
    }
}
