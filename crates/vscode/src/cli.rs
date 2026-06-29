//! The `vsc` CLI namespace: `idesync vsc apply|check|create`. Pass-through sync
//! of VSCode-family `settings.json` / `keybindings.json` / extensions.

use crate::config::{VsCodeCfg, VsCodeExtensionsCfg};
use crate::sync::{self, ExtensionInstall, Family};
use anyhow::{anyhow, bail, Context, Result};
use clap::{ArgMatches, Args, FromArgMatches, Subcommand};
use idesync_core::prompt;
use idesync_core::runner::{print_diff, write_change};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

/// `$schema` for generated configs: the schema attached to the latest GitHub
/// release (uploaded by the release workflow via `.publisher.json` extra-assets).
const SCHEMA_URL: &str = "https://github.com/JoaaoVerona/idesync/releases/latest/download/idesync-vscode.schema.json";

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
	/// Path to the JSON config. Omit (on a terminal) to be prompted.
	config: Option<PathBuf>,
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
	/// Prompt for every option interactively (implied when no config is given).
	#[arg(short, long)]
	interactive: bool,
}

#[derive(Args)]
struct CheckArgs {
	/// Path to the JSON config. Omit (on a terminal) to be prompted.
	config: Option<PathBuf>,
	#[arg(long)]
	product: Option<String>,
	#[arg(long)]
	targets_only: bool,
	/// Prompt for every option interactively (implied when no config is given).
	#[arg(short, long)]
	interactive: bool,
}

#[derive(Args)]
struct CreateArgs {
	/// Output directory for idesync.json. Omit (on a terminal) to be prompted.
	#[arg(long)]
	out: Option<PathBuf>,
	/// Restrict to these editors (repeatable); default: every discovered editor.
	#[arg(long = "product")]
	products: Vec<String>,
	/// Fold the host's primary modifier in captured bindings into the
	/// platform-relative `mod` token (Ctrl on Linux/Windows, Cmd on macOS), so
	/// they follow the OS on apply. Literal `alt`/non-primary modifiers stay put.
	#[arg(long)]
	portable_keymap: bool,
	/// Prompt for every option interactively (implied when no --out is given).
	#[arg(short, long)]
	interactive: bool,
}

// --- interactive wizards -----------------------------------------------------

/// See `idesync_jetbrains`'s equivalent: prompt only when forced or when a
/// required input is missing AND we're on a terminal (never hangs in CI).
fn want_interactive(forced: bool, missing: bool) -> Result<bool> {
	if forced && !prompt::is_interactive() {
		bail!("--interactive requires a terminal (stdin/stdout are not a TTY)");
	}
	Ok(forced || (missing && prompt::is_interactive()))
}

fn prompt_config(current: &Option<PathBuf>) -> Result<PathBuf> {
	let value = match current.as_ref().map(|p| p.display().to_string()) {
		Some(def) => prompt::text_default("Config path", &def)?,
		None => prompt::text("Config path")?,
	};
	Ok(PathBuf::from(value))
}

fn prompt_product(current: &Option<String>) -> Result<Option<String>> {
	let keys = sync::family_keys();
	let mut items = vec!["All discovered / config targets".to_string()];
	items.extend(keys.iter().map(|s| s.to_string()));
	let default = current
		.as_ref()
		.and_then(|c| keys.iter().position(|k| k == c))
		.map(|i| i + 1)
		.unwrap_or(0);
	let idx = prompt::select("Editor", &items, default)?;
	Ok((idx > 0).then(|| items[idx].clone()))
}

fn wizard_apply(a: &mut ApplyArgs) -> Result<()> {
	a.config = Some(prompt_config(&a.config)?);
	a.product = prompt_product(&a.product)?;
	a.targets_only = prompt::confirm("Only editors listed in `targets`?", a.targets_only)?;
	a.dry_run = prompt::confirm("Dry run (show diffs, write nothing)?", a.dry_run)?;
	a.no_backup = prompt::confirm("Skip backups of overwritten files?", a.no_backup)?;
	Ok(())
}

fn wizard_check(a: &mut CheckArgs) -> Result<()> {
	a.config = Some(prompt_config(&a.config)?);
	a.product = prompt_product(&a.product)?;
	a.targets_only = prompt::confirm("Only editors listed in `targets`?", a.targets_only)?;
	Ok(())
}

fn wizard_create(a: &mut CreateArgs) -> Result<()> {
	let def = a
		.out
		.as_ref()
		.map(|p| p.display().to_string())
		.unwrap_or_else(|| "./vscode".to_string());
	a.out = Some(PathBuf::from(prompt::text_default("Output directory", &def)?));
	let keys: Vec<String> = sync::family_keys().iter().map(|s| s.to_string()).collect();
	let chosen = prompt::multiselect("Editors to snapshot (none = all discovered)", &keys)?;
	a.products = chosen.iter().map(|&i| keys[i].clone()).collect();
	a.portable_keymap = prompt::confirm("Portable keymap (fold ctrl/cmd into mod)?", a.portable_keymap)?;
	Ok(())
}

// --- command handlers --------------------------------------------------------

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

fn cmd_apply(mut a: ApplyArgs) -> Result<i32> {
	if want_interactive(a.interactive, a.config.is_none())? {
		wizard_apply(&mut a)?;
	}
	let config = a
		.config
		.clone()
		.ok_or_else(|| anyhow!("config path required (pass it, or run on a terminal for the wizard)"))?;
	let cfg = VsCodeCfg::load(&config)?;
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

fn cmd_check(mut a: CheckArgs) -> Result<i32> {
	if want_interactive(a.interactive, a.config.is_none())? {
		wizard_check(&mut a)?;
	}
	let config = a
		.config
		.clone()
		.ok_or_else(|| anyhow!("config path required (pass it, or run on a terminal for the wizard)"))?;
	let cfg = VsCodeCfg::load(&config)?;
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
fn cmd_create(mut a: CreateArgs) -> Result<i32> {
	use serde_json::Value;
	if want_interactive(a.interactive, a.out.is_none())? {
		wizard_create(&mut a)?;
	}
	let out = a
		.out
		.clone()
		.ok_or_else(|| anyhow!("--out required (pass it, or run on a terminal for the wizard)"))?;
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
						// `--portable-keymap`: fold the host primary modifier into `mod`.
						keybindings = Some(if a.portable_keymap {
							crate::keymap::collapse(&arr, crate::keymap::host_primary())
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
		schema: Some(SCHEMA_URL.to_string()),
		targets: vec![],
		settings,
		keybindings,
		extensions,
	};

	std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
	let json = serde_json::to_string_pretty(&cfg)? + "\n";
	let cfg_path = out.join("idesync.json");
	std::fs::write(&cfg_path, json).with_context(|| format!("writing {}", cfg_path.display()))?;
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
