//! Импорт файлов в библиотеку без интерфейса (для проверок): `cargo run -p sc-library --example import -- <каталог> <файлы…>`.
//! Каталог приложения — `~/.local/share/shortscutter/library`. Имя шрифта здесь не заполняется (его узнаёт растеризатор).

fn main() {
    let mut args = std::env::args_os().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("каталог библиотеки"));
    let mut lib = sc_library::Library::open(&dir);
    for f in args {
        let path = std::path::PathBuf::from(f);
        let r = sc_library::stage(&dir, &path, |h| lib.by_hash(h).is_some()).and_then(|s| lib.add(s));
        println!("{}: {r:?}", path.display());
    }
}
