mod bundle;
mod composition;
mod devices;
mod diagnostics;
mod presentation;
mod settings;

fn main() {
    if bundle::prepare().is_err() {
        eprintln!(
            "The bundled media runtime could not be initialized. Reinstall NextCloud Modern Talk from its disk image."
        );
        std::process::exit(1);
    }
    #[cfg(all(feature = "dev-probes", debug_assertions))]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--dev-device-call")) {
        std::process::exit(composition::device_call::run());
    }
    let _diagnostics = diagnostics::start();
    composition::run();
}
