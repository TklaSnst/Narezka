//! Запуск внешних утилит с таймаутом.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::MediaError;

pub(crate) struct Output {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Запускает команду и читает stdout/stderr; по таймауту убивает процесс.
pub(crate) fn run(tool: &'static str, cmd: &mut Command, timeout: Duration) -> Result<Output, MediaError> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| MediaError::Spawn { tool, source })?;

    // Каналы читаем в потоках, иначе процесс может встать на заполненном пайпе.
    let mut out = child.stdout.take().expect("stdout piped");
    let mut err = child.stderr.take().expect("stderr piped");
    let out_t = thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::Timeout { tool, secs: timeout.as_secs() });
        }
        thread::sleep(Duration::from_millis(20));
    };

    Ok(Output {
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
        success: status.success(),
    })
}
