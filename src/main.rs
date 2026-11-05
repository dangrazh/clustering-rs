use anyhow::Result;
use incident_clustering_analyzer::config::AppConfig;
use incident_clustering_analyzer::web;

#[tokio::main]
async fn main() -> Result<()> {
    incident_clustering_analyzer::logging::LoggingConfig::from_env()?.init()?;
    let result = run().await;
    if let Err(error) = &result {
        tracing::error!(%error, "Application stopped with an error");
    }
    result
}

async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "healthcheck") {
        let address = std::env::var("CLUSTERING_WEB_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".into())
            .parse::<std::net::SocketAddr>()?;
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(4))
            .build()?
            .get(format!("http://127.0.0.1:{}/healthz", address.port()))
            .send()
            .await?
            .error_for_status()?;
        return Ok(());
    }
    if args.get(1).is_some_and(|s| s == "backup") {
        anyhow::ensure!(
            args.len() == 3,
            "Usage: incident-clustering-analyzer backup <new-directory>"
        );
        let root = std::env::var("APP_DATA_DIR").unwrap_or_else(|_| "data".into());
        let _service_lock = incident_clustering_analyzer::migration::service_lock(std::path::Path::new(&root))?;
        return incident_clustering_analyzer::backup::backup(
            &incident_clustering_analyzer::migration::resolve(std::path::Path::new(&root))?,
            std::path::Path::new(&args[2]),
        );
    }
    if args.get(1).is_some_and(|s| s == "restore") {
        anyhow::ensure!(
            args.len() == 4,
            "Usage: incident-clustering-analyzer restore <backup> <new-data-directory>"
        );
        return incident_clustering_analyzer::backup::restore(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        );
    }
    let logical_cores = configure_parallelism()?;
    if args.get(1).is_some_and(|s| s == "worker") {
        anyhow::ensure!(args.len() == 4, "Worker requires input and output paths");
        return incident_clustering_analyzer::jobs::worker(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        );
    }
    tracing::info!(logical_cores, "configured parallel processing thread pool");

    let address = std::env::var("CLUSTERING_WEB_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    let config = AppConfig::load_from_env()?;
    let root = std::path::absolute(std::env::var("APP_DATA_DIR").unwrap_or_else(|_| "data".into()))?;
    std::fs::create_dir_all(&root)?;
    let _service_lock = incident_clustering_analyzer::migration::service_lock(&root)?;
    // Recovery runs before opening the live database, including a rename interrupted by a crash.
    incident_clustering_analyzer::migration::offline(&root).await?;
    loop {
        web::serve(address, config.clone()).await?;
        if !incident_clustering_analyzer::migration::offline(&root).await? { return Ok(()); }
    }
}

fn configure_parallelism() -> Result<usize> {
    let logical_cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    let threads = incident_clustering_analyzer::config::number(
        "APP_WORKER_THREADS",
        logical_cores.saturating_sub(2).max(1),
        1,
        64,
    )?;
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()?;
    Ok(logical_cores)
}
