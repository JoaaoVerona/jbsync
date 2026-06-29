//! The `vsc` CLI namespace: `idesync vsc apply|check|create`. Pass-through sync
//! of VSCode-family `settings.json` / `keybindings.json` / extensions.

use crate::config::{VsCodeCfg, VsCodeExtensionsCfg};
use crate::sync::{self, ExtensionInstall, Family};
use anyhow::{anyhow, bail, Context, Result};
use clap::{ArgMatches, Args, FromArgMatches, Subcommand};
use idesync_core::runner::{print_diff, write_change};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

const SCHEMA_JSON: &str = include_str!("../schema/idesync-vscode.schema.json");

/// The subcommands under `idesync vsc`.
#[derive(Subcommand)]
enum VscCmd {
	/// Apply the config to the target editors.
	Apply(ApplyArgs),
	/// Report whether the editors match the config (exit 1 if drift).
	Check(CheckArgs),
	/// Snapshot current editor settings into a portable config.
	Create(CreateArgs),
}

/// Build the `vsc` clap subcommand (its operations augmented onto it).
pub fn command() -> clap::Command {
	// `about` is set AFTER augment so it isn't overwritten by the enum doc.
	VscCmd::augment_subcommands(clap::Command::new("vsc"))
		.about("VSCode-family editors (VS Code, Insiders, VSCodium, Cursor, Windsurf)")
		.subcommand_required(true)
		.arg_required_else_help(true)
}

/// Dispatch a parsed `vsc` subcommand.
pub fn dispatch(matches: &ArgMatches) -> Result<i32> {
	match VscCmd::from_arg_matches(matches).map_err(|e| anyhow!(e.to_string()))? {
		VscCmd::Apply(a) => cmd_apply(a),
		VscCmd::Check(a) => cmd_check(a),
		VscCmd::Create(a) => cmd_create(a),
	}
}

#[derive(Args)]
struct ApplyArgs {
	/// Path to the JSON config.
	config: PathBuf,
	/// Show diffs without writing.
	#[arg(long)]
	dry_run: bool,
	/// Do not back up overwritten files.
	#[arg(long)]
	no_backup: bool,
	/// Only this editor (by name, e.g. "Code", "Cursor").
	#[arg(long)]
	product: Option<String>,
	/// Only editors listed in the config's `targets` (no auto-discovery).
	#[arg(long)]
	targets_only: bool,
}

#[derive(Args)]
struct CheckArgs {
	config: PathBuf,
	#[arg(long)]
	product: Option<String>,
	#[arg(long)]
	targets_only: bool,
}

#[derive(Args)]
struct CreateArgs {
	/// Output directory for idesync.json.
	#[arg(long)]
	out: PathBuf,
	/// Restrict to these editors (repeatable); default: every discovered editor.
	#[arg(long = "product")]
	products: Vec<String>,
	/// Fold captured `ctrl` keys with a matching `cmd` macOS override into the
	/// platform-relative `mod` token, so the binding follows the OS on apply
	/// (Ctrl on Linux/Windows, Cmd on macOS).
	#[arg(long)]
	portable_keymap: bool,
}

/// Resolve the editors to act on, erroring if `--product` names an unknown
/// editor or nothing matches.
fn editors_for(cfg: &VsCodeCfg, product: &Option<String>, targets_only: bool) -> Result<Vec<&'static Family>> {
	if let Some(p) = product {
		if !sync::is_family(p) {
			bail!(
				"unknown VSCode editor '{p}' (known: {})",
				sync::family_keys().join(", ")
			);
		}
	}
	let editors = sync::resolve_editors(cfg, product.as_deref(), targets_only);
	if editors.is_empty() {
		bail!("no VSCode editors to act on (none discovered, or none match the config `targets`)");
	}
	Ok(editors)
}

fn cmd_apply(a: ApplyArgs) -> Result<i32> {
	let cfg = VsCodeCfg::load(&a.config)?;
	let editors = editors_for(&cfg, &a.product, a.targets_only)?;

	if !a.dry_run {
		println!("⚠  Make sure the target editor is fully closed — it overwrites config on exit.\n");
	}

	let mut total = 0usize;
	for fam in editors {
		let dir = sync::user_dir(fam)?;
		println!("● {}  [VSCode]  ({})", fam.key, dir.display());
		if !sync::is_installed(fam) {
			println!("  (editor not found — files will be created)");
		}
		let plan = sync::build_plan(&cfg, fam)?;
		if plan.is_empty() {
			println!("  already in sync\n");
			continue;
		}
		for ch in &plan.files {
			if a.dry_run {
				print_diff(ch);
			} else {
				write_change(ch, !a.no_backup)?;
				let tag = if ch.is_new() { "create" } else { "update" };
				println!("  {tag}  {}", ch.rel);
			}
		}
		for inst in &plan.installs {
			if a.dry_run {
				println!("  run  {}", inst.command_display());
			} else {
				run_ext_install(inst)?;
			}
		}
		total += plan.change_count();
		println!();
	}

	if a.dry_run {
		println!("dry-run: {total} item(s) would change");
	} else {
		println!("applied {total} change(s)");
	}
	Ok(0)
}

