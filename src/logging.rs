//! Operator-controlled logging, shared by the server and clustering workers.
use anyhow::{Context, Result};
use std::{fs::OpenOptions, path::PathBuf, sync::Mutex};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub struct LoggingConfig {
    pub filter: EnvFilter,
    pub console: bool,
    pub file: Option<PathBuf>,
}

impl LoggingConfig {
    pub fn from_env() -> Result<Self> {
        Self::parse(
            std::env::var("APP_LOG_LEVEL").ok().as_deref(),
            std::env::var("RUST_LOG").ok().as_deref(),
            std::env::var("APP_LOG_CONSOLE").ok().as_deref(),
            std::env::var("APP_LOG_FILE").ok().as_deref(),
        )
    }

    fn parse(
        level: Option<&str>,
        rust_log: Option<&str>,
        console: Option<&str>,
        file: Option<&str>,
    ) -> Result<Self> {
        let level = level.unwrap_or("INFO").trim().to_ascii_uppercase();
        anyhow::ensure!(
            matches!(
                level.as_str(),
                "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE"
            ),
            "APP_LOG_LEVEL must be ERROR, WARN, INFO, DEBUG or TRACE"
        );
        // Verbose application diagnostics should not enable database internals.
        let default_filter = match level.as_str() {
            "DEBUG" | "TRACE" => format!(
                "info,incident_clustering_analyzer={}",
                level.to_ascii_lowercase()
            ),
            _ => level.to_ascii_lowercase(),
        };
        let filter = EnvFilter::try_new(rust_log.unwrap_or(&default_filter))
            .context("Invalid RUST_LOG filter")?;
        let console = match console
            .unwrap_or("true")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "true" => true,
            "false" => false,
            _ => anyhow::bail!("APP_LOG_CONSOLE must be true or false"),
        };
        let file = file
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        anyhow::ensure!(
            console || file.is_some(),
            "Enable APP_LOG_CONSOLE or set APP_LOG_FILE"
        );
        Ok(Self {
            filter,
            console,
            file,
        })
    }

    pub fn init(self) -> Result<()> {
        let file = self
            .file
            .map(|path| {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .with_context(|| format!("Cannot open log file {}", path.display()))
                    .map(Mutex::new)
            })
            .transpose()?;
        let console = self.console.then(|| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr)
        });
        let file = file.map(|writer| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer)
        });
        tracing_subscriber::registry()
            .with(self.filter)
            .with(console)
            .with(file)
            .try_init()
            .context("Cannot initialize logging")?;
        Ok(())
    }
}

/// Only routine successful traffic is demoted; failures retain DEBUG visibility.
pub(crate) fn routine_response(
    method: &http::Method,
    path: &str,
    status: http::StatusCode,
) -> bool {
    if !status.is_success() && status != http::StatusCode::NOT_MODIFIED {
        return false;
    }
    if method == http::Method::GET || method == http::Method::HEAD {
        return matches!(
            path,
            "/healthz"
                | "/api/jobs"
                | "/api/dashboard/data"
                | "/api/maintenance"
                | "/api/admin/status"
        ) || (!path.starts_with("/api/")
            && (path.ends_with(".js") || path.ends_with(".css") || path == "/favicon.ico"));
    }
    method == http::Method::POST
        && path.strip_prefix("/api/analyses/").is_some_and(|tail| {
            tail.split_once('/')
                .is_some_and(|(id, action)| !id.is_empty() && action == "presence")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routine_responses_keep_errors_and_user_operations_visible() {
        use http::{Method, StatusCode};
        for path in [
            "/app.js",
            "/app.css",
            "/healthz",
            "/api/jobs",
            "/api/dashboard/data",
        ] {
            assert!(routine_response(&Method::GET, path, StatusCode::OK));
            assert!(!routine_response(
                &Method::GET,
                path,
                StatusCode::INTERNAL_SERVER_ERROR
            ));
            assert!(!routine_response(&Method::POST, path, StatusCode::OK));
        }
        assert!(routine_response(
            &Method::POST,
            "/api/analyses/123/presence",
            StatusCode::OK
        ));
        assert!(!routine_response(
            &Method::POST,
            "/api/analyses/123/presence",
            StatusCode::UNAUTHORIZED
        ));
        assert!(!routine_response(
            &Method::GET,
            "/api/analyses/123",
            StatusCode::OK
        ));
        assert!(!routine_response(
            &Method::GET,
            "/api/analyses/example.js",
            StatusCode::OK
        ));
    }

    #[test]
    fn verbose_levels_only_enable_application_diagnostics_unless_overridden() {
        for (level, override_filter, dependency_debug) in [
            ("DEBUG", None, false),
            ("TRACE", None, false),
            ("DEBUG", Some("debug"), true),
        ] {
            let file = tempfile::NamedTempFile::new().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(
                    LoggingConfig::parse(Some(level), override_filter, None, None)
                        .unwrap()
                        .filter,
                )
                .with_ansi(false)
                .with_writer(Mutex::new(file.reopen().unwrap()))
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                tracing::debug!("application debug");
                tracing::debug!(target: "turso_core::storage::pager", "dependency debug");
                tracing::info!(target: "turso_core", "dependency info");
                tracing::warn!(target: "turso_core", "dependency warning");
            });
            let output = std::fs::read_to_string(file.path()).unwrap();
            assert!(output.contains("application debug"));
            assert!(output.contains("dependency info"));
            assert!(output.contains("dependency warning"));
            assert_eq!(output.contains("dependency debug"), dependency_debug);
        }
    }

    #[test]
    fn defaults_and_validation() {
        let config = LoggingConfig::parse(None, None, None, None).unwrap();
        assert!(config.console);
        assert!(config.file.is_none());
        assert_eq!(config.filter.to_string(), "info");
        assert!(LoggingConfig::parse(Some("invalid"), None, None, None).is_err());
        assert!(LoggingConfig::parse(None, None, Some("false"), None).is_err());
        assert!(LoggingConfig::parse(None, None, Some("yes"), None).is_err());
        assert!(LoggingConfig::parse(None, None, Some("false"), Some("app.log")).is_ok());
        assert_eq!(
            LoggingConfig::parse(Some("WARN"), Some("debug"), None, None)
                .unwrap()
                .filter
                .to_string(),
            "debug"
        );
    }

    #[test]
    fn file_output_filters_all_five_levels_and_appends() {
        for (level, expected) in [
            ("ERROR", 1),
            ("WARN", 2),
            ("INFO", 3),
            ("DEBUG", 4),
            ("TRACE", 5),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("app.log");
            std::fs::write(&path, "existing\n").unwrap();
            let file = OpenOptions::new().append(true).open(&path).unwrap();
            let config = LoggingConfig::parse(Some(level), None, None, None).unwrap();
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(config.filter)
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                tracing::error!("error event");
                tracing::warn!("warn event");
                tracing::info!("info event");
                tracing::debug!("debug event");
                tracing::trace!("trace event");
            });
            let output = std::fs::read_to_string(path).unwrap();
            assert!(output.starts_with("existing\n"));
            assert_eq!(output.lines().count(), expected + 1, "{level}: {output}");
            assert!(!output.contains('\u{1b}'));
        }
    }
}
