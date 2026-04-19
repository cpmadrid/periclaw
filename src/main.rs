mod app;
mod config;
mod domain;
mod net;
mod scene;
mod ui;

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

    iced::application(app::App::default, app::App::update, app::App::view)
        .title("Mission Control")
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .window_size((1280.0, 800.0))
        .run()
}
