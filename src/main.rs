#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter};

use gpui::{size, App, AppContext, Application, WindowBounds, WindowOptions};
use gpui_component::{
    theme::Theme,
    Root,
};
use incident_clustering_analyzer::app::IncidentClusteringApp;

fn main() -> Result<()> {
    init_logging();
    let logical_cores = configure_parallelism()?;
    tracing::info!(logical_cores, "configured parallel processing thread pool");

    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::sync_system_appearance(None, cx);

        let bounds = WindowBounds::centered(size(gpui::px(1280.0), gpui::px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(bounds),
            window_min_size: Some(size(gpui::px(960.0), gpui::px(640.0))),
            app_id: Some("incident-clustering-analyzer".into()),
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let view = cx.new(|_cx| IncidentClusteringApp::default());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open GPUI window");
    });

    Ok(())
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).without_time().try_init();
}

fn configure_parallelism() -> Result<usize> {
    let logical_cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    rayon::ThreadPoolBuilder::new()
        .num_threads(logical_cores)
        .build_global()?;
    Ok(logical_cores)
}
