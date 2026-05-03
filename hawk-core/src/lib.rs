// hawk-core: central kernel — agent lifecycle, permissions, orchestration

pub mod air_gap;
pub mod agent_manager;
pub mod config;
pub mod config_engine;
pub mod daemon;
pub mod db;
pub mod error;
pub mod llm_router;
pub mod manifest;
pub mod orchestrator;
pub mod pattern_detector;
pub mod permission_guard;
pub mod platform;
pub mod resource_monitor;
pub mod self_healer;
pub mod session_recorder;
pub mod talon;
pub mod token_tracker;
pub mod types;

#[cfg(test)]
mod tests;
