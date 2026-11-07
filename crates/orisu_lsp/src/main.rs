fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("orisu-lsp {}", orisu_lsp::version());
        return;
    }
    if let Err(error) = orisu_lsp::run() {
        eprintln!("orisu-lsp: {error}");
        std::process::exit(1);
    }
}
