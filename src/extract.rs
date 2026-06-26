//! `create`: snapshot the current IDE settings into a portable jbsync config
//! plus copied/merged scheme files. The reverse of `apply` — and strictly
//! read-only with respect to the IDEs (it only writes into the output dir).

use crate::appliers::plugins::installed_ids;
use crate::config::*;
use crate::discovery;
use crate::platform::Os;
use crate::scheme_merge;
use crate::xmlpatch::{get_attr, get_option};
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SCHEMA_JSON: &str = include_str!("../schema/jbsync.schema.json");

pub struct CreateOptions {
	pub out_dir: PathBuf,
	/// Restrict to these products; empty = every discovered IDE.
	pub products: Vec<String>,
	/// Product whose single-valued settings win. Required when >1 IDE; no default.
	pub primary: Option<String>,
	/// Emit the host's primary keyboard modifier as `mod` (so Ctrl→Cmd on macOS).
	pub portable_keymap: bool,
}

struct Ide {
	product: String,
	version: String,
	config_dir: PathBuf,
	install_dir: PathBuf,
}

pub fn create(opts: &CreateOptions) -> Result<()> {
	let mut ides = select_ides(&opts.products)?;
	if ides.is_empty() {
		bail!("no IDEs found to snapshot");
	}
	// The primary IDE provides all single-valued settings (fonts, toggles, active
	// scheme/style, heap, keymap) and wins conflicts in the cross-IDE scheme
	// merge. It must be chosen explicitly when there's more than one IDE — there
	// is no default.
	let primary_idx = resolve_primary(&ides, opts.primary.as_deref())?;
	ides.swap(0, primary_idx);
	let primary = &ides[0];
	println!(
		"Snapshotting {} IDE(s); primary = {}{}",
		ides.len(),
		primary.product,
		primary.version
	);

	std::fs::create_dir_all(&opts.out_dir).with_context(|| format!("creating {}", opts.out_dir.display()))?;

	// Merge + write scheme files across all IDEs into tidy subdirs.
	let color_files = merge_and_write(&ides, "colors", "color-schemes", "icls", &opts.out_dir)?;
	let style_files = merge_and_write(&ides, "codestyles", "code-styles", "xml", &opts.out_dir)?;
	// Copy the primary's self-contained managed files (menus, templates, …).
	let managed = collect_managed_files(primary, &opts.out_dir)?;

	let mut cfg = build_config(&ides, primary, &color_files, &style_files, opts.portable_keymap)?;
	cfg.files = managed.clone();

	// IDE-specific managed files (e.g. window layouts): snapshot each IDE's own
	// copy under targets/<product>/ and record it on that target. `cfg.targets`
	// is built from `ides` in the same order, so they zip 1:1.
	let mut per_target_total = 0usize;
	for (target, ide) in cfg.targets.iter_mut().zip(&ides) {
		target.files = collect_target_files(ide, &opts.out_dir)?;
		per_target_total += target.files.len();
	}

	// Write the config + a copy of the schema for editor autocomplete.
	let json = serde_json::to_string_pretty(&cfg)? + "\n";
	let cfg_path = opts.out_dir.join("jbsync.json");
	std::fs::write(&cfg_path, json).with_context(|| format!("writing {}", cfg_path.display()))?;
	std::fs::write(opts.out_dir.join("jbsync.schema.json"), SCHEMA_JSON)?;

	println!("wrote {}", cfg_path.display());
	println!(
		"wrote {} color scheme(s), {} code style(s), {} shared + {} per-IDE managed file(s) into {}",
		color_files.len(),
		style_files.len(),
		managed.len(),
		per_target_total,
		opts.out_dir.display()
	);
	Ok(())
}

/// Copy each present per-target file from this IDE into
/// `<out>/targets/<product>/<path>`, returning the IDE-relative paths copied
/// (for `Target.files`).
fn collect_target_files(ide: &Ide, out_dir: &Path) -> Result<Vec<String>> {
	let dest_root = out_dir.join("targets").join(&ide.product);
	let mut copied = vec![];
	for rel in PER_TARGET_FILES {
		let src = ide.config_dir.join(rel);
		if src.is_file() {
			let dest = dest_root.join(rel);
			if let Some(parent) = dest.parent() {
				std::fs::create_dir_all(parent)?;
			}
			std::fs::copy(&src, &dest).with_context(|| format!("copying {}", src.display()))?;
			copied.push(rel.to_string());
		}
	}
	Ok(copied)
}

/// Self-contained config dirs we snapshot verbatim from the primary IDE and
/// SHARE across all IDEs. These hold user content that's the same everywhere
/// (live templates, file templates, inspection profiles). IDE-specific
/// `options/*.xml` go in `PER_TARGET_FILES` instead. None are option-patched.
const MANAGED_FILES: &[&str] = &["templates", "fileTemplates", "inspectionProfiles"];

/// Self-contained files snapshotted PER IDE (not shared from the primary), kept
/// under `targets/<product>/<path>` and applied to that IDE only.
///
/// These `options/*.xml` are IDE-specific — menus/toolbars (`customization.xml`),
/// file types, debugger, diff, notifications, parameter hints, VCS, advanced
/// settings, and window layouts all differ between IDEs (each has its own
/// actions, tool windows, and languages), so the primary's copy must not be
/// imposed on the others. NB: `window.state.xml` is deliberately excluded
/// everywhere — it holds per-monitor pixel geometry (DimensionService keys like
/// `…1920.0.1920.1080@120dpi`) that must not follow you across machines.
const PER_TARGET_FILES: &[&str] = &[
	"options/advancedSettings.xml",
	"options/customization.xml",
	"options/debugger.xml",
	"options/diff.xml",
	"options/filetypes.xml",
	"options/grazie_global.xml",
	"options/notifications.xml",
	"options/parameter.hints.xml",
	"options/vcs.xml",
	"options/window.layouts.xml",
];

