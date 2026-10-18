//! Minimal consumer of the future renderer boundary, with no adapter imports.
use talk_core::ui::DesktopPort;

pub fn describe(backend: &mut dyn DesktopPort) {
    let _snapshot = backend.take_snapshot();
    println!("Nextcloud Modern Talk: contract bootstrap only.");
    println!("Renderer, network, vault, media and runtime are unconfigured.");
}
