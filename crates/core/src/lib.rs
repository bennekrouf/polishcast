//! Project model: what was recorded and how it should look. No capture, no GPU, no UI here,
//! so every decision about the picture is a pure function that tests can pin down.
//!
//! Coordinates are normalized to the captured display: (0, 0) is its top-left corner and
//! (1, 1) its bottom-right, whatever its resolution. Times are seconds from the take's start.

pub mod blanks;
pub mod camera;
pub mod clips;
pub mod cursor;
pub mod events;
pub mod history;
pub mod license;
pub mod project;
pub mod zoom;

pub use blanks::{BlankParams, Cut, Loudness, Timeline};
pub use camera::Viewport;
pub use events::{Button, Click, CursorSample, EventLog};
pub use project::{Project, Style};
pub use zoom::Zoom;

/// Files inside a take's folder.
pub mod take {
    /// The screen recording, cursor hidden, with the microphone's sound.
    pub const SCREEN: &str = "screen.mov";
    /// Pointer movements and clicks (`EventLog` as JSON).
    pub const EVENTS: &str = "events.json";
    /// The user's edits (`Project` as JSON).
    pub const PROJECT: &str = "project.json";
    /// The sound's loudness over time (`Loudness` as JSON), measured once for blank cutting.
    pub const LOUDNESS: &str = "loudness.json";
}