/// Copy each present managed file/dir from the primary into the output dir,
/// returning the list actually copied (for `config.files`).
fn collect_managed_files(primary: &Ide, out_dir: &Path) -> Result<Vec<String>> {
	let mut copied = vec![];
	for rel in MANAGED_FILES {
		let src = primary.config_dir.join(rel);
		if *rel == "templates" {
			// The live-templates dir is often dominated by records of *disabled
			// bundled* templates (every entry `deactivated="true"`), not custom
			// content. Only copy files that have at least one active template.
			if src.is_dir() && copy_custom_templates(&src, &out_dir.join(rel))? {
				copied.push(rel.to_string());
			}
		} else if src.is_dir() {
			if copy_dir(&src, &out_dir.join(rel))? {
				copied.push(rel.to_string());
			}
		} else if src.is_file() {
			if let Some(parent) = out_dir.join(rel).parent() {
				std::fs::create_dir_all(parent)?;
			}
			std::fs::copy(&src, out_dir.join(rel)).with_context(|| format!("copying {}", src.display()))?;
			copied.push(rel.to_string());
		}
	}
	Ok(copied)
}

/// Copy only the live-template files that contain custom (non-deactivated)
/// templates; skip groups that are purely disabled bundled templates.
fn copy_custom_templates(src: &Path, dst: &Path) -> Result<bool> {
	let mut any = false;
	let mut skipped = 0usize;
	for entry in std::fs::read_dir(src)?.flatten() {
		let path = entry.path();
		if path.extension().is_some_and(|e| e == "xml") {
			if template_file_has_custom(&path) {
				std::fs::create_dir_all(dst)?;
				std::fs::copy(&path, dst.join(entry.file_name()))?;
				any = true;
			} else {
				skipped += 1;
			}
		}
	}
	if skipped > 0 {
		println!("  skipped {skipped} bundled-only template group(s) (all disabled, no custom templates)");
	}
	Ok(any)
}

/// True if a live-template set file has at least one template that isn't a
/// disabled bundled entry (`deactivated="true"`).
fn template_file_has_custom(path: &Path) -> bool {
	let Ok(xml) = std::fs::read_to_string(path) else {
		return false;
	};
	let mut reader = Reader::from_str(&xml);
	loop {
		match reader.read_event() {
			Ok(Event::Start(b)) | Ok(Event::Empty(b)) if tag_name(&b) == "template" => {
				if tag_attr(&b, "deactivated").as_deref() != Some("true") {
					return true;
				}
			}
			Ok(Event::Eof) => return false,
			Ok(_) => {}
			Err(_) => return false,
		}
	}
}

/// Recursively copy a directory; returns false if it had no files.
fn copy_dir(src: &Path, dst: &Path) -> Result<bool> {
	let mut any = false;
	for entry in std::fs::read_dir(src)?.flatten() {
		let path = entry.path();
		let target = dst.join(entry.file_name());
		if path.is_dir() {
			any |= copy_dir(&path, &target)?;
		} else {
			std::fs::create_dir_all(dst)?;
			std::fs::copy(&path, &target)?;
			any = true;
		}
	}
	Ok(any)
}

fn select_ides(products: &[String]) -> Result<Vec<Ide>> {
	let mut out = vec![];
	for (product, version, config_dir) in discovery::discover_all()? {
		if !products.is_empty() && !products.iter().any(|p| p == &product) {
			continue;
		}
		let folder = config_dir.file_name().map(|n| n.to_owned()).unwrap_or_default();
		let install_dir = discovery::data_base(&product)?.join(&folder);
		out.push(Ide {
			product,
			version,
			config_dir,
			install_dir,
		});
	}
	Ok(out)
}

/// Index of the primary IDE. Requires an explicit `--primary` when more than one
/// IDE is being snapshotted; never assumes a default.
fn resolve_primary(ides: &[Ide], primary: Option<&str>) -> Result<usize> {
	match primary {
		Some(p) => ides.iter().position(|i| i.product == p).ok_or_else(|| {
			anyhow!(
				"--primary '{p}' is not among the IDEs being snapshotted ({})",
				product_list(ides)
			)
		}),
		None if ides.len() == 1 => Ok(0),
		None => bail!(
			"multiple IDEs found — choose the primary explicitly with --primary <product>.\n\
             It supplies the single-valued settings (fonts, toggles, active scheme/style, heap, keymap)\n\
             and wins conflicts when merging schemes. Options: {}",
			product_list(ides)
		),
	}
}

fn product_list(ides: &[Ide]) -> String {
	ides.iter().map(|i| i.product.as_str()).collect::<Vec<_>>().join(", ")
}

// ---------------------------------------------------------------------------
// scheme collection + merge
// ---------------------------------------------------------------------------

