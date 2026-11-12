fn main() {
    // libmpv.so из пакета libmpv-dev (или аналога в другом дистрибутиве).
    println!("cargo:rustc-link-lib=mpv");
}
