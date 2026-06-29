//! Read JetBrains' *bundled default keymaps* out of the IDE application install.
//!
//! A user's `keymaps/*.xml` only stores deviations from its parent keymap; the
//! inherited bindings (e.g. Find = Ctrl+F) live in the platform's default keymap,
//! which ships as a resource (`keymaps/<name>.xml`) inside a `lib/*.jar` of the
//! IDE install. `extract::create --portable-keymap` resolves that parent chain so
//! the inherited primary-modifier bindings can be ported (Ctrl→Cmd on macOS)
//! instead of silently re-inheriting the target platform's own default.
//!
//! There is a SECOND source of default shortcuts: plugins/components declare
//! theirs *inline* in action definitions (`<action id=…><keyboard-shortcut
//! keymap="$default" first-keystroke=…/></action>`), scattered across every jar
//! in `lib/` and `plugins/` — NOT in the `keymaps/*.xml` files. Git push
//! (Ctrl+Shift+K), most VCS/refactor actions, etc. live there. `component_shortcuts`
//! scans for those.
//!
//! This module is the filesystem/jar half: find the jar, read a named keymap out
//! of it, and scan component shortcuts. The parse + merge live in `extract` (next
//! to the keymap parser they reuse).

use idesync_core::Os;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The base default keymap every platform keymap inherits from; finding it tells
/// us a jar carries the keymap resources.
const ROOT_KEYMAP: &str = "keymaps/$default.xml";

/// Locate the `lib/*.jar` in this product's install that carries the default
/// keymap resources, or `None` if the install can't be found.
///
/// Two strategies: first the launcher (precise, when it resolves); then a scan of
/// well-known install roots (Toolbox `apps/`, Program Files, `/Applications`) for
/// a directory matching this product. The latter is what makes it work when the
/// launcher isn't on PATH — notably on Windows, where Toolbox lives under
/// `%LOCALAPPDATA%`, not the roaming data dir idesync derives the launcher from.
pub fn locate_keymap_jar(product: &str, os: Os) -> Option<PathBuf> {
	for home in app_home_candidates(product, os) {
		if let Some(jar) = jar_with_keymaps(&home.join("lib")) {
			return Some(jar);
		}
	}
	for root in install_root_candidates(os) {
		let Ok(entries) = std::fs::read_dir(&root) else {
			continue;
		};
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() && matches_product(&path, product) {
				if let Some(jar) = search_install(&path, 3) {
					return Some(jar);
				}
			}
		}
	}
	None
}

/// Well-known parent dirs that contain IDE installs, per OS. Each child is a
/// candidate install (Toolbox app slug, or a `Program Files`/`Applications` dir).
fn install_root_candidates(os: Os) -> Vec<PathBuf> {
	let mut roots = Vec::new();
	let mut env_join = |var: &str, sub: &str| {
		if let Some(v) = std::env::var_os(var) {
			roots.push(PathBuf::from(v).join(sub));
		}
	};
	match os {
		Os::Windows => {
			env_join("LOCALAPPDATA", "JetBrains/Toolbox/apps");
			env_join("APPDATA", "JetBrains/Toolbox/apps");
			env_join("LOCALAPPDATA", "Programs");
			env_join("ProgramFiles", "JetBrains");
			env_join("ProgramFiles(x86)", "JetBrains");
		}
		Os::Macos => {
			roots.push(PathBuf::from("/Applications"));
			if let Some(h) = dirs::home_dir() {
				roots.push(h.join("Applications"));
				roots.push(h.join("Library/Application Support/JetBrains/Toolbox/apps"));
			}
		}
		Os::Linux => {
			if let Some(d) = dirs::data_dir() {
				roots.push(d.join("JetBrains/Toolbox/apps"));
			}
			roots.push(PathBuf::from("/opt"));
		}
	}
	roots
}

/// A directory name matches a product if, normalised (lower-case, separators
/// stripped), it contains the product id — so "intellij-idea", "IntelliJ IDEA
/// 2026.1" and "IntelliJIdea2026.1" all match "IntelliJIdea".
fn matches_product(path: &Path, product: &str) -> bool {
	let Some(name) = path.file_name() else {
		return false;
	};
	let norm = |s: &str| s.to_ascii_lowercase().replace(['-', ' ', '_'], "");
	norm(&name.to_string_lossy()).contains(&norm(product))
}

