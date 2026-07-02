//! `idesync-vscode` — the VSCode-family editor plugin for idesync.
//!
//! Pass-through sync of `settings.json` (surgically merged), `keybindings.json`
//! (owned wholesale), and extensions (ensure-installed via the editor CLI) across
//! VS Code, Insiders, VSCodium, Cursor, and Windsurf. Exposes the [`Editor`]
//! implementation the `idesync` binary registers under the `vsc` CLI namespace.

mod cli;
mod config;
mod jsonc;
mod keymap;
mod sync;
mod vsix;

use anyhow::Result;
use clap::{ArgMatches, Command};
use idesync_core::{Discovered, Editor};

/// The VSCode editor plugin. Construct with [`editor`].
pub struct VsCode;

/// The VSCode editor plugin instance, for the binary's registry.
pub fn editor() -> VsCode {
	VsCode
}

impl Editor for VsCode {
	fn key(&self) -> &'static str {
		"vsc"
	}

	fn name(&self) -> &'static str {
		"VSCode"
	}

	fn discover(&self) -> Vec<Discovered> {
		sync::discover()
			.into_iter()
			.map(|fam| Discovered::new(fam.key, sync::user_dir(fam).unwrap_or_default()))
			.collect()
	}

	fn command(&self) -> Command {
		cli::command()
	}

	fn run(&self, matches: &ArgMatches) -> Result<i32> {
		cli::dispatch(matches)
	}
}
