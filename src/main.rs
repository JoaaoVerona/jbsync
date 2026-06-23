//! jbapply — declaratively apply JetBrains IDE settings from one JSON config.
//!
//! The IDE writes its own config on exit and reads it on startup, so the IDE
//! must be closed when applying. `jbapply` makes the JSON config the single
//! source of truth and the IDE a read-only consumer — replacing (not
//! augmenting) JetBrains "Settings Sync".

mod appliers;
mod cli;
mod config;
mod discovery;
mod extract;
mod launcher;
mod plan;
mod platform;
mod scheme_merge;
mod settings;
mod xmlpatch;

fn main() {
	std::process::exit(cli::run());
}