/// Look for the keymap jar at `<dir>/lib`, descending into subdirs up to `depth`
/// (Toolbox nests installs under `<app>/ch-0/<build>/`). Returns on first hit and
/// never recurses past a level that already has the jar, so it won't trawl
/// `plugins/` or `jbr/`.
fn search_install(dir: &Path, depth: u8) -> Option<PathBuf> {
	if let Some(jar) = jar_with_keymaps(&dir.join("lib")) {
		return Some(jar);
	}
	if depth == 0 {
		return None;
	}
	for entry in std::fs::read_dir(dir).ok()?.flatten() {
		let path = entry.path();
		if path.is_dir() {
			if let Some(jar) = search_install(&path, depth - 1) {
				return Some(jar);
			}
		}
	}
	None
}

/// Read `keymaps/<name>.xml` out of `jar` (the raw XML), or `None` if absent.
pub fn read_keymap_xml(jar: &Path, name: &str) -> Option<String> {
	let file = std::fs::File::open(jar).ok()?;
	let mut archive = zip::ZipArchive::new(file).ok()?;
	let mut entry = archive.by_name(&format!("keymaps/{name}.xml")).ok()?;
	let mut xml = String::new();
	entry.read_to_string(&mut xml).ok()?;
	Some(xml)
}

/// Candidate IDE-install home dirs, derived from the launcher. A direct install
/// has the launcher at `<home>/bin/<script>`; a Toolbox launcher is a wrapper
/// script that execs the real binary, so we also mine paths out of its text.
fn app_home_candidates(product: &str, os: Os) -> Vec<PathBuf> {
	let mut out: Vec<PathBuf> = Vec::new();
	let Some(launcher) = crate::launcher::find_launcher(product, os) else {
		return out;
	};
	push_ancestors(&launcher, &mut out);
	// Toolbox launchers are wrappers (a shell script, a `.cmd`, or an `.exe` with
	// the path embedded) that exec the real binary; mine any install-looking path
	// out of the bytes, tolerating both `/` and `\` separators.
	if let Ok(bytes) = std::fs::read(&launcher) {
		let text = String::from_utf8_lossy(&bytes);
		for tok in text.split(|c: char| c == '"' || c == '\'' || c.is_whitespace() || c == '\0') {
			let lower = tok.to_ascii_lowercase();
			if (tok.contains('/') || tok.contains('\\')) && (lower.contains("apps") || lower.contains("bin")) {
				push_ancestors(Path::new(tok), &mut out);
			}
		}
	}
	out
}

/// Push a path's nearest ancestor dirs (a launcher lives a few levels below the
/// install home — deeper under Toolbox's `apps/<app>/ch-0/<build>/bin/` layout),
/// de-duplicated.
fn push_ancestors(path: &Path, out: &mut Vec<PathBuf>) {
	for anc in path.ancestors().skip(1).take(6) {
		if !out.iter().any(|p| p == anc) {
			out.push(anc.to_path_buf());
		}
	}
}

/// The first jar under `lib/` that contains the default-keymap resources.
/// Platform jars (names mentioning `platform`/`ide.impl`) are tried first so we
/// usually hit on the first open rather than scanning every jar.
fn jar_with_keymaps(lib: &Path) -> Option<PathBuf> {
	let mut jars: Vec<PathBuf> = std::fs::read_dir(lib)
		.ok()?
		.flatten()
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|e| e == "jar"))
		.collect();
	jars.sort_by_key(|p| !likely_platform_jar(p));
	jars.into_iter().find(|jar| jar_has(jar, ROOT_KEYMAP))
}

fn likely_platform_jar(path: &Path) -> bool {
	path.file_name()
		.map(|n| {
			let n = n.to_string_lossy();
			n.contains("ide.impl") || n.contains("platform")
		})
		.unwrap_or(false)
}

fn jar_has(jar: &Path, name: &str) -> bool {
	(|| {
		let file = std::fs::File::open(jar).ok()?;
		let mut archive = zip::ZipArchive::new(file).ok()?;
		let found = archive.by_name(name).is_ok();
		Some(found)
	})()
	.unwrap_or(false)
}

/// One shortcut declared inline in a component/plugin action descriptor. Mouse
/// (`<mouse-shortcut keystroke=…>`) and keyboard (`<keyboard-shortcut
/// first-keystroke=…>`) collapse to the same shape once parsed — a mouse
/// keystroke like "control button1" carries its button token in `first` and is
/// resolved by the `buttonN` detection on the generate side.
#[derive(Debug, Clone)]
pub struct CompShortcut {
	pub first: String,
	pub second: Option<String>,
	/// `remove="true"`: the declaration *removes* an inherited shortcut.
	pub remove: bool,
}

