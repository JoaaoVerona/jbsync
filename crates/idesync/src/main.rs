//! idesync — declaratively apply IDE/editor settings from per-editor JSON
//! configs, cross-platform.
//!
//! The binary is a thin shell: it holds the list of pluggable editors (each its
//! own crate implementing [`idesync_core::Editor`]), builds the CLI by attaching
//! each one's subcommand, and dispatches by key. Adding a new editor is a new
//! crate plus one line in [`editors`].

use anyhow::Result;
use idesync_core::Editor;

/// The registered editor plugins. Order here is the order shown by `list`.
fn editors() -> Vec<Box<dyn Editor>> {
	vec![
		Box::new(idesync_jetbrains::editor()),
		Box::new(idesync_vscode::editor()),
	]
}

fn main() {
	std::process::exit(run());
}

fn run() -> i32 {
	let editors = editors();
	let mut cli = clap::Command::new("idesync")
		.version(env!("CARGO_PKG_VERSION"))
		.about("Apply IDE/editor settings from per-editor JSON configs, cross-platform.")
		.subcommand_required(true)
		.arg_required_else_help(true)
		.subcommand(clap::Command::new("list").about("List installed editors/IDEs discovered on this machine."));
	for e in &editors {
		cli = cli.subcommand(e.command());
	}

	let matches = cli.get_matches();
	let result: Result<i32> = match matches.subcommand() {
		Some(("list", _)) => cmd_list(&editors),
		Some((key, sub)) => match editors.iter().find(|e| e.key() == key) {
			Some(e) => e.run(sub),
			None => {
				eprintln!("error: unknown command '{key}'");
				Ok(2)
			}
		},
		None => Ok(0),
	};

	match result {
		Ok(code) => code,
		Err(e) => {
			eprintln!("error: {e:#}");
			1
		}
	}
}

/// Discover and list installed editors/IDEs across every registered plugin,
/// grouped by editor family.
fn cmd_list(editors: &[Box<dyn Editor>]) -> Result<i32> {
	let mut any = false;
	for e in editors {
		let found = e.discover();
		if found.is_empty() {
			continue;
		}
		if any {
			println!();
		}
		any = true;
		println!("{}:", e.name());
		for d in found {
			println!("  {}  ({})", d.label, d.path.display());
		}
	}
	if !any {
		println!("no editors/IDEs found");
	}
	Ok(0)
}
