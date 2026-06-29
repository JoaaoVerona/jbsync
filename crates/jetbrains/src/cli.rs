//! The `jb` CLI namespace: `idesync jb apply|check|create|keymap`. Builds the
//! plan for each JetBrains target and applies / diffs / reports it via the shared
//! [`idesync_core::runner`].

use crate::appliers::{self, keymap, Ctx, Section};
use crate::config::{Config, PluginsCfg, Target};
use crate::discovery;
use crate::plan::PluginInstall;
use anyhow::{anyhow, bail, Context, Result};
use clap::{ArgMatches, Args, FromArgMatches, Subcommand};
use idesync_core::runner::{print_diff, write_change};
use idesync_core::Os;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The subcommands under `idesync jb`.
#[derive(Subcommand)]
enum JbCmd {
	/// Apply the config to the target IDEs.
	Apply(ApplyArgs),
	/// Report whether the IDEs match the config (exit 1 if drift).
	Check(CheckArgs),
	/// Snapshot current IDE settings into a portable config + merged scheme files.
	Create(CreateArgs),
	/// Generate per-OS keymaps to a directory (for committing / other machines).
	Keymap(KeymapArgs),
}

/// Build the `jb` clap subcommand (its operations augmented onto it).
pub fn command() -> clap::Command {
	// `about` is set AFTER augment so it isn't overwritten by the enum doc.
	JbCmd::augment_subcommands(clap::Command::new("jb"))
		.about("JetBrains IDEs (IntelliJ IDEA, WebStorm, RustRover, … and Android Studio)")
		.subcommand_required(true)
		.arg_required_else_help(true)
}

