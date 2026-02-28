//! Event adapter implementations

pub mod app_server_v2;
pub mod exec_json;
pub mod rollout_replay;

mod r#trait;

pub use r#trait::{AdapterError, EventAdapter};

pub use app_server_v2::AppServerV2Adapter;
pub use exec_json::ExecJsonAdapter;
pub use rollout_replay::RolloutReplayAdapter;
