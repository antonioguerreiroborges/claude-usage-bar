#![windows_subsystem = "windows"]

mod app;
mod auth;
mod fmt;
mod geom;
mod store;
mod usage;
mod win;
mod worker;

use eframe::egui;

fn main() -> eframe::Result<()> {
    store::init_log();
    std::panic::set_hook(Box::new(|info| log::error!("pânico: {info}")));

    if !win::single_instance() {
        log::info!("outra instância já está rodando; saindo");
        return Ok(());
    }
    log::info!("iniciando v{}", env!("CARGO_PKG_VERSION"));

    let cfg = store::Config::load();
    let width = geom::clamp_width(cfg.width_pt.unwrap_or(geom::DEFAULT_WIDTH));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Uso Claude")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(true)
            // Tamanho lógico: na mudança de DPI o winit o preserva.
            .with_inner_size([width, geom::HEIGHT])
            .with_min_inner_size([geom::MIN_WIDTH, geom::HEIGHT])
            .with_max_inner_size([geom::MAX_WIDTH, geom::HEIGHT])
            // Fora da tela até o primeiro frame posicionar a janela via Win32.
            .with_position([-32000.0, -32000.0]),
        ..Default::default()
    };
    eframe::run_native(
        "usage-bar-rs",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg)))),
    )
}
