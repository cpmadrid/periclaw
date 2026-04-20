mod app;
mod config;
mod device_identity;
mod domain;
mod logs;
mod net;
mod notifications;
mod palette;
mod scene;
mod transcript;
mod ui;
mod ui_state;

pub use app::Message;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sassy_mc=info,warn".into()),
        )
        .init();

    // Required before opening any `wss://` connection. rustls 0.23
    // refuses to pick a default crypto backend at runtime unless one
    // is either selected at crate-feature time or installed here.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring crypto provider");

    tracing::info!("starting Mission Control Desktop");

    // Pull persisted UI state before the window opens so restored
    // dimensions apply on first paint, not after a visible resize.
    let loaded = ui_state::load();

    let window_size = loaded
        .window
        .map(|w| iced::Size::new(w.width, w.height))
        .unwrap_or_else(|| iced::Size::new(1280.0, 800.0));
    let window_position = loaded
        .window
        .and_then(|w| w.position)
        .map(|(x, y)| iced::window::Position::Specific(iced::Point::new(x, y)))
        .unwrap_or_default();

    iced::application(
        move || app::App::new(loaded.clone()),
        app::App::update,
        app::App::view,
    )
    .title("Mission Control")
    .theme(app::App::theme)
    .subscription(app::App::subscription)
    .window(iced::window::Settings {
        size: window_size,
        position: window_position,
        ..Default::default()
    })
    .run()
}