/// Scan every jar in the install (`lib/` + bundled `plugins/`) for shortcuts that
/// components contribute to `keymap_name` (e.g. "$default") via inline
/// `<keyboard-shortcut>`/`<mouse-shortcut>` elements. Returns action id → its
/// declared shortcuts. `platform_jar` anchors the install (it lives in `lib/`).
pub fn component_shortcuts(platform_jar: &Path, keymap_name: &str) -> BTreeMap<String, Vec<CompShortcut>> {
	let mut out: BTreeMap<String, Vec<CompShortcut>> = BTreeMap::new();
	for jar in install_jars(platform_jar) {
		scan_jar_components(&jar, keymap_name, &mut out);
	}
	out
}

/// Every jar under the install's `lib/` and `plugins/*/lib/`.
fn install_jars(platform_jar: &Path) -> Vec<PathBuf> {
	let Some(home) = platform_jar.parent().and_then(Path::parent) else {
		return vec![];
	};
	let mut jars = Vec::new();
	collect_jars(&home.join("lib"), &mut jars);
	if let Ok(plugins) = std::fs::read_dir(home.join("plugins")) {
		for p in plugins.flatten() {
			collect_jars(&p.path().join("lib"), &mut jars);
		}
	}
	jars
}

fn collect_jars(dir: &Path, out: &mut Vec<PathBuf>) {
	if let Ok(rd) = std::fs::read_dir(dir) {
		for e in rd.flatten() {
			let p = e.path();
			if p.extension().is_some_and(|x| x == "jar") {
				out.push(p);
			}
		}
	}
}

fn scan_jar_components(jar: &Path, keymap_name: &str, out: &mut BTreeMap<String, Vec<CompShortcut>>) {
	let Ok(file) = std::fs::File::open(jar) else {
		return;
	};
	let Ok(mut archive) = zip::ZipArchive::new(file) else {
		return;
	};
	for i in 0..archive.len() {
		let Ok(mut entry) = archive.by_index(i) else {
			continue;
		};
		if !entry.name().ends_with(".xml") {
			continue;
		}
		let mut xml = String::new();
		if entry.read_to_string(&mut xml).is_err() {
			continue;
		}
		// Cheap pre-filter: only parse descriptors that bind shortcuts to a keymap.
		if xml.contains("keymap=") {
			parse_component_xml(&xml, keymap_name, out);
		}
	}
}

/// Parse one component descriptor, collecting `<keyboard-shortcut>`/`<mouse-shortcut>`
/// elements whose `keymap` attribute equals `keymap_name`, attributing each to its
/// nearest enclosing `<action id=…>`/`<reference ref=…>`.
fn parse_component_xml(xml: &str, keymap_name: &str, out: &mut BTreeMap<String, Vec<CompShortcut>>) {
	let mut reader = Reader::from_str(xml);
	// One entry per open element (so End pops cleanly); holds the action/reference
	// id when the element introduces one, else None.
	let mut stack: Vec<Option<String>> = Vec::new();
	loop {
		match reader.read_event() {
			Ok(Event::Start(b)) => stack.push(action_id(&b)),
			Ok(Event::End(_)) => {
				stack.pop();
			}
			Ok(Event::Empty(b)) => {
				let name = b.name();
				let (kb, mouse) = (
					name.as_ref() == b"keyboard-shortcut",
					name.as_ref() == b"mouse-shortcut",
				);
				if (kb || mouse) && attr(&b, "keymap").as_deref() == Some(keymap_name) {
					if let Some(id) = stack.iter().rev().flatten().next() {
						if let Some(sc) = shortcut_from(&b, mouse) {
							out.entry(id.clone()).or_default().push(sc);
						}
					}
				}
			}
			Ok(Event::Eof) => break,
			Err(_) => break,
			_ => {}
		}
	}
}

/// The id an element contributes to the stack: `<action id=…>` or
/// `<reference ref=…/id=…>` carry one; everything else contributes `None`.
fn action_id(b: &quick_xml::events::BytesStart) -> Option<String> {
	match b.name().as_ref() {
		b"action" => attr(b, "id"),
		b"reference" => attr(b, "ref").or_else(|| attr(b, "id")),
		_ => None,
	}
}

fn shortcut_from(b: &quick_xml::events::BytesStart, mouse: bool) -> Option<CompShortcut> {
	let remove = attr(b, "remove").as_deref() == Some("true");
	let (first, second) = if mouse {
		(attr(b, "keystroke")?, None)
	} else {
		(attr(b, "first-keystroke")?, attr(b, "second-keystroke"))
	};
	Some(CompShortcut { first, second, remove })
}