fn cmd_check(a: CheckArgs) -> Result<i32> {
	let cfg = VsCodeCfg::load(&a.config)?;
	let editors = editors_for(&cfg, &a.product, a.targets_only)?;

	let mut drift = 0usize;
	for fam in editors {
		let plan = sync::build_plan(&cfg, fam)?;
		let label = format!("{} (VSCode)", fam.key);
		if plan.is_empty() {
			println!("✓ {label}: in sync");
		} else {
			drift += plan.change_count();
			println!("✗ {label}: {} item(s) would change", plan.change_count());
			for ch in &plan.files {
				println!("    {} {}", if ch.is_new() { "+" } else { "~" }, ch.rel);
			}
			for inst in &plan.installs {
				println!("    + install {} extension(s): {}", inst.ids.len(), inst.ids.join(", "));
			}
		}
	}
	Ok(if drift == 0 { 0 } else { 1 })
}

/// Snapshot the discovered editors into a config: settings.json keys are unioned
/// (first editor wins on conflict), keybindings come from the first editor that
/// has any, and extensions are the union of installed IDs.
fn cmd_create(a: CreateArgs) -> Result<i32> {
	use serde_json::Value;
	let editors: Vec<_> = sync::discover()
		.into_iter()
		.filter(|f| a.products.is_empty() || a.products.iter().any(|p| p == f.key))
		.collect();
	if editors.is_empty() {
		bail!("no VSCode editors found to snapshot");
	}

	let mut settings = serde_json::Map::new();
	let mut keybindings: Option<Vec<Value>> = None;
	let mut install: BTreeSet<String> = BTreeSet::new();
	for fam in &editors {
		let dir = sync::user_dir(fam)?;
		if let Ok(text) = std::fs::read_to_string(dir.join("settings.json")) {
			if let Ok(Value::Object(map)) = crate::jsonc::parse(&text) {
				for (k, v) in map {
					settings.entry(k).or_insert(v);
				}
			}
		}
		if keybindings.is_none() {
			if let Ok(text) = std::fs::read_to_string(dir.join("keybindings.json")) {
				if let Ok(Value::Array(arr)) = crate::jsonc::parse(&text) {
					if !arr.is_empty() {
						// `--portable-keymap`: fold ctrl+cmd pairs back into `mod`.
						keybindings = Some(if a.portable_keymap {
							crate::keymap::collapse(&arr)
						} else {
							arr
						});
					}
				}
			}
		}
		install.extend(sync::installed_extension_ids(fam));
	}
	let extensions = (!install.is_empty()).then(|| VsCodeExtensionsCfg {
		install: install.into_iter().collect(),
	});
	let cfg = VsCodeCfg {
		schema: Some("./idesync-vscode.schema.json".to_string()),
		targets: vec![],
		settings,
		keybindings,
		extensions,
	};

	std::fs::create_dir_all(&a.out).with_context(|| format!("creating {}", a.out.display()))?;
	let json = serde_json::to_string_pretty(&cfg)? + "\n";
	let cfg_path = a.out.join("idesync.json");
	std::fs::write(&cfg_path, json).with_context(|| format!("writing {}", cfg_path.display()))?;
	std::fs::write(a.out.join("idesync-vscode.schema.json"), SCHEMA_JSON)?;
	println!(
		"Captured {} editor(s): {} setting(s), {} keybinding(s), {} extension(s)",
		editors.len(),
		cfg.settings.len(),
		cfg.keybindings.as_ref().map_or(0, Vec::len),
		cfg.extensions.as_ref().map_or(0, |e| e.install.len()),
	);
	println!("wrote {}", cfg_path.display());
	Ok(0)
}

fn run_ext_install(inst: &ExtensionInstall) -> Result<()> {
	let cli = inst.cli.as_ref().ok_or_else(|| {
		anyhow!(
			"cannot find the '{}' CLI for {} — add it to PATH to install extensions",
			inst.cli_name,
			inst.editor
		)
	})?;
	println!("  install {} extension(s):", inst.ids.len());
	println!("    {}", inst.command_display());
	println!("    (downloads from the editor's Marketplace — make sure the editor is closed)");
	let status = Command::new(cli)
		.args(inst.args())
		.status()
		.with_context(|| format!("running {}", cli.display()))?;
	if !status.success() {
		bail!("{} --install-extension exited with {status}", inst.cli_name);
	}
	let after = sync::installed_extensions(sync::family(&inst.editor).expect("known editor"));
	let still_missing: Vec<&str> = inst
		.ids
		.iter()
		.filter(|id| !after.contains(&id.to_ascii_lowercase()))
		.map(String::as_str)
		.collect();
	if !still_missing.is_empty() {
		println!("    ⚠ still missing after install: {}", still_missing.join(", "));
	}
	Ok(())
}
