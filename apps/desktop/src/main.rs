mod bundle;
mod composition;
mod devices;
mod presentation;
mod settings;

fn main() {
    if bundle::prepare().is_err() {
        eprintln!(
            "The bundled media runtime could not be initialized. Reinstall NextCloud Modern Talk from its disk image."
        );
        std::process::exit(1);
    }
    composition::run();
}
