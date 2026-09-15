// pub mod app;
#[cfg(test)]
extern crate self as incident_clustering_analyzer;
#[cfg(test)]
mod acceptance_tests;
pub mod artifacts;
pub mod auth;
pub mod backup;
pub mod clustering;
pub mod config;
#[cfg(test)]
#[path = "test_fixtures.rs"]
pub(crate) mod fixtures;
pub mod io;
pub mod jobs;
pub mod labels;
pub mod model;
pub mod progress;
pub mod schema;
pub mod session;
pub mod storage;
pub mod text;
pub mod web;
pub mod worker;
pub mod workflow;
