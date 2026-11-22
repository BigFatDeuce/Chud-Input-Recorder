//! # chud-core
//!
//! Platform independent heart of Chud Input Recorder: the event model, the
//! editable timeline with undo/redo, smart editing tools, the `.chud` file
//! format, playback planning, the Flow automation interpreter and on-screen
//! image search.
//!
//! Nothing in this crate talks to the operating system, which keeps every
//! rule unit-testable on any platform.

pub mod document;
pub mod event;
pub mod expr;
pub mod flow;
pub mod format;
pub mod history;
pub mod keys;
pub mod ops;
pub mod playback;
pub mod stats;
pub mod timeline;
pub mod transform;
pub mod vision;

pub use document::MacroDoc;
pub use event::{Event, EventId, EventKind, Key, Micros, MouseButton, PadAxis, PadButton};
pub use timeline::{Change, Timeline};
