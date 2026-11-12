//! Минимальные FFI-биндинги к libmpv (client.h + render_gl.h) и обёртка над ними.
//! Для спайка — руками, в приложении будет `-sys` крейт.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::Arc;

#[repr(C)]
pub struct MpvHandle {
    _p: [u8; 0],
}
#[repr(C)]
pub struct MpvRenderContext {
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

pub const EVENT_NONE: c_int = 0;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_VIDEO_RECONFIG: c_int = 17;
pub const EVENT_PLAYBACK_RESTART: c_int = 21;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

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
    fn mpv_wait_event(h: *mut MpvHandle, timeout: f64) -> *mut RawEvent;
    fn mpv_set_wakeup_callback(h: *mut MpvHandle, cb: Callback, d: *mut c_void);
    fn mpv_error_string(e: c_int) -> *const c_char;
    fn mpv_render_context_create(res: *mut *mut MpvRenderContext, h: *mut MpvHandle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_set_update_callback(ctx: *mut MpvRenderContext, cb: Callback, d: *mut c_void);
    fn mpv_render_context_update(ctx: *mut MpvRenderContext) -> u64;
    fn mpv_render_context_render(ctx: *mut MpvRenderContext, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_report_swap(ctx: *mut MpvRenderContext);
    fn mpv_render_context_free(ctx: *mut MpvRenderContext);
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn check(code: c_int, what: &str) -> Result<(), String> {
    if code >= 0 {
        Ok(())
    } else {
        let msg = unsafe { CStr::from_ptr(mpv_error_string(code)) }.to_string_lossy();
        Err(format!("{what}: {msg}"))
    }
}

pub type GetProcAddress = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;

unsafe extern "C" fn get_proc_trampoline(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let f = unsafe { &*(ctx as *const GetProcAddress) };
    f(unsafe { CStr::from_ptr(name) }) as *mut c_void
}

unsafe extern "C" fn repaint_trampoline(ctx: *mut c_void) {
    let egui_ctx = unsafe { &*(ctx as *const eframe::egui::Context) };
    egui_ctx.request_repaint();
}

pub struct Mpv {
    h: *mut MpvHandle,
    render: *mut MpvRenderContext,
    // Должны жить, пока жив render context: на них указывают колбэки mpv.
    _proc: Box<GetProcAddress>,
    _egui_wakeup: Box<eframe::egui::Context>,
    _egui_update: Box<eframe::egui::Context>,
}

#[derive(Clone, Debug)]
pub enum Prop {
    Flag(bool),
    Int(i64),
    Double(f64),
    Str(String),
}

pub enum Format {
    Flag,
    Int,
    Double,
    Str,
}

pub struct Event {
    pub id: c_int,
    #[allow(dead_code)]
    pub error: c_int,
    /// Для PROPERTY_CHANGE: имя и значение (None — свойство сейчас недоступно).
    pub prop: Option<(String, Option<Prop>)>,
}

impl Mpv {
    /// `wl_display` — указатель на wl_display окна (для VAAPI-интеропа на Wayland), может быть null.
    pub fn new(
        options: &[(&str, &str)],
        get_proc: GetProcAddress,
        wl_display: *mut c_void,
        egui_ctx: &eframe::egui::Context,
    ) -> Result<Self, String> {
        unsafe {
            let h = mpv_create();
            if h.is_null() {
                return Err("mpv_create вернул null".into());
            }
            for (k, v) in options {
                check(mpv_set_option_string(h, cs(k).as_ptr(), cs(v).as_ptr()), k)?;
            }
            check(mpv_initialize(h), "mpv_initialize")?;

            let proc = Box::new(get_proc);
            let mut init = OpenGlInitParams {
                get_proc_address: Some(get_proc_trampoline),
                ctx: &*proc as *const GetProcAddress as *mut c_void,
            };
            let api = cs("opengl");
            let mut params = vec![
                RenderParam { ty: PARAM_API_TYPE, data: api.as_ptr() as *mut c_void },
                RenderParam { ty: PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
            ];
            if !wl_display.is_null() {
                params.push(RenderParam { ty: PARAM_WL_DISPLAY, data: wl_display });
            }
            params.push(RenderParam { ty: PARAM_INVALID, data: std::ptr::null_mut() });
            let mut render = std::ptr::null_mut();
            check(mpv_render_context_create(&mut render, h, params.as_mut_ptr()), "mpv_render_context_create")?;

            let wake = Box::new(egui_ctx.clone());
            mpv_set_wakeup_callback(h, Some(repaint_trampoline), &*wake as *const _ as *mut c_void);
            let upd = Box::new(egui_ctx.clone());
            mpv_render_context_set_update_callback(render, Some(repaint_trampoline), &*upd as *const _ as *mut c_void);

            Ok(Self { h, render, _proc: proc, _egui_wakeup: wake, _egui_update: upd })
        }
    }

    // Всё ниже — асинхронно. Синхронные mpv_get_property/mpv_command из GL-потока ведут к взаимному
    // ожиданию: ядро ждёт vo, vo ждёт report_swap от нас, мы ждём ядро — разрывает только таймаут ~200 мс.

    pub fn command(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        unsafe { mpv_command_async(self.h, 0, ptrs.as_mut_ptr()) };
    }

    pub fn set(&self, name: &str, value: &str) {
        let v = cs(value);
        let mut p = v.as_ptr();
        unsafe { mpv_set_property_async(self.h, 0, cs(name).as_ptr(), FORMAT_STRING, &mut p as *mut _ as *mut c_void) };
    }

    /// Подписка на свойство: значения приходят событиями PROPERTY_CHANGE.
    pub fn observe(&self, name: &str, format: Format) {
        let f = match format {
            Format::Flag => FORMAT_FLAG,
            Format::Int => FORMAT_INT64,
            Format::Double => FORMAT_DOUBLE,
            Format::Str => FORMAT_STRING,
        };
        unsafe { mpv_observe_property(self.h, 0, cs(name).as_ptr(), f) };
    }

    /// Следующее событие без ожидания.
    pub fn poll_event(&self) -> Option<Event> {
        let e = unsafe { &*mpv_wait_event(self.h, 0.0) };
        if e.event_id == EVENT_NONE {
            return None;
        }
        let mut prop = None;
        if e.event_id == EVENT_PROPERTY_CHANGE && !e.data.is_null() {
            unsafe {
                let p = &*(e.data as *const RawEventProperty);
                let name = CStr::from_ptr(p.name).to_string_lossy().into_owned();
                let value = if p.data.is_null() {
                    None
                } else {
                    match p.format {
                        FORMAT_FLAG => Some(Prop::Flag(*(p.data as *const c_int) != 0)),
                        FORMAT_INT64 => Some(Prop::Int(*(p.data as *const i64))),
                        FORMAT_DOUBLE => Some(Prop::Double(*(p.data as *const f64))),
                        FORMAT_STRING => {
                            Some(Prop::Str(CStr::from_ptr(*(p.data as *const *const c_char)).to_string_lossy().into_owned()))
                        }
                        _ => None,
                    }
                };
                prop = Some((name, value));
            }
        }
        Some(Event { id: e.event_id, error: e.error, prop })
    }

    /// Есть ли новый кадр, который надо отрисовать.
    pub fn wants_frame(&self) -> bool {
        unsafe { mpv_render_context_update(self.render) & UPDATE_FRAME != 0 }
    }

    /// Рисует текущий кадр в FBO. Вызывать в потоке с текущим GL-контекстом.
    pub fn render(&self, fbo: u32, w: i32, h: i32, flip_y: bool) {
        let mut target = OpenGlFbo { fbo: fbo as c_int, w, h, internal_format: 0 };
        let mut flip: c_int = flip_y as c_int;
        // По умолчанию render() ждёт «времени показа» кадра и блокирует поток UI; если кадр однажды
        // опоздал (окно не перерисовалось вовремя), ждёт вечно. В egui — только без блокировки.
        let mut block: c_int = 0;
        let mut params = [
            RenderParam { ty: PARAM_OPENGL_FBO, data: &mut target as *mut _ as *mut c_void },
            RenderParam { ty: PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { ty: PARAM_BLOCK_FOR_TARGET_TIME, data: &mut block as *mut _ as *mut c_void },
            RenderParam { ty: PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        unsafe { mpv_render_context_render(self.render, params.as_mut_ptr()) };
    }

    pub fn report_swap(&self) {
        unsafe { mpv_render_context_report_swap(self.render) };
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // Порядок из документации: сначала render context (в GL-потоке), потом ядро.
        unsafe {
            mpv_render_context_free(self.render);
            mpv_terminate_destroy(self.h);
        }
    }
}
