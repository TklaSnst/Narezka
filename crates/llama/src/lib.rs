//! llama-server (llama.cpp, Vulkan) отдельным процессом: запуск с моделью, ожидание готовности, запросы по HTTP.
//!
//! Процесс живёт, пока жив [`Server`]; при удалении он завершается и VRAM освобождается сразу —
//! так whisper и LLM гарантированно не занимают видеопамять одновременно (DECISIONS).
//! Запросы: эмбеддинги (`/v1/embeddings`) и чат с ответом по JSON-схеме (`/v1/chat/completions`).
//! Переменные ggml (`GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM`) сервер наследует от приложения.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum LlamaError {
    #[error("не удалось запустить {}: {source}", .path.display())]
    Spawn { path: PathBuf, source: std::io::Error },
    #[error("llama-server завершился при запуске: {0}")]
    Exited(String),
    #[error("llama-server не ответил за {0} с")]
    Timeout(u64),
    #[error("запрос к llama-server: {0}")]
    Http(String),
    #[error("ответ модели не по схеме: {0}")]
    BadAnswer(String),
    #[error("llama-server вернул {got} векторов вместо {want}")]
    Count { want: usize, got: usize },
    #[error("отменено")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Сколько строк лога сервера держать для сообщения об ошибке запуска.
const LOG_TAIL: usize = 20;
/// Загрузка модели в несколько гигабайт с холодного диска — до минуты; берём с запасом.
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// Предложений в одном запросе эмбеддингов.
const EMBED_CHUNK: usize = 64;

/// Путь к `llama-server`: `SC_LLAMA_SERVER` из окружения → рядом с исполняемым файлом → собранный `build.rs`.
pub fn server_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("SC_LLAMA_SERVER") {
        return p.into();
    }
    let near = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("llama-server")));
    near.filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(env!("SC_LLAMA_SERVER_BUILT")))
}

#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// `None` — [`server_binary`].
    pub binary: Option<PathBuf>,
    pub model: PathBuf,
    /// Контекст в токенах (на все слоты).
    pub ctx: u32,
    /// Режим эмбеддингов (`--embedding`); пулинг берётся из метаданных GGUF.
    pub embedding: bool,
    /// Дополнительные аргументы командной строки.
    pub args: Vec<String>,
}

impl ServerOptions {
    /// Эмбеддинги предложений: предложение (≤30 с, ~250 токенов) целиком влезает в один ubatch;
    /// контекст 2048 — KV 224 МБ вместо 896 при 8192, скорость та же (~22 мс на предложение, шаг 2.2).
    pub fn embedding(model: &Path) -> Self {
        let args = ["-b", "2048", "-ub", "2048"].map(String::from).to_vec();
        Self { binary: None, model: model.to_owned(), ctx: 2048, embedding: true, args }
    }

    /// LLM для нарезки (шаг 0.3): все слои на GPU, flash attention, шаблон чата из GGUF, рассуждения выключены —
    /// с грамматикой они не завершаются. Контекст 12288 — окно 6 мин (~3–4 тыс. токенов) с промптом и ответом с запасом.
    pub fn chat(model: &Path) -> Self {
        let args = ["-ngl", "99", "-fa", "on", "--jinja", "--reasoning-budget", "0"].map(String::from).to_vec();
        Self { binary: None, model: model.to_owned(), ctx: 12288, embedding: false, args }
    }
}

/// Ответ модели упёрся в `max_tokens` — JSON недописан.
const FINISH_LENGTH: &str = "length";

pub struct Server {
    child: Child,
    base: String,
    agent: ureq::Agent,
}

