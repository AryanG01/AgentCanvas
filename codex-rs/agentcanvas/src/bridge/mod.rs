//! Bridge module: translates `NormalizedEvent` streams into UI `AppEvent` types.

mod app_event;
mod translator;

pub use app_event::AppEvent;
pub use translator::translate;
