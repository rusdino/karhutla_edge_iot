//! Karhutla Rust host server library (embeddable by Dioxus Desktop or runnable standalone).

pub mod engine;
pub mod http;
pub mod mqtt;

pub use engine::{EngineConfig, KarhutlaEngine};
pub use http::{build_router, build_router_with_web_dir};
