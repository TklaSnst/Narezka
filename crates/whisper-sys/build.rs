//! Сборка whisper.cpp (статически, Vulkan) и генерация биндингов.
//!
//! Первая сборка — ~2 мин (шейдеры Vulkan); дальше cargo её не повторяет, пока не изменится `build.rs`
//! или версия в `third_party/whisper.cpp`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::env;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../third_party/whisper.cpp");
    let root = root.canonicalize().unwrap_or_else(|_| {
        panic!("нет исходников whisper.cpp в {} — выполните `git submodule update --init`", root.display())
    });
    println!("cargo:rerun-if-changed={}", root.join("include/whisper.h").display());
    println!("cargo:rerun-if-changed={}", root.join("CMakeLists.txt").display());

    let dst = cmake::Config::new(&root)
        .profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("GGML_VULKAN", "ON")
        // OpenMP не нужен: считает GPU; без него не тянем libgomp.
        .define("GGML_OPENMP", "OFF")
        .define("WHISPER_BUILD_TESTS", "OFF")
        .define("WHISPER_BUILD_EXAMPLES", "OFF")
        .define("WHISPER_BUILD_SERVER", "OFF")
        .define("WHISPER_SDL2", "OFF")
        .define("WHISPER_CURL", "OFF")
        .build();

    for dir in ["lib", "lib64"] {
        println!("cargo:rustc-link-search=native={}", dst.join(dir).display());
    }
    // Порядок важен для статической линковки: зависимые раньше зависимостей.
    for lib in ["whisper", "ggml", "ggml-vulkan", "ggml-cpu", "ggml-base"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    println!("cargo:rustc-link-lib=dylib=vulkan");
    println!("cargo:rustc-link-lib=dylib=stdc++");

    bindgen::Builder::default()
        .header(root.join("include/whisper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", root.join("ggml/include").display()))
        .clang_args(gcc_include().map(|dir| format!("-I{dir}")))
        .allowlist_function("whisper_.*")
        .allowlist_type("whisper_.*")
        .generate()
        .expect("bindgen: whisper.h")
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("whisper.rs"))
        .unwrap();
}

/// У системного libclang может не быть своих заголовков (`stdbool.h`) — берём у gcc.
fn gcc_include() -> Option<String> {
    let out = Command::new("gcc").arg("-print-file-name=include").output().ok()?;
    let dir = String::from_utf8(out.stdout).ok()?.trim().to_string();
    Path::new(&dir).join("stdbool.h").exists().then_some(dir)
}