/// Group same-named schemes across IDEs, merge each group, and write the result
/// into `<out>/<out_subdir>/`. Returns scheme name -> config-relative file path.
fn merge_and_write(
	ides: &[Ide],
	subdir: &str,
	out_subdir: &str,
	ext: &str,
	out_dir: &Path,
) -> Result<BTreeMap<String, String>> {
	let root = if subdir == "colors" { "scheme" } else { "code_scheme" };
	// name -> contents, in IDE order (primary first, so it wins conflicts)
	let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
	for ide in ides {
		let dir = ide.config_dir.join(subdir);
		let Ok(entries) = std::fs::read_dir(&dir) else {
			continue;
		};
		for entry in entries.flatten() {
			let path = entry.path();
			if path.extension().is_none_or(|e| e != "icls" && e != "xml") {
				continue;
			}
			let Ok(content) = std::fs::read_to_string(&path) else {
				continue;
			};
			// Only group files whose root matches this kind (colors vs codestyles).
			if let Some(name) = scheme_merge::scheme_name_of(&content, root) {
				groups.entry(name).or_default().push(content);
			}
		}
	}

	let mut files = BTreeMap::new();
	let mut skipped_empty = 0usize;
	for (name, sources) in groups {
		let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
		let merged = if subdir == "colors" {
			scheme_merge::merge_color_schemes(&refs)
		} else {
			scheme_merge::merge_code_styles(&refs)
		};
		// A single bad file shouldn't abort the whole snapshot.
		let merged = match merged {
			Ok(m) => m,
			Err(e) => {
				eprintln!("  skipped {subdir} scheme '{name}': {e}");
				continue;
			}
		};
		// Skip empty color-scheme overrides (`_@user_*` partialSave artifacts
		// with no actual colors/attributes — nothing to sync).
		if subdir == "colors" && scheme_merge::color_scheme_is_empty(&merged) {
			skipped_empty += 1;
			continue;
		}
		let rel = format!("{out_subdir}/{}.{ext}", sanitize_filename(&name));
		let dest = out_dir.join(&rel);
		if let Some(parent) = dest.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::write(&dest, merged).with_context(|| format!("writing {rel}"))?;
		files.insert(name, rel);
	}
	if skipped_empty > 0 {
		println!("  skipped {skipped_empty} empty color scheme override(s) (no colors/attributes)");
	}
	Ok(files)
}

fn sanitize_filename(name: &str) -> String {
	name.chars()
		.map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c })
		.collect()
}

// ---------------------------------------------------------------------------
// scalar extraction (reverse of the appliers)
// ---------------------------------------------------------------------------

fn build_config(
	ides: &[Ide],
	primary: &Ide,
	color_files: &BTreeMap<String, String>,
	style_files: &BTreeMap<String, String>,
	portable_keymap: bool,
) -> Result<Config> {
	let dir = &primary.config_dir;

	let editor = extract_font(dir, "options/editor-font.xml", "DefaultFont").map(|font| EditorCfg { font: Some(font) });
	let terminal = extract_font(dir, "options/terminal-font.xml", "TerminalFontOptions")
		.map(|font| TerminalCfg { font: Some(font) });
	let console =
		extract_font(dir, "options/console-font.xml", "ConsoleFont").map(|font| ConsoleCfg { font: Some(font) });
	let ui = extract_ui(dir);
	let editor_behavior = extract_editor_behavior(dir);
	let settings = extract_settings(dir);

	let color_scheme = active_scheme(dir, color_files, "options/colors.scheme.xml", |xml| {
		get_attr(xml, "EditorColorsManagerImpl", "global_color_scheme", None, "name")
	});
	let code_style = active_scheme(dir, style_files, "options/code.style.schemes.xml", |xml| {
		get_option(xml, "CodeStyleSchemeSettings", "CURRENT_SCHEME_NAME")
	});

	let vm_options = extract_heap(dir).map(|mb| VmOptionsCfg {
		heap_size_mb: Some(mb),
		extra: vec![],
	});

	let keymap = extract_keymap(dir, &primary.product, portable_keymap);

	Ok(Config {
		schema: Some("./jbsync.schema.json".to_string()),
		// Plugins are emitted PER TARGET (each IDE has its own disabled/installed
		// set), so there is no global plugins block to over-disable.
		targets: ides.iter().map(extract_target_plugins).collect(),
		editor,
		terminal,
		console,
		ui,
		editor_behavior,
		color_scheme,
		code_style,
		plugins: None,
		vm_options,
		keymap,
		settings,
		files: vec![],
	})
}

/// Read every registry setting present in the primary IDE.
fn extract_settings(dir: &Path) -> BTreeMap<String, serde_json::Value> {
	use std::collections::HashMap;
	let mut cache: HashMap<&str, Option<String>> = HashMap::new();
	let mut out = BTreeMap::new();
	for def in crate::settings::SETTINGS {
		let xml = cache.entry(def.file).or_insert_with(|| read(dir, def.file)).clone();
		if let Some(stored) = xml.and_then(|x| get_option(&x, def.component, def.option)) {
			out.insert(def.key.to_string(), crate::settings::to_value(def, &stored));
		}
	}
	out
}

/// One target with its own plugin set: disabled (from the IDE's
/// `disabled_plugins.txt`) and install (the IDs it actually has installed).
fn extract_target_plugins(ide: &Ide) -> Target {
	let disabled = read(&ide.config_dir, "disabled_plugins.txt")
		.map(|t| {
			t.lines()
				.map(str::trim)
				.filter(|l| !l.is_empty())
				.map(str::to_string)
				.collect::<Vec<_>>()
		})
		.unwrap_or_default();
	let install: Vec<String> = installed_ids(&ide.install_dir).into_iter().collect();
	let plugins = (!disabled.is_empty() || !install.is_empty()).then(|| PluginsCfg {
		install,
		repositories: vec![],
		disabled,
	});
	Target {
		product: ide.product.clone(),
		version: Some(ide.version.clone()),
		plugins,
		// Filled in by `collect_target_files` once the output dir is known.
		files: vec![],
	}
}

fn read(dir: &Path, rel: &str) -> Option<String> {
	std::fs::read_to_string(dir.join(rel)).ok()
}

