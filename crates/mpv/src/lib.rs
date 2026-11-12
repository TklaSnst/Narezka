//! Минимальные FFI-объявления к libmpv (client.h + render_gl.h) и обёртка над ними.
//!
//! **Только асинхронно.** Синхронные `mpv_get_property` / `mpv_command` из GL-потока ведут к
//! взаимному ожиданию: ядро ждёт vo, vo ждёт `report_swap` от нас, мы ждём ядро — разрывает только
//! таймаут ~200 мс (шаг 0.5). Поэтому здесь нет синхронных вызовов: состояние — из событий
//! `observe`, команды — `command`/`set` без ожидания ответа.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::Arc;

#[repr(C)]
struct MpvHandle {
    _p: [u8; 0],
}
#[repr(C)]
struct MpvRenderContext {
    _p: [u8; 0],
}
#[repr(C)]
struct RenderParam {
    ty: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void>,
    ctx: *mut c_void,
}
#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}
#[repr(C)]
struct RawEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct RawEventLogMessage {
    prefix: *const c_char,
    level: *const c_char,
    text: *const c_char,
    log_level: c_int,
}
#[repr(C)]
struct RawEventEndFile {
    reason: c_int,
    error: c_int,
}
#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

const PARAM_INVALID: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;
const PARAM_WL_DISPLAY: c_int = 9;
const PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
const UPDATE_FRAME: u64 = 1;
const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_NONE: c_int = 0;
const EVENT_LOG_MESSAGE: c_int = 2;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;
const END_FILE_REASON_ERROR: c_int = 4;

