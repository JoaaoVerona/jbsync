//! The pluggable editor interface. Each supported editor/IDE family lives in its
//! own crate and implements [`Editor`]; the `idesync` binary holds a list of them
//! and dispatches by CLI key. Adding a new editor is a new crate implementing
//! this trait plus one line in the binary's registry.

use anyhow::Result;
use clap::{ArgMatches, Command};
use std::path::PathBuf;

/// One discovered editor/IDE install, surfaced by `idesync list`.
pub struct Discovered {
	/// Short label for this instance, e.g. "IntelliJIdea 2026.1" or "Code".
	pub label: String,
	/// Where its settings live on disk.
	pub path: PathBuf,
}

impl Discovered {
	pub fn new(label: impl Into<String>, path: PathBuf) -> Self {
		Discovered {
			label: label.into(),
			path,
		}
	}
}

/// A pluggable editor family (JetBrains, VSCode, …). Each owns its CLI subtree,
/// config format, and apply/check/create execution end to end.
pub trait Editor {
	/// CLI namespace key, e.g. `"jb"` or `"vsc"`. Must be unique and match the
	/// name of the [`command`](Editor::command) this editor returns.
	fn key(&self) -> &'static str;

	/// Human-readable family name, e.g. `"JetBrains"` or `"VSCode"`.
	fn name(&self) -> &'static str;

	/// Installs discovered on this machine, for `idesync list`.
	fn discover(&self) -> Vec<Discovered>;

	/// This editor's clap subcommand — its `key` as the command name, with
	/// `apply` / `check` / `create` (and any editor-specific extras) under it.
	fn command(&self) -> Command;

	/// Execute after the user selected this editor's subcommand. `matches` is the
	/// `ArgMatches` for this editor's `command()`. Returns the process exit code.
	fn run(&self, matches: &ArgMatches) -> Result<i32>;
}
