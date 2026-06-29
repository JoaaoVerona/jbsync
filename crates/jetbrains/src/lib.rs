//! `idesync-jetbrains` — the JetBrains editor plugin for idesync.
//!
//! Surgically patches JetBrains `options/*.xml`, generates per-OS keymaps, merges
//! color schemes / code styles, and ensure-installs plugins — driven by one JSON
//! config. Exposes the [`Editor`] implementation the `idesync` binary registers
//! under the `jb` CLI namespace.

mod appliers;
mod cli;
mod config;
mod default_keymap;
mod discovery;
mod extract;
mod launcher;
mod plan;
mod scheme_merge;
mod settings;
mod xmlpatch;

use anyhow::Result;
use clap::{ArgMatches, Command};
use idesync_core::{Discovered, Editor};

/// The JetBrains editor plugin. Construct with [`editor`].
pub struct JetBrains;

/// The JetBrains editor plugin instance, for the binary's registry.
pub fn editor() -> JetBrains {
	JetBrains
}

impl Editor for JetBrains {
	fn key(&self) -> &'static str {
		"jb"
	}

	fn name(&self) -> &'static str {
		"JetBrains"
	}

	fn discover(&self) -> Vec<Discovered> {
		discovery::discover_all()
			.unwrap_or_default()
			.into_iter()
			.map(|(product, version, path)| Discovered::new(format!("{product}{version}"), path))
			.collect()
	}

	fn command(&self) -> Command {
		cli::command()
	}

	fn run(&self, matches: &ArgMatches) -> Result<i32> {
		cli::dispatch(matches)
	}
}
