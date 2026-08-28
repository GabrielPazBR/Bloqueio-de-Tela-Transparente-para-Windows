fn main() {
    #[cfg(windows)]
    if let Err(error) = bloqueio_transparente::windows_app::run() {
        eprintln!("Bloqueio Transparente: {error:#}");
        std::process::exit(1);
    }

    #[cfg(not(windows))]
    eprintln!("Bloqueio Transparente requer Windows.");
}