fn extract_font(dir: &Path, rel: &str, component: &str) -> Option<FontCfg> {
	let xml = read(dir, rel)?;
	let f = FontCfg {
		family: get_option(&xml, component, "FONT_FAMILY"),
		size: get_option(&xml, component, "FONT_SIZE").and_then(|s| s.parse().ok()),
		line_spacing: get_option(&xml, component, "LINE_SPACING").and_then(|s| s.parse().ok()),
		ligatures: get_option(&xml, component, "USE_LIGATURES").map(|s| s == "true"),
		regular_weight: get_option(&xml, component, "FONT_REGULAR_SUB_FAMILY"),
		bold_weight: get_option(&xml, component, "FONT_BOLD_SUB_FAMILY"),
	};
	let empty = f.family.is_none()
		&& f.size.is_none()
		&& f.line_spacing.is_none()
		&& f.ligatures.is_none()
		&& f.regular_weight.is_none()
		&& f.bold_weight.is_none();
	(!empty).then_some(f)
}

fn extract_ui(dir: &Path) -> Option<UiCfg> {
	let theme = read(dir, "options/laf.xml").and_then(|xml| get_attr(&xml, "LafManager", "laf", None, "themeId"));
	let lnf = read(dir, "options/ui.lnf.xml");
	let other = read(dir, "options/other.xml");
	let general = read(dir, "options/ide.general.xml");

	let font = other.as_deref().and_then(|xml| {
		let family = get_option(xml, "NotRoamableUiSettings", "fontFace");
		let size = get_option(xml, "NotRoamableUiSettings", "fontSize").and_then(|s| s.parse().ok());
		(family.is_some() || size.is_some()).then_some(FontCfg {
			family,
			size,
			..Default::default()
		})
	});

	let lnf_opt = |key: &str| lnf.as_deref().and_then(|xml| get_option(xml, "UISettings", key));
	let ui = UiCfg {
		font,
		theme,
		compact_tree_indents: lnf_opt("compactTreeIndents").map(|s| s == "true"),
		merge_main_menu_into_toolbar: lnf_opt("SHOW_MAIN_MENU_MODE").map(|s| s == "MERGED_WITH_MAIN_TOOLBAR"),
		contrast_scrollbars: lnf_opt("CONTRAST_SCROLLBARS").map(|s| s == "true"),
		experimental_ui: general.as_deref().and_then(|xml| {
			get_attr(xml, "Registry", "entry", Some(("key", "ide.experimental.ui")), "value").map(|s| s == "true")
		}),
		registry: BTreeMap::new(),
	};
	let empty = ui.font.is_none()
		&& ui.theme.is_none()
		&& ui.compact_tree_indents.is_none()
		&& ui.merge_main_menu_into_toolbar.is_none()
		&& ui.contrast_scrollbars.is_none()
		&& ui.experimental_ui.is_none();
	(!empty).then_some(ui)
}

fn extract_editor_behavior(dir: &Path) -> Option<EditorBehaviorCfg> {
	let editor = read(dir, "options/editor.xml");
	let opt = |key: &str| editor.as_deref().and_then(|xml| get_option(xml, "EditorSettings", key));
	let b = EditorBehaviorCfg {
		soft_wrap: opt("USE_SOFT_WRAPS").map(|s| s == "MAIN_EDITOR"),
		show_breadcrumbs: opt("SHOW_BREADCRUMBS").map(|s| s == "true"),
		show_sticky_lines: opt("SHOW_STICKY_LINES").map(|s| s == "true"),
		ensure_newline_at_eof: opt("IS_ENSURE_NEWLINE_AT_EOF").map(|s| s == "true"),
		emmet: read(dir, "options/emmet.xml")
			.and_then(|xml| get_option(&xml, "EmmetOptions", "emmetEnabled"))
			.map(|s| s == "true"),
		postfix_templates: read(dir, "options/postfixTemplates.xml")
			.and_then(|xml| get_option(&xml, "PostfixTemplatesSettings", "postfixTemplatesEnabled"))
			.map(|s| s == "true"),
	};
	let empty = b.soft_wrap.is_none()
		&& b.show_breadcrumbs.is_none()
		&& b.show_sticky_lines.is_none()
		&& b.ensure_newline_at_eof.is_none()
		&& b.emmet.is_none()
		&& b.postfix_templates.is_none();
	(!empty).then_some(b)
}

fn active_scheme(
	dir: &Path,
	files: &BTreeMap<String, String>,
	rel: &str,
	read_name: impl Fn(&str) -> Option<String>,
) -> Option<SchemeRef> {
	let name = read(dir, rel).and_then(|xml| read_name(&xml))?;
	let file = files.get(&name).cloned();
	Some(SchemeRef {
		name,
		file,
		activate: true,
	})
}

/// Parse `-Xmx<n>m` (or `g`) from the first `*.vmoptions` file in the config dir.
fn extract_heap(dir: &Path) -> Option<u32> {
	let entries = std::fs::read_dir(dir).ok()?;
	for entry in entries.flatten() {
		let path = entry.path();
		if path.extension().is_some_and(|e| e == "vmoptions") {
			if let Ok(text) = std::fs::read_to_string(&path) {
				if let Some(mb) = text.lines().find_map(parse_xmx) {
					return Some(mb);
				}
			}
		}
	}
	None
}

fn parse_xmx(line: &str) -> Option<u32> {
	let rest = line.trim().strip_prefix("-Xmx")?;
	let (num, unit) = rest.split_at(rest.find(|c: char| !c.is_ascii_digit())?);
	let n: u32 = num.parse().ok()?;
	match unit.to_ascii_lowercase().as_str() {
		"m" => Some(n),
		"g" => Some(n * 1024),
		_ => None,
	}
}

// ---------------------------------------------------------------------------
// keymap reverse-engineering
// ---------------------------------------------------------------------------

