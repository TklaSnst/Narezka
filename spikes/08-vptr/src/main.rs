//! Виртуальная мышь для проверки UI в headless sway (zwlr_virtual_pointer_v1): у seat без устройств нет указателя,
//! `swaymsg seat … cursor` до клиента не доходит. Команды — по строке из stdin (держать открытым через FIFO):
//!   move X Y | down | up | click X Y | drag X0 Y0 X1 Y1 [шагов] | wheel DY | sleep МС
//! Координаты — пиксели вывода; размер вывода — аргументы (по умолчанию 1920 1080).
use std::io::BufRead;
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

struct S;
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for S {
    fn event(_: &mut S, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<S>) {}
}
impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for S {
    fn event(_: &mut S, _: &ZwlrVirtualPointerManagerV1, _: <ZwlrVirtualPointerManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<S>) {}
}
impl Dispatch<ZwlrVirtualPointerV1, ()> for S {
    fn event(_: &mut S, _: &ZwlrVirtualPointerV1, _: <ZwlrVirtualPointerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<S>) {}
}

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

fn main() {
    let args: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (w, h) = (args.first().copied().unwrap_or(1920), args.get(1).copied().unwrap_or(1080));
    let conn = Connection::connect_to_env().expect("нет WAYLAND_DISPLAY");
    let (globals, mut q) = registry_queue_init::<S>(&conn).unwrap();
    let qh = q.handle();
    let mgr: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).expect("нет zwlr_virtual_pointer_manager_v1");
    let p = mgr.create_virtual_pointer(None, &qh, ());
    q.roundtrip(&mut S).unwrap();
    let t0 = Instant::now();
    let now = || t0.elapsed().as_millis() as u32;
    let mut pos = (0.0f64, 0.0f64);
    let mut send = |f: &dyn Fn()| {
        f();
        p.frame();
        q.roundtrip(&mut S).unwrap();
    };
    let mv = |x: f64, y: f64| p.motion_absolute(now(), x.round() as u32, y.round() as u32, w, h);
    let btn_of = |b: u32, down: bool| p.button(now(), b, if down { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released });
    let btn = |down: bool| btn_of(BTN_LEFT, down);
    let pause = |ms: u64| std::thread::sleep(Duration::from_millis(ms));
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let v: Vec<&str> = line.split_whitespace().collect();
        let n = |i: usize| v.get(i).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        match v.first().copied() {
            Some("move") => {
                pos = (n(1), n(2));
                send(&|| mv(pos.0, pos.1));
            }
            Some("down") => send(&|| btn(true)),
            Some("up") => send(&|| btn(false)),
            Some("click") => {
                pos = (n(1), n(2));
                send(&|| mv(pos.0, pos.1));
                pause(50);
                send(&|| btn(true));
                pause(60);
                send(&|| btn(false));
            }
            // Правая кнопка.
            Some("rclick") => {
                pos = (n(1), n(2));
                send(&|| mv(pos.0, pos.1));
                pause(50);
                send(&|| btn_of(BTN_RIGHT, true));
                pause(60);
                send(&|| btn_of(BTN_RIGHT, false));
            }
            Some("drag") => {
                let (x0, y0, x1, y1) = (n(1), n(2), n(3), n(4));
                let steps = if v.len() > 5 { n(5) as u32 } else { 20 };
                send(&|| mv(x0, y0));
                pause(50);
                send(&|| btn(true));
                for i in 1..=steps {
                    let k = i as f64 / steps as f64;
                    pause(16);
                    send(&|| mv(x0 + (x1 - x0) * k, y0 + (y1 - y0) * k));
                }
                pause(80);
                send(&|| btn(false));
                pos = (x1, y1);
            }
            Some("wheel") => {
                let dy = n(1);
                send(&|| {
                    p.axis_source(wl_pointer::AxisSource::Wheel);
                    p.axis_discrete(now(), wl_pointer::Axis::VerticalScroll, dy * 15.0, dy as i32);
                });
            }
            Some("sleep") => pause(n(1) as u64),
            _ => {}
        }
    }
    let _ = pos;
}
