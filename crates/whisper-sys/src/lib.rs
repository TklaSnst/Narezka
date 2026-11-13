//! Сырые FFI-объявления whisper.cpp (bindgen). Безопасная обёртка — в `sc-analysis`.
//!
//! whisper.cpp и ggml собираются статически из `third_party/whisper.cpp` (закреплённая версия)
//! с бэкендом Vulkan; см. `build.rs`.
#![allow(non_camel_case_types, non_upper_case_globals, non_snake_case, dead_code, clippy::all)]

include!(concat!(env!("OUT_DIR"), "/whisper.rs"));