impl Server {
    /// Запускает сервер и ждёт, пока модель загрузится. `cancel` проверяется во время ожидания.
    pub fn start(opts: &ServerOptions, cancel: &AtomicBool) -> Result<Self, LlamaError> {
        let bin = opts.binary.clone().unwrap_or_else(server_binary);
        let port = free_port()?;
        let mut cmd = Command::new(&bin);
        cmd.arg("-m").arg(&opts.model)
            .args(["-c", &opts.ctx.to_string()])
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "-np", "1"])
            .args(&opts.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if opts.embedding {
            cmd.arg("--embedding");
        }
        log::info!("llama-server: {} на порту {port}", opts.model.display());
        let mut child = cmd.spawn().map_err(|source| LlamaError::Spawn { path: bin, source })?;
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_TAIL)));
        forward_log(child.stderr.take().expect("stderr piped"), tail.clone());

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .into();
        let mut server = Self { child, base: format!("http://127.0.0.1:{port}"), agent };
        server.wait_ready(cancel, &tail)?;
        Ok(server)
    }

    fn wait_ready(&mut self, cancel: &AtomicBool, tail: &Mutex<VecDeque<String>>) -> Result<(), LlamaError> {
        let started = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(LlamaError::Cancelled);
            }
            match self.agent.get(format!("{}/health", self.base)).call() {
                Ok(r) if r.status() == 200 => {
                    log::info!("llama-server готов за {:.1} с", started.elapsed().as_secs_f32());
                    return Ok(());
                }
                // 503 — модель ещё грузится; ошибка соединения — сервер ещё не слушает порт.
                _ => {}
            }
            if started.elapsed() > START_TIMEOUT {
                return Err(LlamaError::Timeout(START_TIMEOUT.as_secs()));
            }
            if self.child.try_wait()?.is_some() {
                // Поток лога дочитывает stderr — даём ему время, чтобы причина попала в сообщение.
                std::thread::sleep(Duration::from_millis(200));
                let log = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
                return Err(LlamaError::Exited(log));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Эмбеддинги текстов (L2-нормированные), в том же порядке.
    pub fn embeddings(&self, texts: &[String], cancel: &AtomicBool) -> Result<Vec<Vec<f32>>, LlamaError> {
        #[derive(Deserialize)]
        struct Item {
            index: usize,
            embedding: Vec<f32>,
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Vec<Item>,
        }
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(EMBED_CHUNK) {
            if cancel.load(Ordering::Relaxed) {
                return Err(LlamaError::Cancelled);
            }
            let mut resp = self
                .agent
                .post(format!("{}/v1/embeddings", self.base))
                .send_json(serde_json::json!({ "input": chunk }))
                .map_err(|e| LlamaError::Http(e.to_string()))?;
            if resp.status() != 200 {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                return Err(LlamaError::Http(format!("{}: {body}", resp.status())));
            }
            let mut data = resp.body_mut().read_json::<Resp>().map_err(|e| LlamaError::Http(e.to_string()))?.data;
            if data.len() != chunk.len() {
                return Err(LlamaError::Count { want: chunk.len(), got: data.len() });
            }
            data.sort_by_key(|d| d.index);
            out.extend(data.into_iter().map(|d| normalized(d.embedding)));
        }
        Ok(out)
    }

    /// Ответ чат-модели строго по JSON-схеме (llama.cpp строит из неё грамматику). Схема гарантирует формат,
    /// но не смысл — номера и диапазоны проверяет вызывающий. `cancel` проверяется до запроса: сам запрос
    /// (секунды) не прерывается.
    pub fn chat_json<T: serde::de::DeserializeOwned>(
        &self,
        req: &ChatRequest,
        cancel: &AtomicBool,
    ) -> Result<T, LlamaError> {
        #[derive(Deserialize)]
        struct Message {
            content: String,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Message,
            finish_reason: Option<String>,
        }
        #[derive(Deserialize, Default)]
        struct Timings {
            prompt_n: u32,
            prompt_ms: f64,
            predicted_n: u32,
            predicted_ms: f64,
        }
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<Choice>,
            #[serde(default)]
            timings: Timings,
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(LlamaError::Cancelled);
        }
        let body = serde_json::json!({
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
            "temperature": req.temperature,
            "seed": 1,
            "max_tokens": req.max_tokens,
            "response_format": { "type": "json_schema", "json_schema": { "name": "out", "schema": req.schema } },
            "chat_template_kwargs": { "enable_thinking": false },
        });
        let mut resp = self
            .agent
            .post(format!("{}/v1/chat/completions", self.base))
            .send_json(body)
            .map_err(|e| LlamaError::Http(e.to_string()))?;
        if resp.status() != 200 {
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(LlamaError::Http(format!("{}: {body}", resp.status())));
        }
        let resp = resp.body_mut().read_json::<Resp>().map_err(|e| LlamaError::Http(e.to_string()))?;
        let t = &resp.timings;
        log::debug!(
            "чат: промпт {} ток. за {:.1} с, ответ {} ток. за {:.1} с",
            t.prompt_n,
            t.prompt_ms / 1e3,
            t.predicted_n,
            t.predicted_ms / 1e3
        );
        let choice = resp.choices.into_iter().next().ok_or_else(|| LlamaError::BadAnswer("нет choices".into()))?;
        if choice.finish_reason.as_deref() == Some(FINISH_LENGTH) {
            return Err(LlamaError::BadAnswer(format!("ответ обрезан на {} токенах", req.max_tokens)));
        }
        serde_json::from_str(&choice.message.content).map_err(|e| {
            let head: String = choice.message.content.chars().take(200).collect();
            LlamaError::BadAnswer(format!("{e}: {head}"))
        })
    }
}

/// Запрос к чат-модели с ответом по JSON-схеме.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    pub schema: serde_json::Value,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// Свободный порт на localhost: занимаем `:0`, запоминаем номер и отпускаем.
fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// Лог сервера → `log::debug`, последние строки — для сообщения об ошибке.
fn forward_log(stderr: std::process::ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            log::debug!(target: "llama-server", "{line}");
            let mut t = tail.lock().unwrap();
            if t.len() == LOG_TAIL {
                t.pop_front();
            }
            t.push_back(line);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        let v = normalized(vec![3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        assert_eq!(normalized(vec![0.0, 0.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn missing_binary_is_spawn_error() {
        let opts = ServerOptions {
            binary: Some("/nonexistent/llama-server".into()),
            ..ServerOptions::embedding(Path::new("/nonexistent.gguf"))
        };
        let err = Server::start(&opts, &AtomicBool::new(false)).err().unwrap();
        assert!(matches!(err, LlamaError::Spawn { .. }), "{err}");
    }

    #[test]
    fn bad_model_reports_exit() {
        let tmp = std::env::temp_dir().join("sc-llama-bad.gguf");
        std::fs::write(&tmp, b"not a gguf").unwrap();
        let err = Server::start(&ServerOptions::embedding(&tmp), &AtomicBool::new(false)).err().unwrap();
        assert!(matches!(err, LlamaError::Exited(_)), "{err}");
    }
}
