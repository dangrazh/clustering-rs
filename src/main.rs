use anyhow::Result;
use incident_clustering_analyzer::web;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let logical_cores = configure_parallelism()?;
    tracing::info!(logical_cores, "configured parallel processing thread pool");

    let address = std::env::var("CLUSTERING_WEB_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    web::serve(address).await
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
