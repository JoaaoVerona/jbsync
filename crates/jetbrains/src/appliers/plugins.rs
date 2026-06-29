//! Manage plugins: disable (via `disabled_plugins.txt`) and ensure-installed
//! (via the IDE's `installPlugins` CLI).
//!
//! Disabling is *merged* (union) with whatever the IDE already disabled, so
//! applying the config never silently re-enables a plugin the IDE turned off.
//!
//! For installs we detect what is already present by reading each installed
//! plugin's descriptor (`META-INF/plugin.xml`, unpacked or inside a `lib/*.jar`)
//! and only install the IDs that are missing — keeping `apply` idempotent.

use super::{whole_file, Ctx};
use crate::plan::PluginInstall;
use anyhow::Result;
use idesync_core::FileChange;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

pub fn disabled(ctx: &Ctx) -> Result<Vec<FileChange>> {
	let Some(p) = ctx.plugins.as_ref() else {
		return Ok(vec![]);
	};
	if p.disabled.is_empty() {
		return Ok(vec![]);
	}

	let path = ctx.ide_dir.join("disabled_plugins.txt");
	let existing = std::fs::read_to_string(&path).unwrap_or_default();
	let trailing_newline = existing.is_empty() || existing.ends_with('\n');

	let mut set: BTreeSet<String> = existing
		.lines()
		.map(str::trim)
		.filter(|l| !l.is_empty())
		.map(str::to_string)
		.collect();
	for id in &p.disabled {
		set.insert(id.trim().to_string());
	}

	let mut content = set.into_iter().collect::<Vec<_>>().join("\n");
	if trailing_newline {
		content.push('\n');
	}
	Ok(vec![whole_file(&ctx.ide_dir, "disabled_plugins.txt", content)])
}

/// Plan plugin installs: configured IDs minus the ones already installed.
pub fn installs(ctx: &Ctx) -> Vec<PluginInstall> {
	let Some(p) = ctx.plugins.as_ref() else {
		return vec![];
	};
	if p.install.is_empty() {
		return vec![];
	}
	let present = installed_ids(&ctx.install_dir);
	let missing: Vec<String> = p
		.install
		.iter()
		.filter(|id| !present.contains(id.as_str()))
		.cloned()
		.collect();
	if missing.is_empty() {
		return vec![];
	}
	let launcher = crate::launcher::find_launcher(&ctx.product, ctx.target_os);
	vec![PluginInstall {
		product: ctx.product.clone(),
		launcher,
		ids: missing,
		repositories: p.repositories.clone(),
		install_dir: ctx.install_dir.clone(),
	}]
}

/// Scan the install dir for plugin IDs already present.
pub fn installed_ids(install_dir: &Path) -> BTreeSet<String> {
	let mut ids = BTreeSet::new();
	let Ok(entries) = std::fs::read_dir(install_dir) else {
		return ids;
	};
	for entry in entries.flatten() {
		let dir = entry.path();
		if !dir.is_dir() {
			continue;
		}
		if let Some(id) = id_from_plugin_dir(&dir) {
			ids.insert(id);
		}
	}
	ids
}

fn id_from_plugin_dir(dir: &Path) -> Option<String> {
	// Unpacked descriptor.
	if let Ok(xml) = std::fs::read_to_string(dir.join("META-INF/plugin.xml")) {
		if let Some(id) = plugin_id(&xml) {
			return Some(id);
		}
	}
	// Descriptor inside a jar in lib/.
	if let Ok(jars) = std::fs::read_dir(dir.join("lib")) {
		for jar in jars.flatten() {
			let p = jar.path();
			if p.extension().is_some_and(|e| e == "jar") {
				if let Some(id) = id_from_jar(&p) {
					return Some(id);
				}
			}
		}
	}
	None
}

fn id_from_jar(path: &Path) -> Option<String> {
	let file = std::fs::File::open(path).ok()?;
	let mut archive = zip::ZipArchive::new(file).ok()?;
	let mut entry = archive.by_name("META-INF/plugin.xml").ok()?;
	let mut xml = String::new();
	entry.read_to_string(&mut xml).ok()?;
	plugin_id(&xml)
}

/// A plugin's identifier is its `<id>` (or `<name>` when `<id>` is absent —
/// JetBrains' own fallback).
pub fn plugin_id(xml: &str) -> Option<String> {
	first_element_text(xml, "id").or_else(|| first_element_text(xml, "name"))
}

fn first_element_text(xml: &str, want: &str) -> Option<String> {
	let mut reader = Reader::from_str(xml);
	let mut in_want = false;
	loop {
		match reader.read_event() {
			Ok(Event::Start(e)) if e.name().as_ref() == want.as_bytes() => in_want = true,
			Ok(Event::Text(t)) if in_want => {
				let s = t.unescape().ok()?.trim().to_string();
				if !s.is_empty() {
					return Some(s);
				}
				in_want = false;
			}
			Ok(Event::End(_)) if in_want => in_want = false,
			Ok(Event::Eof) => break,
			Ok(_) => {}
			Err(_) => break,
		}
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_id_then_falls_back_to_name() {
		assert_eq!(
			plugin_id("<idea-plugin>\n<id>com.x.y</id>\n<name>Whatever</name>\n</idea-plugin>").as_deref(),
			Some("com.x.y")
		);
		assert_eq!(
			plugin_id("<idea-plugin>\n<name>Only Name</name>\n</idea-plugin>").as_deref(),
			Some("Only Name")
		);
	}

	#[test]
	fn detects_unpacked_and_jar_plugins() {
		let tmp = tempfile::tempdir().unwrap();
		let dir = tmp.path();

		// unpacked plugin
		std::fs::create_dir_all(dir.join("alpha/META-INF")).unwrap();
		std::fs::write(
			dir.join("alpha/META-INF/plugin.xml"),
			"<idea-plugin><id>com.alpha</id></idea-plugin>",
		)
		.unwrap();

		// jar-packed plugin
		std::fs::create_dir_all(dir.join("beta/lib")).unwrap();
		write_jar_with_descriptor(
			&dir.join("beta/lib/beta.jar"),
			"<idea-plugin><id>com.beta</id></idea-plugin>",
		);

		let ids = installed_ids(dir);
		assert!(ids.contains("com.alpha"));
		assert!(ids.contains("com.beta"));
	}

	fn write_jar_with_descriptor(path: &Path, descriptor: &str) {
		use std::io::Write;
		use zip::write::SimpleFileOptions;
		let file = std::fs::File::create(path).unwrap();
		let mut zip = zip::ZipWriter::new(file);
		zip.start_file("META-INF/plugin.xml", SimpleFileOptions::default())
			.unwrap();
		zip.write_all(descriptor.as_bytes()).unwrap();
		zip.finish().unwrap();
	}
}