type Callback = Option<unsafe extern "C" fn(*mut c_void)>;

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_initialize(h: *mut MpvHandle) -> c_int;
    fn mpv_terminate_destroy(h: *mut MpvHandle);
    fn mpv_set_option_string(h: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_set_property_async(h: *mut MpvHandle, ud: u64, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_observe_property(h: *mut MpvHandle, ud: u64, name: *const c_char, format: c_int) -> c_int;
    fn mpv_command_async(h: *mut MpvHandle, ud: u64, args: *mut *const c_char) -> c_int;
    fn mpv_request_log_messages(h: *mut MpvHandle, min_level: *const c_char) -> c_int;
    fn mpv_wait_event(h: *mut MpvHandle, timeout: f64) -> *mut RawEvent;
    fn mpv_set_wakeup_callback(h: *mut MpvHandle, cb: Callback, d: *mut c_void);
    fn mpv_error_string(e: c_int) -> *const c_char;
    fn mpv_render_context_create(res: *mut *mut MpvRenderContext, h: *mut MpvHandle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_set_update_callback(ctx: *mut MpvRenderContext, cb: Callback, d: *mut c_void);
    fn mpv_render_context_update(ctx: *mut MpvRenderContext) -> u64;
    fn mpv_render_context_render(ctx: *mut MpvRenderContext, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_report_swap(ctx: *mut MpvRenderContext);
    fn mpv_render_context_free(ctx: *mut MpvRenderContext);

    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{what}: {message}")]
pub struct MpvError {
    pub what: String,
    pub message: String,
}

/// libmpv отказывается работать, если `LC_NUMERIC` не «C» (разбор чисел в опциях).
/// Вызывать в начале `main`, до создания потоков.
pub fn set_c_numeric_locale() {
    const LC_NUMERIC: c_int = 1;
    unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
}

fn cs(s: &str) -> CString {
    // Нулевой байт в пути/опции невозможен в нормальной работе; заменяем, а не паникуем.
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

fn error_string(code: c_int) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)) }.to_string_lossy().into_owned()
}

fn check(code: c_int, what: &str) -> Result<(), MpvError> {
    if code >= 0 { Ok(()) } else { Err(MpvError { what: what.to_owned(), message: error_string(code) }) }
}

unsafe fn str_from(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

/// Адрес GL-функции по имени (от eframe: `CreationContext::get_proc_address`).
pub type GetProcAddress = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;
/// Вызывается из потоков mpv, когда есть события или новый кадр — обычно `request_repaint`.
pub type Wakeup = Arc<dyn Fn() + Send + Sync>;

unsafe extern "C" fn get_proc_trampoline(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let f = unsafe { &*(ctx as *const GetProcAddress) };
    f(unsafe { CStr::from_ptr(name) }) as *mut c_void
}

unsafe extern "C" fn wakeup_trampoline(ctx: *mut c_void) {
    let f = unsafe { &*(ctx as *const Wakeup) };
    f();
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Flag(bool),
    Int(i64),
    Double(f64),
    Str(String),
}

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Flag,
    Int,
    Double,
    Str,
}

#[derive(Debug)]
pub enum Event {
    /// Наблюдаемое свойство изменилось; `None` — сейчас недоступно (например, файл не загружен).
    Property { name: String, value: Option<Prop> },
    FileLoaded,
    /// Файл закрыт; `error` — если открыть или проиграть не удалось.
    EndFile { error: Option<String> },
    /// Воспроизведение возобновилось после seek или загрузки.
    PlaybackRestart,
}

pub struct Mpv {
    h: *mut MpvHandle,
    render: *mut MpvRenderContext,
    // Должны жить, пока жив render context: на них указывают колбэки mpv.
    _proc: Box<GetProcAddress>,
    _wakeup: Box<Wakeup>,
}

impl Mpv {
    /// Создаёт ядро и render context. Вызывать в потоке с текущим GL-контекстом.
    /// `wl_display` — указатель на `wl_display` окна (нужен VAAPI на Wayland), может быть null.
    pub fn new(
        options: &[(&str, &str)],
        get_proc: GetProcAddress,
        wl_display: *mut c_void,
        wakeup: Wakeup,
    ) -> Result<Self, MpvError> {
        unsafe {
            let h = mpv_create();
            if h.is_null() {
                return Err(MpvError { what: "mpv_create".into(), message: "null".into() });
            }
            let init = || -> Result<(), MpvError> {
                for (k, v) in options {
                    check(mpv_set_option_string(h, cs(k).as_ptr(), cs(v).as_ptr()), k)?;
                }
                check(mpv_initialize(h), "mpv_initialize")
            };
            if let Err(e) = init() {
                mpv_terminate_destroy(h);
                return Err(e);
            }
            mpv_request_log_messages(h, c"warn".as_ptr());

            let proc = Box::new(get_proc);
            let mut gl_init = OpenGlInitParams {
                get_proc_address: Some(get_proc_trampoline),
                ctx: &*proc as *const GetProcAddress as *mut c_void,
            };
            let api = c"opengl";
            let mut params = vec![
                RenderParam { ty: PARAM_API_TYPE, data: api.as_ptr() as *mut c_void },
                RenderParam { ty: PARAM_OPENGL_INIT_PARAMS, data: &mut gl_init as *mut _ as *mut c_void },
            ];
            if !wl_display.is_null() {
                params.push(RenderParam { ty: PARAM_WL_DISPLAY, data: wl_display });
            }
            params.push(RenderParam { ty: PARAM_INVALID, data: std::ptr::null_mut() });
            let mut render = std::ptr::null_mut();
            if let Err(e) = check(mpv_render_context_create(&mut render, h, params.as_mut_ptr()), "mpv_render_context_create") {
                mpv_terminate_destroy(h);
                return Err(e);
            }

            let wakeup = Box::new(wakeup);
            let wp = &*wakeup as *const Wakeup as *mut c_void;
            mpv_set_wakeup_callback(h, Some(wakeup_trampoline), wp);
            mpv_render_context_set_update_callback(render, Some(wakeup_trampoline), wp);

            Ok(Self { h, render, _proc: proc, _wakeup: wakeup })
        }
    }

    pub fn command(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let code = unsafe { mpv_command_async(self.h, 0, ptrs.as_mut_ptr()) };
        if code < 0 {
            log::warn!("mpv {args:?}: {}", error_string(code));
        }
    }

    pub fn set(&self, name: &str, value: &str) {
        let v = cs(value);
        let mut p = v.as_ptr();
        let code =
            unsafe { mpv_set_property_async(self.h, 0, cs(name).as_ptr(), FORMAT_STRING, &mut p as *mut _ as *mut c_void) };
        if code < 0 {
            log::warn!("mpv {name}={value}: {}", error_string(code));
        }
    }

    /// Подписка на свойство: значения приходят событиями [`Event::Property`].
    pub fn observe(&self, name: &str, format: Format) {
        let f = match format {
            Format::Flag => FORMAT_FLAG,
            Format::Int => FORMAT_INT64,
            Format::Double => FORMAT_DOUBLE,
            Format::Str => FORMAT_STRING,
        };
        unsafe { mpv_observe_property(self.h, 0, cs(name).as_ptr(), f) };
    }

    /// Следующее интересное событие без ожидания; сообщения mpv уходят в `log`.
    pub fn poll_event(&self) -> Option<Event> {
        loop {
            let e = unsafe { &*mpv_wait_event(self.h, 0.0) };
            match e.event_id {
                EVENT_NONE => return None,
                EVENT_FILE_LOADED => return Some(Event::FileLoaded),
                EVENT_PLAYBACK_RESTART => return Some(Event::PlaybackRestart),
                EVENT_END_FILE if !e.data.is_null() => {
                    let d = unsafe { &*(e.data as *const RawEventEndFile) };
                    let error = (d.reason == END_FILE_REASON_ERROR).then(|| error_string(d.error));
                    return Some(Event::EndFile { error });
                }
                EVENT_PROPERTY_CHANGE if !e.data.is_null() => unsafe {
                    let p = &*(e.data as *const RawEventProperty);
                    let name = str_from(p.name);
                    let value = if p.data.is_null() {
                        None
                    } else {
                        match p.format {
                            FORMAT_FLAG => Some(Prop::Flag(*(p.data as *const c_int) != 0)),
                            FORMAT_INT64 => Some(Prop::Int(*(p.data as *const i64))),
                            FORMAT_DOUBLE => Some(Prop::Double(*(p.data as *const f64))),
                            FORMAT_STRING => Some(Prop::Str(str_from(*(p.data as *const *const c_char)))),
                            _ => None,
                        }
                    };
                    return Some(Event::Property { name, value });
                },
                EVENT_LOG_MESSAGE if !e.data.is_null() => unsafe {
                    let m = &*(e.data as *const RawEventLogMessage);
                    let (prefix, level, text) = (str_from(m.prefix), str_from(m.level), str_from(m.text));
                    let text = text.trim_end();
                    match level.as_str() {
                        "fatal" | "error" => log::error!("mpv/{prefix}: {text}"),
                        _ => log::warn!("mpv/{prefix}: {text}"),
                    }
                },
                _ => {}
            }
        }
    }

    /// Есть ли новый кадр, который надо отрисовать.
    pub fn wants_frame(&self) -> bool {
        unsafe { mpv_render_context_update(self.render) & UPDATE_FRAME != 0 }
    }

    /// Рисует текущий кадр в FBO (без переворота — для своей текстуры).
    /// Вызывать в потоке с текущим GL-контекстом.
    pub fn render(&self, fbo: u32, w: i32, h: i32) {
        let mut target = OpenGlFbo { fbo: fbo as c_int, w, h, internal_format: 0 };
        // FLIP_Y нужен только при рисовании прямо в окно (FBO 0), в текстуру — нет.
        let mut flip: c_int = 0;
        // По умолчанию render() ждёт «времени показа» кадра; если кадр однажды опоздал,
        // поток UI зависает навсегда. В egui — только без блокировки.
        let mut block: c_int = 0;
        let mut params = [
            RenderParam { ty: PARAM_OPENGL_FBO, data: &mut target as *mut _ as *mut c_void },
            RenderParam { ty: PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { ty: PARAM_BLOCK_FOR_TARGET_TIME, data: &mut block as *mut _ as *mut c_void },
            RenderParam { ty: PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        unsafe { mpv_render_context_render(self.render, params.as_mut_ptr()) };
    }

    /// Сообщить mpv, что кадр UI показан. Вызывать в конце каждого кадра UI.
    pub fn report_swap(&self) {
        unsafe { mpv_render_context_report_swap(self.render) };
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // Порядок из документации: сначала render context (при текущем GL-контексте), потом ядро.
        unsafe {
            mpv_render_context_free(self.render);
            mpv_terminate_destroy(self.h);
        }
    }
}