fn extract_keymap(dir: &Path, product: &str, portable: bool) -> Option<KeymapCfg> {
	// The active-keymap pointer lives in the per-OS settings subdir.
	let active_rel = format!("options/{}/keymap.xml", Os::host().settings_subdir());
	let active = read(dir, &active_rel).and_then(|xml| get_attr(&xml, "KeymapManager", "active_keymap", None, "name"));
	let file = locate_keymap_file(dir, active.as_deref())?;
	let xml = std::fs::read_to_string(file).ok()?;
	// Portable: rewrite the host's primary keyboard modifier (Ctrl on Linux/
	// Windows, Cmd on macOS) to `mod` so it follows the platform on apply.
	let mod_key = portable.then(|| Os::host().primary_modifier());
	let mut km = parse_keymap(&xml, mod_key)?;

	// The user's keymap file stores only *deviations* from its parent default
	// keymap, so inherited bindings (e.g. Find = Ctrl+F) never appear here and the
	// Ctrl→Cmd port can't act on them. When portable, resolve the parent chain
	// from the IDE install and materialise the inherited *primary-modifier*
	// bindings explicitly — those are the ones whose meaning changes across
	// platforms, so on macOS they must port to Cmd instead of being re-inherited
	// as the OS's own default. Non-primary-modifier defaults (function keys,
	// Alt-combos) are identical on every OS and stay inherited.
	if let Some(mk) = mod_key {
		match crate::default_keymap::locate_keymap_jar(product, Os::host()) {
			Some(jar) => {
				let mut defaults = resolve_default_chain(&jar, &km.parent, Some(mk));
				// Plugins/components declare their default shortcuts inline in action
				// descriptors (Git push = Ctrl+Shift+K, etc.), not in keymaps/*.xml,
				// so fold those in too. They fill actions the keymap file leaves
				// empty/absent; a real binding already in the file wins.
				merge_component_defaults(&jar, &km.parent, Some(mk), &mut defaults);
				let mut added = 0usize;
				for (id, binding) in defaults {
					if !km.bindings.contains_key(&id) && binding_has_mod(&binding) {
						km.bindings.insert(id, binding);
						added += 1;
					}
				}
				if added == 0 {
					eprintln!(
						"  warning: --portable-keymap found the default-keymap jar but resolved 0 inherited \
						 bindings for {product} (parent \"{}\"). Inherited shortcuts like Ctrl+F may not port.",
						km.parent
					);
				} else {
					println!("  portable-keymap: materialised {added} inherited shortcut(s) for {product}");
				}
			}
			None => eprintln!(
				"  warning: --portable-keymap could not locate {product}'s default-keymap jar (set \
				 JBSYNC_LAUNCHER to the IDE launcher). Only your *explicit* keymap overrides were captured; \
				 inherited defaults like Ctrl+F will NOT be ported to Cmd on macOS."
			),
		}
	}
	Some(km)
}

/// Resolve a default keymap's full parent chain (read from the IDE jar) into one
/// flat action→binding map, a child keymap's bindings overriding its parent's.
/// `mod_key` rewrites the source primary modifier to `mod` so bindings are
/// portable. Cycles and missing files terminate the walk.
fn resolve_default_chain(jar: &Path, root: &str, mod_key: Option<&str>) -> BTreeMap<String, Binding> {
	fn walk(
		jar: &Path,
		name: &str,
		mod_key: Option<&str>,
		seen: &mut Vec<String>,
		out: &mut BTreeMap<String, Binding>,
	) {
		if seen.iter().any(|n| n == name) {
			return;
		}
		seen.push(name.to_string());
		let Some(xml) = crate::default_keymap::read_keymap_xml(jar, name) else {
			return;
		};
		let Some(km) = parse_keymap(&xml, mod_key) else {
			return;
		};
		// Resolve the parent first so this keymap's bindings layer on top.
		if km.parent != name {
			walk(jar, &km.parent, mod_key, seen, out);
		}
		out.extend(km.bindings);
	}
	let mut out = BTreeMap::new();
	walk(jar, root, mod_key, &mut Vec::new(), &mut out);
	out
}

/// True if any of a binding's keystroke specs contains the portable `mod` token
/// — i.e. it uses the primary modifier and thus changes meaning across platforms.
fn binding_has_mod(b: &Binding) -> bool {
	b.keystrokes()
		.iter()
		.any(|spec| spec.split([',', '+', ' ']).any(|t| t.eq_ignore_ascii_case("mod")))
}

/// True if a binding carries no actual keystroke (a removal / placeholder, e.g.
/// `<action id="Vcs.Push"/>` in the keymap file before the plugin fills it).
fn binding_is_empty(b: &Binding) -> bool {
	b.keystrokes().iter().all(|s| s.trim().is_empty())
}

/// Fold component-declared shortcuts (scanned from every install jar) into the
/// keymap-file defaults. A component binding fills an action the keymap file
/// leaves empty or absent; an explicit keymap-file binding wins. Mouse shortcuts
/// and removals are dropped (only portable keyboard bindings matter here).
fn merge_component_defaults(
	jar: &Path,
	keymap_name: &str,
	mod_key: Option<&str>,
	defaults: &mut BTreeMap<String, Binding>,
) {
	for (id, shortcuts) in crate::default_keymap::component_shortcuts(jar, keymap_name) {
		if defaults.get(&id).is_some_and(|b| !binding_is_empty(b)) {
			continue; // the keymap file already binds it; don't override.
		}
		let specs: Vec<String> = shortcuts
			.iter()
			.filter(|s| !s.remove && !s.mouse)
			.map(|s| {
				let mut spec = keystroke_to_spec(&s.first, mod_key);
				if let Some(sec) = &s.second {
					spec.push_str(", ");
					spec.push_str(&keystroke_to_spec(sec, mod_key));
				}
				spec
			})
			.collect();
		match specs.len() {
			0 => {}
			1 => {
				defaults.insert(id, Binding::One(specs.into_iter().next().unwrap()));
			}
			_ => {
				defaults.insert(id, Binding::Many(specs));
			}
		}
	}
}

