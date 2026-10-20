fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("orisu-lsp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(error) = orisu_lsp::run() {
        eprintln!("orisu-lsp: {error}");
        std::process::exit(1);
    }
}