/// Dispatch a parsed `jb` subcommand.
pub fn dispatch(matches: &ArgMatches) -> Result<i32> {
	match JbCmd::from_arg_matches(matches).map_err(|e| anyhow!(e.to_string()))? {
		JbCmd::Apply(a) => cmd_apply(a),
		JbCmd::Check(a) => cmd_check(a),
		JbCmd::Create(a) => cmd_create(a),
		JbCmd::Keymap(a) => cmd_keymap(a),
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
	/// Override target product (otherwise uses config `targets`).
	#[arg(long)]
	product: Option<String>,
	/// Override target version.
	#[arg(long)]
	version: Option<String>,
	/// Override target OS (default: host). One of linux|macos|windows.
	#[arg(long)]
	os: Option<String>,
	/// Skip a config section (repeatable), e.g. `--exclude plugins --exclude
	/// keymap`. Excluded sections are left untouched.
	#[arg(long, value_enum)]
	exclude: Vec<Section>,
	/// Only touch IDEs listed in the config's `targets`. By default, shared
	/// settings (font, keymap, schemes, …) are ALSO applied to any other IDE
	/// found on this machine — even ones absent from the config.
	#[arg(long)]
	targets_only: bool,
}

#[derive(Args)]
struct CheckArgs {
	config: PathBuf,
	#[arg(long)]
	product: Option<String>,
	#[arg(long)]
	version: Option<String>,
	#[arg(long)]
	os: Option<String>,
	/// Skip a config section (repeatable), e.g. `--exclude plugins --exclude
	/// keymap`. Excluded sections are not reported as drift.
	#[arg(long, value_enum)]
	exclude: Vec<Section>,
	/// Only check IDEs listed in the config's `targets` (see `apply --targets-only`).
	#[arg(long)]
	targets_only: bool,
}

#[derive(Args)]
struct KeymapArgs {
	config: PathBuf,
	/// Output directory; keymaps are written under `<out>/keymaps/`.
	#[arg(long)]
	out: PathBuf,
}

#[derive(Args)]
struct CreateArgs {
	/// Output directory for idesync.json + merged scheme files.
	#[arg(long)]
	out: PathBuf,
	/// Restrict to these products (repeatable); default: every discovered IDE.
	#[arg(long = "product")]
	products: Vec<String>,
	/// Product whose single-valued settings win (required when >1 IDE; no default).
	#[arg(long)]
	primary: Option<String>,
	/// Emit keymap shortcuts with the platform-relative `mod` modifier instead of
	/// literal Ctrl/Cmd, so they follow the OS on apply (Ctrl on Linux/Windows,
	/// Cmd on macOS). Applies to mouse shortcuts too (Ctrl-click → Cmd-click).
	#[arg(long)]
	portable_keymap: bool,
}

fn config_dir_of(path: &Path) -> PathBuf {
	path.parent()
		.map(Path::to_path_buf)
		.unwrap_or_else(|| PathBuf::from("."))
}

fn target_os(over: &Option<String>) -> Result<Os> {
	match over {
		Some(s) => Os::parse(s).ok_or_else(|| anyhow!("unknown --os '{s}'")),
		None => Ok(Os::host()),
	}
}

fn resolve_targets(
	cfg: &Config,
	product: &Option<String>,
	version: &Option<String>,
	targets_only: bool,
) -> Result<Vec<Target>> {
	if let Some(p) = product {
		// CLI override of product/version uses the matching config target's
		// plugin + per-target file overrides if one exists, so `--product X`
		// still respects them.
		let matching = cfg.targets.iter().find(|t| &t.product == p);
		return Ok(vec![Target {
			product: p.clone(),
			version: version.clone(),
			plugins: matching.and_then(|t| t.plugins.clone()),
			files: matching.map(|t| t.files.clone()).unwrap_or_default(),
		}]);
	}
	let mut targets = cfg.targets.clone();
	if !targets_only {
		let discovered = discovery::discover_all().unwrap_or_default();
		targets = extend_with_discovered(targets, &discovered);
	}
	if targets.is_empty() {
		bail!("no targets: add a `targets` array to the config or pass --product");
	}
	Ok(targets)
}

/// Append a SHARED-only target for every discovered IDE not already configured
/// (e.g. Rider captured on another machine): no per-target plugins/files, so it
/// receives only the global settings (font/keymap/schemes). Latest installed
/// version (`None`). Configured targets are kept as-is and win on product name.
fn extend_with_discovered(mut targets: Vec<Target>, discovered: &[(String, String, PathBuf)]) -> Vec<Target> {
	for (product, _ver, _path) in discovered {
		if !targets.iter().any(|t| &t.product == product) {
			targets.push(Target {
				product: product.clone(),
				version: None,
				plugins: None,
				files: vec![],
			});
		}
	}
	targets
}

/// Products explicitly listed in the config (vs. discovered shared-only IDEs).
fn configured_products(cfg: &Config) -> std::collections::HashSet<String> {
	cfg.targets.iter().map(|t| t.product.clone()).collect()
}

fn build_ctx(cfg: &Config, cfg_path: &Path, target: &Target, os: Os) -> Result<(Ctx, bool)> {
	let ide_dir = discovery::resolve_ide_dir(&target.product, target.version.as_deref())?;
	let exists = ide_dir.exists();
	// The plugin install dir (data dir) uses the same "<Product><Version>" folder name.
	let dir_name = ide_dir.file_name().map(|n| n.to_owned()).unwrap_or_default();
	let install_dir = discovery::data_base(&target.product)?.join(&dir_name);
	Ok((
		Ctx {
			ide_dir,
			install_dir,
			product: target.product.clone(),
			target_os: os,
			config_dir: config_dir_of(cfg_path),
			plugins: PluginsCfg::effective(cfg.plugins.as_ref(), target.plugins.as_ref()),
			files: target.files.clone(),
		},
		exists,
	))
}

fn cmd_apply(a: ApplyArgs) -> Result<i32> {
	let cfg = Config::load(&a.config)?;
	let os = target_os(&a.os)?;
	let targets = resolve_targets(&cfg, &a.product, &a.version, a.targets_only)?;
	let configured = configured_products(&cfg);

	if !a.dry_run {
		println!("⚠  Make sure the target IDE is fully closed — it overwrites config on exit.\n");
	}

	let mut total = 0usize;
	for target in &targets {
		let (ctx, exists) = build_ctx(&cfg, &a.config, target, os)?;
		let label = format!("{}{}", target.product, target.version.as_deref().unwrap_or(""));
		println!("● {label}  [{}]  ({})", ctx.target_os, ctx.ide_dir.display());
		if !configured.contains(&target.product) {
			println!("  (not in config — applying shared settings only)");
		}
		if !exists {
			println!("  (config dir does not exist yet — it will be created)");
		}
		let plan = appliers::build_plan(&cfg, &ctx, &a.exclude)?;
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
				run_install(inst)?;
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
	let cfg = Config::load(&a.config)?;
	let os = target_os(&a.os)?;
	let targets = resolve_targets(&cfg, &a.product, &a.version, a.targets_only)?;
	let configured = configured_products(&cfg);

	let mut drift = 0usize;
	for target in &targets {
		let (ctx, _) = build_ctx(&cfg, &a.config, target, os)?;
		let suffix = if configured.contains(&target.product) {
			""
		} else {
			" (shared only)"
		};
		let label = format!("{}{}{suffix}", target.product, target.version.as_deref().unwrap_or(""));
		let plan = appliers::build_plan(&cfg, &ctx, &a.exclude)?;
		if plan.is_empty() {
			println!("✓ {label}: in sync");
		} else {
			drift += plan.change_count();
			println!("✗ {label}: {} item(s) would change", plan.change_count());
			for ch in &plan.files {
				println!("    {} {}", if ch.is_new() { "+" } else { "~" }, ch.rel);
			}
			for inst in &plan.installs {
				println!("    + install {} plugin(s): {}", inst.ids.len(), inst.ids.join(", "));
			}
		}
	}
	Ok(if drift == 0 { 0 } else { 1 })
}

fn cmd_create(a: CreateArgs) -> Result<i32> {
	crate::extract::create(&crate::extract::CreateOptions {
		out_dir: a.out,
		products: a.products,
		primary: a.primary,
		portable_keymap: a.portable_keymap,
	})?;
	Ok(0)
}

fn cmd_keymap(a: KeymapArgs) -> Result<i32> {
	let cfg = Config::load(&a.config)?;
	let km = cfg
		.keymap
		.as_ref()
		.ok_or_else(|| anyhow!("config has no `keymap` section"))?;
	let dir = a.out.join("keymaps");
	std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
	for os in Os::ALL {
		let name = keymap::keymap_name(&km.name, os);
		let file = dir.join(keymap::keymap_filename(&name));
		let xml = keymap::generate(km, os);
		std::fs::write(&file, xml).with_context(|| format!("writing {}", file.display()))?;
		println!("wrote {}", file.display());
	}
	Ok(0)
}

fn run_install(inst: &PluginInstall) -> Result<()> {
	let launcher = inst.launcher.as_ref().ok_or_else(|| {
		anyhow!(
			"cannot find the IDE launcher for {} — add it to PATH or set IDESYNC_JB_LAUNCHER",
			inst.product
		)
	})?;
	println!("  install {} plugin(s):", inst.ids.len());
	println!("    {}", inst.command_display());
	println!("    (this launches the IDE headless and downloads from Marketplace — make sure it is closed)");
	let status = Command::new(launcher)
		.args(inst.args())
		.status()
		.with_context(|| format!("running {}", launcher.display()))?;
	if !status.success() {
		bail!("installPlugins exited with {status}");
	}
	let after = appliers::plugins::installed_ids(&inst.install_dir);
	let still_missing: Vec<&str> = inst
		.ids
		.iter()
		.filter(|id| !after.contains(id.as_str()))
		.map(String::as_str)
		.collect();
	if !still_missing.is_empty() {
		println!("    ⚠ still missing after install: {}", still_missing.join(", "));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn target(product: &str, version: Option<&str>, files: Vec<String>) -> Target {
		Target {
			product: product.to_string(),
			version: version.map(str::to_string),
			plugins: None,
			files,
		}
	}

	fn disco(product: &str, version: &str) -> (String, String, PathBuf) {
		(product.to_string(), version.to_string(), PathBuf::from("/x"))
	}

	#[test]
	fn discovered_ides_become_shared_only_targets_without_clobbering_configured() {
		// IntelliJIdea is configured (with per-target files); the rest are only
		// discovered on this machine.
		let configured = vec![target("IntelliJIdea", Some("2026.1"), vec!["options/x.xml".into()])];
		let discovered = [
			disco("IntelliJIdea", "2026.1"), // already configured → must not duplicate
			disco("RustRover", "2026.1"),
			disco("RustRover", "2025.3"), // second version of same product → no dup
			disco("WebStorm", "2026.1"),
		];

		let merged = extend_with_discovered(configured, &discovered);
		let products: Vec<&str> = merged.iter().map(|t| t.product.as_str()).collect();
		assert_eq!(products, ["IntelliJIdea", "RustRover", "WebStorm"]);

		// Configured target keeps its version + per-target files.
		let ij = &merged[0];
		assert_eq!(ij.version.as_deref(), Some("2026.1"));
		assert_eq!(ij.files, vec!["options/x.xml".to_string()]);

		// Discovered ones are shared-only: latest version (None), no files.
		let rr = merged.iter().find(|t| t.product == "RustRover").unwrap();
		assert!(rr.version.is_none() && rr.files.is_empty() && rr.plugins.is_none());
	}
}
