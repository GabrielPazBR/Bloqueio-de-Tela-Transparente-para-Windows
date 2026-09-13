#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    {
        // The release pipeline signs the application BEFORE embedding it.
        const PAYLOAD: &[u8] = include_bytes!(env!("BT_APP_PAYLOAD"));
        if let Err(error) = bloqueio_transparente::windows_app::run_installer(PAYLOAD) {
            if std::env::args()
                .any(|argument| matches!(argument.as_str(), "--repair-quiet" | "--update-quiet"))
            {
                eprintln!("{error:#}");
            } else {
                bloqueio_transparente::windows_app::show_installer_error(&format!("{error:#}"));
            }
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    eprintln!("O instalador requer Windows.");
}