fn attr(b: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
	for a in b.attributes().with_checks(false).flatten() {
		if a.key.as_ref() == key.as_bytes() {
			return Some(String::from_utf8_lossy(&a.value).into_owned());
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;
	use zip::write::SimpleFileOptions;

	fn write_jar(path: &Path, entries: &[(&str, &str)]) {
		let file = std::fs::File::create(path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		for (name, body) in entries {
			zip.start_file(*name, SimpleFileOptions::default()).unwrap();
			zip.write_all(body.as_bytes()).unwrap();
		}
		zip.finish().unwrap();
	}

	#[test]
	fn finds_keymap_jar_and_reads_named_keymap() {
		let tmp = tempfile::tempdir().unwrap();
		let lib = tmp.path().join("lib");
		std::fs::create_dir_all(&lib).unwrap();
		// A decoy jar without keymaps, plus the real platform jar.
		write_jar(&lib.join("zzz-other.jar"), &[("META-INF/x", "x")]);
		write_jar(
			&lib.join("intellij.platform.ide.impl.jar"),
			&[
				("keymaps/$default.xml", "<keymap name=\"$default\" version=\"1\"/>"),
				(
					"keymaps/macOS.xml",
					"<keymap name=\"macOS\" parent=\"$default\" version=\"1\"/>",
				),
			],
		);

		let jar = jar_with_keymaps(&lib).expect("should find the platform jar");
		assert!(read_keymap_xml(&jar, "$default").unwrap().contains("$default"));
		assert!(read_keymap_xml(&jar, "macOS").unwrap().contains("parent=\"$default\""));
		assert!(read_keymap_xml(&jar, "Nope").is_none());
	}

	#[test]
	fn no_keymap_jar_returns_none() {
		let tmp = tempfile::tempdir().unwrap();
		let lib = tmp.path().join("lib");
		std::fs::create_dir_all(&lib).unwrap();
		write_jar(&lib.join("plain.jar"), &[("a/b.txt", "hi")]);
		assert!(jar_with_keymaps(&lib).is_none());
	}

	#[test]
	fn product_name_matching_is_separator_insensitive() {
		assert!(matches_product(Path::new("/x/intellij-idea"), "IntelliJIdea"));
		assert!(matches_product(Path::new("/x/IntelliJ IDEA 2026.1"), "IntelliJIdea"));
		assert!(matches_product(Path::new("/x/IntelliJIdea2026.1"), "IntelliJIdea"));
		assert!(matches_product(Path::new("/x/android-studio"), "AndroidStudio"));
		assert!(!matches_product(Path::new("/x/webstorm"), "IntelliJIdea"));
	}

	#[test]
	fn component_shortcuts_parse_inline_action_declarations() {
		// Mirrors how plugins ship default shortcuts: inline in the action def,
		// keyed by keymap name, including reference-based and per-keymap variants.
		let xml = r#"<idea-plugin>
  <actions>
    <action id="Vcs.Push" class="x.VcsPushAction">
      <keyboard-shortcut first-keystroke="control shift K" keymap="$default"/>
      <keyboard-shortcut first-keystroke="meta shift K" keymap="Mac OS X 10.5+"/>
    </action>
    <reference ref="EditorClone">
      <keyboard-shortcut first-keystroke="control G" second-keystroke="control G" keymap="$default"/>
    </reference>
    <action id="Some.Removed">
      <keyboard-shortcut first-keystroke="control alt X" keymap="$default" remove="true"/>
    </action>
  </actions>
</idea-plugin>"#;
		let mut out = BTreeMap::new();
		parse_component_xml(xml, "$default", &mut out);

		let push = &out.get("Vcs.Push").unwrap();
		assert_eq!(push.len(), 1, "only the $default variant, not the Mac OS X one");
		assert_eq!(push[0].first, "control shift K");
		assert!(!push[0].remove);

		let clone = &out.get("EditorClone").unwrap()[0];
		assert_eq!(clone.first, "control G");
		assert_eq!(clone.second.as_deref(), Some("control G"));

		assert!(out.get("Some.Removed").unwrap()[0].remove);
	}

	#[test]
	fn search_install_finds_jar_in_nested_toolbox_layout() {
		let tmp = tempfile::tempdir().unwrap();
		// <app>/ch-0/<build>/lib/<jar>, the deeper Toolbox layout.
		let lib = tmp.path().join("ch-0").join("241.99").join("lib");
		std::fs::create_dir_all(&lib).unwrap();
		write_jar(
			&lib.join("intellij.platform.ide.impl.jar"),
			&[("keymaps/$default.xml", "<keymap name=\"$default\" version=\"1\"/>")],
		);
		let jar = search_install(tmp.path(), 3).expect("should descend into ch-0/<build>/lib");
		assert!(read_keymap_xml(&jar, "$default").is_some());
		// Bound the depth: too shallow to reach it.
		assert!(search_install(tmp.path(), 1).is_none());
	}
}
