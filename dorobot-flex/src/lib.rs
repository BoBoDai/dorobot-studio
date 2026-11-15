//! DoRobot Flex - Library
//!
//! A LeRobot dataset viewer built with Makepad.
//!
//! This library provides:
//! - Flex layout with resizable sidebars and footer
//! - Dataset loading for LeRobot v2.0/v3.0 formats
//! - Video playback (with optional FFmpeg support)
//! - Time series visualization
//! - 3D robot viewer

pub use makepad_app_shell;
pub use makepad_widgets;

pub mod api;
pub mod data;
pub mod preferences;
pub mod runtime;
pub mod shared;
pub mod ui;
pub mod ux_probe;
pub mod widgets;

/// Still here because `ui`'s Play screen draws with it; the rest of the old
/// viewer's app modules went with its binary.
pub mod playback_controls;
