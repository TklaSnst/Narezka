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

    let dst = cmake::Config::new(&root)
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
