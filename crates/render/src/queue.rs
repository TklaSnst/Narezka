//! Очередь рендера: N задач параллельно, прогресс по каждой, пауза, отмена по одной и всех.
//!
//! Пауза — `SIGSTOP` запущенным ffmpeg (и новые задачи не стартуют), продолжение — `SIGCONT`.
//! Отмена задачи — флаг и `SIGKILL` её ffmpeg (остановленный процесс сам флаг не заметит);
//! недописанный файл удаляет [`RenderJob::run_with`]. Ошибка одной задачи остальные не останавливает.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::job::{RenderError, RenderJob};

/// События очереди; `index` — номер задачи в переданном списке.
#[derive(Debug)]
pub enum QueueEvent {
    Started { index: usize },
    Progress { index: usize, fraction: f32 },
    /// Задача закончилась: готова, ошибка или отменена (`Err(Cancelled)`, в том числе не начатая).
    Finished { index: usize, result: Result<(), RenderError> },
    /// Все задачи закончились.
    Done,
}

/// Управление одной задачей.
#[derive(Default)]
struct Slot {
    cancel: AtomicBool,
    /// pid запущенного ffmpeg задачи; сигналы — под этим же замком, чтобы не задеть чужой процесс.
    pid: Mutex<Option<u32>>,
}

struct Shared {
    slots: Vec<Slot>,
    paused: Mutex<bool>,
    resumed: Condvar,
    next: AtomicUsize,
}

impl Shared {
    fn is_paused(&self) -> bool {
        *self.paused.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ждёт снятия паузы.
    fn wait_resumed(&self) {
        let mut p = self.paused.lock().unwrap_or_else(|e| e.into_inner());
        while *p {
            p = self.resumed.wait(p).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Запомнить процесс задачи; если очередь на паузе — сразу остановить и его.
    fn attach(&self, index: usize, pid: Option<u32>) {
        let mut slot = self.slots[index].pid.lock().unwrap_or_else(|e| e.into_inner());
        *slot = pid;
        if let Some(pid) = pid {
            if self.slots[index].cancel.load(Ordering::Relaxed) {
                signal(pid, libc::SIGKILL);
            } else if self.is_paused() {
                signal(pid, libc::SIGSTOP);
            }
        }
    }

    fn signal_all(&self, sig: i32) {
        for slot in &self.slots {
            if let Some(pid) = *slot.pid.lock().unwrap_or_else(|e| e.into_inner()) {
                signal(pid, sig);
            }
        }
    }
}

fn signal(pid: u32, sig: i32) {
    // SAFETY: kill(2) только посылает сигнал; pid — живой дочерний процесс (снимается под замком до wait).
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

/// Запущенная очередь. Дроп не останавливает задачи — для этого [`Queue::cancel_all`].
#[derive(Clone)]
pub struct Queue {
    shared: Arc<Shared>,
}

impl Queue {
    /// Запускает `jobs` в `workers` потоках (по порядку списка). События приходят из рабочих потоков.
    pub fn start(jobs: Vec<RenderJob>, workers: usize, on_event: impl Fn(QueueEvent) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            slots: jobs.iter().map(|_| Slot::default()).collect(),
            paused: Mutex::new(false),
            resumed: Condvar::new(),
            next: AtomicUsize::new(0),
        });
        let jobs = Arc::new(jobs);
        let on_event = Arc::new(on_event);
        let workers = workers.clamp(1, jobs.len().max(1));
        let left = Arc::new(AtomicUsize::new(workers));
        for _ in 0..workers {
            let (shared, jobs, on_event, left) = (shared.clone(), jobs.clone(), on_event.clone(), left.clone());
            thread::spawn(move || {
                worker(&shared, &jobs, &*on_event);
                if left.fetch_sub(1, Ordering::AcqRel) == 1 {
                    on_event(QueueEvent::Done);
                }
            });
        }
        Self { shared }
    }

    pub fn is_paused(&self) -> bool {
        self.shared.is_paused()
    }

    /// Пауза: запущенные ffmpeg останавливаются, новые задачи не начинаются.
    pub fn pause(&self) {
        *self.shared.paused.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.shared.signal_all(libc::SIGSTOP);
    }

    pub fn resume(&self) {
        *self.shared.paused.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.shared.signal_all(libc::SIGCONT);
        self.shared.resumed.notify_all();
    }

    /// Отменить задачу: не начатая пропускается, идущая прерывается (файл удаляется).
    pub fn cancel(&self, index: usize) {
        let Some(slot) = self.shared.slots.get(index) else { return };
        let pid = slot.pid.lock().unwrap_or_else(|e| e.into_inner());
        slot.cancel.store(true, Ordering::Relaxed);
        if let Some(pid) = *pid {
            signal(pid, libc::SIGKILL);
        }
    }

    pub fn is_cancelled(&self, index: usize) -> bool {
        self.shared.slots.get(index).is_some_and(|s| s.cancel.load(Ordering::Relaxed))
    }

    /// Отменить всё; с паузы очередь снимается, чтобы потоки дошли до конца.
    pub fn cancel_all(&self) {
        for i in 0..self.shared.slots.len() {
            self.cancel(i);
        }
        self.resume();
    }
}

fn worker(shared: &Shared, jobs: &[RenderJob], on_event: &(impl Fn(QueueEvent) + ?Sized)) {
    loop {
        shared.wait_resumed();
        let index = shared.next.fetch_add(1, Ordering::AcqRel);
        let Some(job) = jobs.get(index) else { return };
        let slot = &shared.slots[index];
        if slot.cancel.load(Ordering::Relaxed) {
            on_event(QueueEvent::Finished { index, result: Err(RenderError::Cancelled) });
            continue;
        }
        on_event(QueueEvent::Started { index });
        let mut last = -1.0;
        let result = job.run_with(&slot.cancel, &|pid| shared.attach(index, pid), |fraction| {
            // Не чаще чем раз в процент — UI не нужно больше.
            if fraction - last >= 0.01 || fraction >= 1.0 {
                last = fraction;
                on_event(QueueEvent::Progress { index, fraction });
            }
        });
        // Убит снаружи — это отмена, а не ошибка ffmpeg.
        let result = match result {
            Err(_) if slot.cancel.load(Ordering::Relaxed) => Err(RenderError::Cancelled),
            r => r,
        };
        if let Err(e) = &result
            && !matches!(e, RenderError::Cancelled)
        {
            log::warn!("рендер {}: {}", job.dest.display(), e.log());
        }
        on_event(QueueEvent::Finished { index, result });
    }
}