fn locate_keymap_file(dir: &Path, active: Option<&str>) -> Option<PathBuf> {
	let keymaps = dir.join("keymaps");
	if let Some(name) = active {
		let cand = keymaps.join(crate::appliers::keymap::keymap_filename(name));
		if cand.is_file() {
			return Some(cand);
		}
	}
	// Fall back to a file matching the host OS, else the first keymap present.
	let host_suffix = format!("({})", Os::host().label());
	let mut first = None;
	let mut host_match = None;
	for entry in std::fs::read_dir(&keymaps).ok()?.flatten() {
		let path = entry.path();
		if path.extension().is_some_and(|e| e == "xml") {
			let stem = path
				.file_stem()
				.map(|s| s.to_string_lossy().into_owned())
				.unwrap_or_default();
			if stem.contains(host_suffix.trim_start_matches('(').trim_end_matches(')')) {
				host_match.get_or_insert(path.clone());
			}
			first.get_or_insert(path);
		}
	}
	host_match.or(first)
}

fn parse_keymap(xml: &str, mod_key: Option<&str>) -> Option<KeymapCfg> {
	let mut reader = Reader::from_str(xml);
	let mut name: Option<String> = None;
	let mut parent: Option<String> = None;
	let mut bindings: BTreeMap<String, Binding> = BTreeMap::new();
	// (action id, shortcut specs (keyboard + mouse), saw any child)
	let mut cur: Option<(String, Vec<String>, bool)> = None;

	loop {
		match reader.read_event() {
			Ok(Event::Start(b)) => on_open(&b, false, &mut name, &mut parent, &mut cur, &mut bindings, mod_key),
			Ok(Event::Empty(b)) => on_open(&b, true, &mut name, &mut parent, &mut cur, &mut bindings, mod_key),
			Ok(Event::End(b)) => on_close(&b, &mut cur, &mut bindings),
			Ok(Event::Eof) => break,
			Ok(_) => {}
			Err(_) => return None,
		}
	}

	let raw_name = name?;
	Some(KeymapCfg {
		name: strip_os_suffix(&raw_name),
		parent: parent.unwrap_or_else(|| "$default".to_string()),
		bindings,
	})
}

fn on_open(
	b: &BytesStart,
	is_empty: bool,
	name: &mut Option<String>,
	parent: &mut Option<String>,
	cur: &mut Option<(String, Vec<String>, bool)>,
	bindings: &mut BTreeMap<String, Binding>,
	mod_key: Option<&str>,
) {
	match tag_name(b).as_str() {
		"keymap" => {
			*name = tag_attr(b, "name");
			*parent = tag_attr(b, "parent");
		}
		"action" => {
			let id = tag_attr(b, "id").unwrap_or_default();
			if is_empty {
				// `<action id="X" />` — an explicitly removed shortcut.
				bindings.insert(id, Binding::Many(vec![]));
			} else {
				*cur = Some((id, vec![], false));
			}
		}
		"keyboard-shortcut" => {
			if let Some(c) = cur.as_mut() {
				c.2 = true;
				if let Some(first) = tag_attr(b, "first-keystroke") {
					let mut spec = keystroke_to_spec(&first, mod_key);
					if let Some(second) = tag_attr(b, "second-keystroke") {
						spec.push_str(", ");
						spec.push_str(&keystroke_to_spec(&second, mod_key));
					}
					c.1.push(spec);
				}
			}
		}
		"mouse-shortcut" => {
			// e.g. "control button1" / "button1 doubleClick" -> "control+button1".
			// Mouse modifiers are always kept literal (Ctrl-click stays Ctrl-click).
			if let Some(c) = cur.as_mut() {
				c.2 = true;
				if let Some(ks) = tag_attr(b, "keystroke") {
					c.1.push(keystroke_to_spec(&ks, None));
				}
			}
		}
		_ => {}
	}
}

fn on_close(b: &BytesEnd, cur: &mut Option<(String, Vec<String>, bool)>, bindings: &mut BTreeMap<String, Binding>) {
	if String::from_utf8_lossy(b.name().as_ref()) != "action" {
		return;
	}
	if let Some((id, shortcuts, saw_child)) = cur.take() {
		if !shortcuts.is_empty() {
			let binding = if shortcuts.len() == 1 {
				Binding::One(shortcuts[0].clone())
			} else {
				Binding::Many(shortcuts)
			};
			bindings.insert(id, binding);
		} else if !saw_child {
			// `<action id="X"></action>` — also a removed shortcut.
			bindings.insert(id, Binding::Many(vec![]));
		}
		// else: a child we don't model (e.g. unknown) — leave the action alone.
	}
}

/// "shift ctrl w" -> "shift+ctrl+w". When `mod_key` is set (e.g. "ctrl"), that
/// token is rewritten to `mod` so it follows the platform's primary modifier on
/// apply; everything else stays literal.
fn keystroke_to_spec(ks: &str, mod_key: Option<&str>) -> String {
	ks.split_whitespace()
		.map(|tok| match mod_key {
			Some(m) if is_primary_modifier(tok, m) => "mod",
			_ => tok,
		})
		.collect::<Vec<_>>()
		.join("+")
}

/// Whether `tok` is the primary modifier `mod_key` in any JetBrains/AWT spelling.
/// The bundled default keymaps use the long forms ("control"/"meta"), so we match
/// the whole family — not just the short token `primary_modifier()` returns.
fn is_primary_modifier(tok: &str, mod_key: &str) -> bool {
	let t = tok.to_ascii_lowercase();
	match mod_key {
		"meta" => matches!(t.as_str(), "meta" | "cmd" | "command"),
		_ => matches!(t.as_str(), "ctrl" | "control"),
	}
}

