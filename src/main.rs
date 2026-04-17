mod app;
mod ui;

pub use app::Message;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sassy_mc=info,warn".into()),
        )
        .init();

    tracing::info!("starting Mission Control Desktop");

    iced::application(app::App::default, app::App::update, app::App::view)
        .title("Mission Control")
        .theme(app::App::theme)
        .window_size((1280.0, 800.0))
        .run()
}
