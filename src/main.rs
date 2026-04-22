use anyhow::Result;
use eframe::egui;
use incident_clustering_analyzer::app::IncidentClusteringApp;
use tracing_subscriber::{fmt, EnvFilter};

fn main() -> Result<()> {
    init_logging();
    let logical_cores = configure_parallelism()?;
    tracing::info!(logical_cores, "configured parallel processing thread pool");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Incident Clustering Analyzer")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 640.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Incident Clustering Analyzer",
        options,
        Box::new(|_cc| Ok(Box::<IncidentClusteringApp>::default())),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))
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