/// "Verona (Linux)" -> "Verona".
fn strip_os_suffix(name: &str) -> String {
	if let Some(idx) = name.rfind(" (") {
		if name.ends_with(')') {
			return name[..idx].to_string();
		}
	}
	name.to_string()
}

fn tag_name(b: &BytesStart) -> String {
	String::from_utf8_lossy(b.name().as_ref()).into_owned()
}

fn tag_attr(b: &BytesStart, key: &str) -> Option<String> {
	for a in b.attributes().with_checks(false) {
		let a = a.ok()?;
		if a.key.as_ref() == key.as_bytes() {
			return Some(a.unescape_value().ok()?.into_owned());
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_xmx_units() {
		assert_eq!(parse_xmx("-Xmx2048m"), Some(2048));
		assert_eq!(parse_xmx("-Xmx4g"), Some(4096));
		assert_eq!(parse_xmx("-Dfoo=bar"), None);
	}

	#[test]
	fn strips_os_suffix_from_keymap_name() {
		assert_eq!(strip_os_suffix("Verona (Linux)"), "Verona");
		assert_eq!(strip_os_suffix("Verona (macOS)"), "Verona");
		assert_eq!(strip_os_suffix("Plain"), "Plain");
	}

	#[test]
	fn reverses_keymap_into_bindings() {
		let xml = r#"<keymap version="1" name="Verona (Linux)" parent="$default">
  <action id="ReformatCode">
    <keyboard-shortcut first-keystroke="ctrl 1" />
  </action>
  <action id="CloseAllEditors">
    <keyboard-shortcut first-keystroke="shift ctrl w" />
  </action>
  <action id="ReformatChord">
    <keyboard-shortcut first-keystroke="ctrl k" second-keystroke="ctrl s" />
  </action>
  <action id="CopyElement" />
  <action id="GotoDeclaration">
    <mouse-shortcut keystroke="control button1" />
  </action>
</keymap>"#;
		let km = parse_keymap(xml, None).unwrap();
		assert_eq!(km.name, "Verona");
		assert_eq!(km.parent, "$default");
		assert!(matches!(km.bindings.get("ReformatCode"), Some(Binding::One(s)) if s == "ctrl+1"));
		assert!(matches!(km.bindings.get("CloseAllEditors"), Some(Binding::One(s)) if s == "shift+ctrl+w"));
		assert!(matches!(km.bindings.get("ReformatChord"), Some(Binding::One(s)) if s == "ctrl+k, ctrl+s"));
		// explicitly-removed shortcut -> empty
		assert!(matches!(km.bindings.get("CopyElement"), Some(Binding::Many(v)) if v.is_empty()));
		// mouse shortcut is now captured
		assert!(matches!(km.bindings.get("GotoDeclaration"), Some(Binding::One(s)) if s == "control+button1"));
	}

	#[test]
	fn mouse_shortcut_round_trips() {
		use crate::appliers::keymap::generate;
		let xml = r#"<keymap version="1" name="V (Linux)" parent="$default">
  <action id="GotoDeclaration">
    <mouse-shortcut keystroke="control button1" />
  </action>
  <action id="ShowIntentionActions">
    <keyboard-shortcut first-keystroke="alt enter" />
    <mouse-shortcut keystroke="button1 doubleClick" />
  </action>
</keymap>"#;
		let km = parse_keymap(xml, None).unwrap();
		let regen = generate(&km, Os::Linux);
		assert!(
			regen.contains(r#"<mouse-shortcut keystroke="control button1" />"#),
			"{regen}"
		);
		assert!(
			regen.contains(r#"<mouse-shortcut keystroke="button1 doubleClick" />"#),
			"{regen}"
		);
		assert!(
			regen.contains(r#"<keyboard-shortcut first-keystroke="alt enter" />"#),
			"{regen}"
		);
	}

	#[test]
	fn portable_keymap_rewrites_primary_modifier_to_mod() {
		let xml = r#"<keymap version="1" name="V (Linux)" parent="$default">
  <action id="A"><keyboard-shortcut first-keystroke="ctrl 1" /></action>
  <action id="B"><keyboard-shortcut first-keystroke="shift ctrl w" /></action>
  <action id="C"><keyboard-shortcut first-keystroke="alt enter" /></action>
  <action id="M"><mouse-shortcut keystroke="control button1" /></action>
</keymap>"#;
		// Simulate a Linux host where the primary modifier is "ctrl".
		let km = parse_keymap(xml, Some("ctrl")).unwrap();
		assert!(matches!(km.bindings.get("A"), Some(Binding::One(s)) if s == "mod+1"));
		assert!(matches!(km.bindings.get("B"), Some(Binding::One(s)) if s == "shift+mod+w"));
		// non-primary modifiers stay literal
		assert!(matches!(km.bindings.get("C"), Some(Binding::One(s)) if s == "alt+enter"));
		// mouse modifiers stay literal (Ctrl-click stays Ctrl-click)
		assert!(matches!(km.bindings.get("M"), Some(Binding::One(s)) if s == "control+button1"));
	}

	#[test]
	fn long_form_control_is_recognised_as_primary_modifier() {
		// Bundled default keymaps spell it "control"/"meta", not "ctrl"/"cmd".
		let xml = r#"<keymap version="1" name="D" parent="$default">
  <action id="Find"><keyboard-shortcut first-keystroke="control F" /></action>
  <action id="Mac"><keyboard-shortcut first-keystroke="meta F" /></action>
</keymap>"#;
		let on_linux = parse_keymap(xml, Some("ctrl")).unwrap();
		assert!(matches!(on_linux.bindings.get("Find"), Some(Binding::One(s)) if s == "mod+F"));
		// "meta" is not the primary modifier on Linux, so it stays literal.
		assert!(matches!(on_linux.bindings.get("Mac"), Some(Binding::One(s)) if s == "meta+F"));

		let on_mac = parse_keymap(xml, Some("meta")).unwrap();
		assert!(matches!(on_mac.bindings.get("Mac"), Some(Binding::One(s)) if s == "mod+F"));
		assert!(matches!(on_mac.bindings.get("Find"), Some(Binding::One(s)) if s == "control+F"));
	}

	fn jar_with(entries: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
		use std::io::Write;
		use zip::write::SimpleFileOptions;
		let tmp = tempfile::tempdir().unwrap();
		let jar = tmp.path().join("platform.jar");
		let file = std::fs::File::create(&jar).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		for (name, body) in entries {
			zip.start_file(*name, SimpleFileOptions::default()).unwrap();
			zip.write_all(body.as_bytes()).unwrap();
		}
		zip.finish().unwrap();
		(tmp, jar)
	}

	#[test]
	fn resolves_parent_chain_with_child_overriding_parent() {
		let (_tmp, jar) = jar_with(&[
			(
				"keymaps/$default.xml",
				r#"<keymap name="$default" version="1">
  <action id="Find">
    <keyboard-shortcut first-keystroke="control F" />
    <keyboard-shortcut first-keystroke="alt F3" />
  </action>
  <action id="FindNext"><keyboard-shortcut first-keystroke="alt F3" /></action>
  <action id="Rename"><keyboard-shortcut first-keystroke="control alt S" /></action>
</keymap>"#,
			),
			(
				"keymaps/macOS.xml",
				r#"<keymap name="macOS" parent="$default" version="1">
  <action id="Find"><keyboard-shortcut first-keystroke="meta F" /></action>
</keymap>"#,
			),
		]);

		// Resolve the macOS chain as if on macOS (primary modifier = meta).
		let resolved = resolve_default_chain(&jar, "macOS", Some("meta"));
		// macOS's Find overrides $default's (single meta→mod, not the two-shortcut form).
		assert!(matches!(resolved.get("Find"), Some(Binding::One(s)) if s == "mod+F"));
		// Inherited from $default, untouched by the macOS layer.
		assert!(matches!(resolved.get("FindNext"), Some(Binding::One(s)) if s == "alt+F3"));
		// On the macOS chain the primary modifier is Cmd, so an inherited literal
		// `control` binding stays Ctrl (macOS keeps those as Ctrl) — not ported.
		assert!(matches!(resolved.get("Rename"), Some(Binding::One(s)) if s == "control+alt+S"));
	}

	#[test]
	fn only_primary_modifier_defaults_are_materialised() {
		let (_tmp, jar) = jar_with(&[(
			"keymaps/$default.xml",
			r#"<keymap name="$default" version="1">
  <action id="Find"><keyboard-shortcut first-keystroke="control F" /></action>
  <action id="FindNext"><keyboard-shortcut first-keystroke="alt F3" /></action>
  <action id="GotoDecl"><mouse-shortcut keystroke="control button1" /></action>
</keymap>"#,
		)]);

		let resolved = resolve_default_chain(&jar, "$default", Some("ctrl"));
		// Find uses the primary modifier → eligible to materialise.
		assert!(binding_has_mod(resolved.get("Find").unwrap()));
		// A function-key combo is identical on every OS → left to inherit.
		assert!(!binding_has_mod(resolved.get("FindNext").unwrap()));
		// Mouse modifiers stay literal, so Ctrl-click is never treated as `mod`.
		assert!(!binding_has_mod(resolved.get("GotoDecl").unwrap()));
	}

	#[test]
	fn component_defaults_fill_empty_placeholders_but_not_real_bindings() {
		// $default.xml has Vcs.Push as a bare placeholder; the plugin contributes
		// the real shortcut inline. The component scan derives the install from the
		// jar living in `<home>/lib/`, so place it there.
		use std::io::Write;
		use zip::write::SimpleFileOptions;
		let tmp = tempfile::tempdir().unwrap();
		let lib = tmp.path().join("lib");
		std::fs::create_dir_all(&lib).unwrap();
		let jar = lib.join("platform.jar");
		let f = std::fs::File::create(&jar).unwrap();
		let mut zip = zip::ZipWriter::new(f);
		for (name, body) in [
			(
				"keymaps/$default.xml",
				r#"<keymap name="$default" version="1">
  <action id="Vcs.Push"/>
  <action id="Find"><keyboard-shortcut first-keystroke="control F" /></action>
</keymap>"#,
			),
			(
				"META-INF/vcs.xml",
				r#"<idea-plugin><actions>
  <action id="Vcs.Push"><keyboard-shortcut first-keystroke="control shift K" keymap="$default"/></action>
  <action id="Find"><keyboard-shortcut first-keystroke="control shift ENTER" keymap="$default"/></action>
</actions></idea-plugin>"#,
			),
		] {
			zip.start_file(name, SimpleFileOptions::default()).unwrap();
			zip.write_all(body.as_bytes()).unwrap();
		}
		zip.finish().unwrap();

		let mut defaults = resolve_default_chain(&jar, "$default", Some("ctrl"));
		// Pre-merge: Push is an empty placeholder, Find has a real binding.
		assert!(binding_is_empty(defaults.get("Vcs.Push").unwrap()));
		merge_component_defaults(&jar, "$default", Some("ctrl"), &mut defaults);

		// The empty placeholder is filled from the component declaration…
		assert!(matches!(defaults.get("Vcs.Push"), Some(Binding::One(s)) if s == "mod+shift+K"));
		// …but Find's real keymap-file binding is NOT overridden by the component one.
		assert!(matches!(defaults.get("Find"), Some(Binding::One(s)) if s == "mod+F"));
	}
}
