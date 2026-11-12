fn main() {
    // libmpv-dev ставит libmpv.so и mpv.pc
    println!("cargo:rustc-link-lib=mpv");
}
