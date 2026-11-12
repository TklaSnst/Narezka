use std::{env, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../out/whisper.cpp");
    let lib = root.join("build/bin").canonicalize().unwrap();
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    println!("cargo:rustc-link-lib=dylib=whisper");
    bindgen::Builder::default()
        .header(root.join("include/whisper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", root.join("ggml/include").display()))
        // у системного libclang нет своих заголовков (stdbool.h) — берём у gcc
        .clang_arg(format!("-I{}", gcc_include()))
        .allowlist_function("whisper_.*")
        .allowlist_type("whisper_.*")
        .generate()
        .unwrap()
        .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("whisper.rs"))
        .unwrap();
}

fn gcc_include() -> String {
    let out = std::process::Command::new("gcc").arg("-print-file-name=include").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
