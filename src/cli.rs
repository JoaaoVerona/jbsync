//! Command-line surface: `apply`, `check`, `list`, `keymap`.

use crate::appliers::{self, keymap, Ctx, Section};
use crate::config::{Config, PluginsCfg, Target};
use crate::discovery;
use crate::plan::{FileChange, PluginInstall};
use crate::platform::Os;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(
	name = "jbsync",
	version,
	about = "Apply JetBrains IDE settings from one JSON config, cross-platform."
)]
struct Cli {
	#[command(subcommand)]
	cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
	/// Apply the config to the target IDEs.
	Apply(ApplyArgs),
	/// Report whether the IDEs match the config (exit 1 if drift).
	Check(CheckArgs),
	/// Snapshot current IDE settings into a portable config + merged scheme files.
	Create(CreateArgs),
	/// List JetBrains IDE config dirs discovered on this machine.
	List,
	/// Generate per-OS keymaps to a directory (for committing / other machines).
	Keymap(KeymapArgs),
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
	/// Output directory for jbsync.json + merged scheme files.
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
	/// Cmd on macOS). Mouse shortcuts stay literal.
	#[arg(long)]
	portable_keymap: bool,
}

pub fn run() -> i32 {
	let cli = Cli::parse();
	let result = match cli.cmd {
		Cmd::Apply(a) => cmd_apply(a),
		Cmd::Check(a) => cmd_check(a),
		Cmd::Create(a) => cmd_create(a),
		Cmd::List => cmd_list(),
		Cmd::Keymap(a) => cmd_keymap(a),
	};
	match result {
		Ok(code) => code,
		Err(e) => {
			eprintln!("error: {e:#}");
			1
		}
	}
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

fn resolve_targets(cfg: &Config, product: &Option<String>, version: &Option<String>) -> Result<Vec<Target>> {
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
	if cfg.targets.is_empty() {
		bail!("no targets: add a `targets` array to the config or pass --product");
	}
	Ok(cfg.targets.clone())
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
	let targets = resolve_targets(&cfg, &a.product, &a.version)?;

	if !a.dry_run {
		println!("⚠  Make sure the target IDE is fully closed — it overwrites config on exit.\n");
	}

	let mut total = 0usize;
	for target in &targets {
		let (ctx, exists) = build_ctx(&cfg, &a.config, target, os)?;
		let label = format!("{}{}", target.product, target.version.as_deref().unwrap_or(""));
		println!("● {label}  [{}]  ({})", ctx.target_os, ctx.ide_dir.display());
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
	let targets = resolve_targets(&cfg, &a.product, &a.version)?;

	let mut drift = 0usize;
	for target in &targets {
		let (ctx, _) = build_ctx(&cfg, &a.config, target, os)?;
		let label = format!("{}{}", target.product, target.version.as_deref().unwrap_or(""));
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

fn cmd_list() -> Result<i32> {
	let found = discovery::discover_all()?;
	if found.is_empty() {
		println!("no IDEs found");
		return Ok(0);
	}
	println!("Discovered IDEs:");
	for (product, version, path) in found {
		println!("  {product}{version}  ({})", path.display());
	}
	Ok(0)
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

// --- write helpers ----------------------------------------------------------

fn print_diff(ch: &FileChange) {
	use similar::{ChangeTag, TextDiff};
	let old = ch.old.as_deref().unwrap_or("");
	println!("  ── {} {}", ch.rel, if ch.is_new() { "(new)" } else { "" });
	let diff = TextDiff::from_lines(old, &ch.new);
	for change in diff.iter_all_changes() {
		let sign = match change.tag() {
			ChangeTag::Delete => "-",
			ChangeTag::Insert => "+",
			ChangeTag::Equal => continue,
		};
		print!("    {sign} {}", change.value());
		if !change.value().ends_with('\n') {
			println!();
		}
	}
}

fn run_install(inst: &PluginInstall) -> Result<()> {
	let launcher = inst.launcher.as_ref().ok_or_else(|| {
		anyhow!(
			"cannot find the IDE launcher for {} — add it to PATH or set JBSYNC_LAUNCHER",
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

fn write_change(ch: &FileChange, backup: bool) -> Result<()> {
	if let Some(parent) = ch.path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
	}
	if backup {
		if let Some(old) = &ch.old {
			backup_file(&ch.path, old, &ch.rel)?;
		}
	}
	atomic_write(&ch.path, &ch.new)
}

fn backup_file(path: &Path, content: &str, rel: &str) -> Result<()> {
	// <ide-dir>/.jbsync-backups/<unix-secs>/<rel>
	let ide_dir = backup_root(path, rel);
	let ts = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let dest = ide_dir.join(".jbsync-backups").join(ts.to_string()).join(rel);
	if let Some(parent) = dest.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(&dest, content).with_context(|| format!("backing up to {}", dest.display()))?;
	Ok(())
}

/// Recover the IDE config dir by stripping the relative path from the full path.
fn backup_root(path: &Path, rel: &str) -> PathBuf {
	let depth = Path::new(rel).components().count();
	let mut p = path.to_path_buf();
	for _ in 0..depth {
		p.pop();
	}
	p
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
	let tmp = path.with_extension("jbsync-tmp");
	std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
	std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
	Ok(())
}
