//! `idesync-core` — the shared substrate for idesync's pluggable editor support.
//!
//! It defines the [`Editor`] plugin trait that each editor/IDE family crate
//! implements, the [`FileChange`] unit of work they all produce, the [`Os`]
//! abstraction, and the [`runner`] that applies/diffs changes uniformly. It
//! knows nothing about any specific editor.

pub mod change;
pub mod editor;
pub mod platform;
pub mod prompt;
pub mod runner;

pub use change::FileChange;
pub use editor::{Discovered, Editor};
pub use platform::Os;
