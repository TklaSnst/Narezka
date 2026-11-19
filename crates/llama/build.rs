//! Сборка `llama-server` из `third_party/llama.cpp` (статически, Vulkan).
//!
//! llama.cpp не линкуется в приложение: в нём своя копия ggml, которая конфликтует с ggml из whisper.cpp
//! (DECISIONS, шаг 2.2). Путь к собранному бинарнику попадает в код через `SC_LLAMA_SERVER_BUILT`.
//! Первая сборка — несколько минут (шейдеры Vulkan), дальше — из кэша cargo.

use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../third_party/llama.cpp");
    let root = root.canonicalize().unwrap_or_else(|_| {
        panic!("нет исходников llama.cpp в {} — выполните `git submodule update --init`", root.display())
    });
    println!("cargo:rerun-if-changed={}", root.join("CMakeLists.txt").display());
    println!("cargo:rerun-if-changed={}", root.join("include/llama.h").display());

    let mut cfg = cmake::Config::new(&root);
    cpu_flags(&mut cfg);
    let dst = cfg
        .profile("Release")
        .build_target("llama-server")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("GGML_VULKAN", "ON")
        .define("GGML_OPENMP", "OFF")
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_APP", "OFF")
        .define("LLAMA_BUILD_TOOLS", "ON")
        .define("LLAMA_BUILD_SERVER", "ON")
        // Веб-интерфейс сервера не нужен и не должен качаться при сборке.
        .define("LLAMA_BUILD_UI", "OFF")
        .define("LLAMA_USE_PREBUILT_UI", "OFF")
        .define("LLAMA_OPENSSL", "OFF")
        .build();

    let bin = dst.join("build/bin/llama-server");
    assert!(bin.exists(), "cmake не собрал {}", bin.display());
    println!("cargo:rustc-env=SC_LLAMA_SERVER_BUILT={}", bin.display());
}

/// `SC_PORTABLE=1` (сборка пакета): ggml не под этот CPU (`GGML_NATIVE`), а под x86-64-v3 —
/// AVX2/FMA/F16C/BMI2, любой x86-64 с 2015 г. Считает GPU (Vulkan), CPU-часть графа мала.
fn cpu_flags(cfg: &mut cmake::Config) {
    println!("cargo:rerun-if-env-changed=SC_PORTABLE");
    if env::var_os("SC_PORTABLE").is_none_or(|v| v.is_empty() || v == "0") {
        return;
    }
    cfg.define("GGML_NATIVE", "OFF");
    for flag in ["GGML_SSE42", "GGML_AVX", "GGML_AVX2", "GGML_FMA", "GGML_F16C", "GGML_BMI2"] {
        cfg.define(flag, "ON");
    }
}
