//! Repack an installed VSCode extension directory into a `.vsix`.
//!
//! Locally-installed extensions (`"source": "vsix"` in the `extensions.json`
//! manifest) have no marketplace to download from, so `create` bundles them
//! into the output dir and `apply` installs the bundle via
//! `<cli> --install-extension <file.vsix>`. VSCode keeps the original package
//! manifest as a hidden `.vsixmanifest` inside each installed extension folder,
//! so the folder can be rebuilt into a valid `.vsix` with no build toolchain: a
//! `.vsix` is a zip holding `extension.vsixmanifest` + `[Content_Types].xml` at
//! the root and the extension payload under `extension/`.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Rebuild `out_vsix` from an installed extension directory. Fails if the
/// directory has no `.vsixmanifest` (nothing to lift as the package manifest).
pub fn repack(ext_dir: &Path, out_vsix: &Path) -> Result<()> {
	use zip::write::SimpleFileOptions;

	let manifest = ext_dir.join(".vsixmanifest");
	if !manifest.is_file() {
		bail!(
			"{} has no .vsixmanifest — cannot repack it into a .vsix",
			ext_dir.display()
		);
	}
	let mut files = Vec::new();
	collect_files(ext_dir, ext_dir, &mut files)?;
	files.sort(); // deterministic archive order

	if let Some(parent) = out_vsix.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
	}
	let f = std::fs::File::create(out_vsix).with_context(|| format!("creating {}", out_vsix.display()))?;
	let mut zip = zip::ZipWriter::new(f);
	let opts = SimpleFileOptions::default();

	zip.start_file("extension.vsixmanifest", opts)?;
	zip.write_all(&std::fs::read(&manifest)?)?;
	zip.start_file("[Content_Types].xml", opts)?;
	zip.write_all(content_types(&files).as_bytes())?;
	for rel in &files {
		let src = ext_dir.join(rel);
		zip.start_file(format!("extension/{rel}"), opts)?;
		zip.write_all(&std::fs::read(&src).with_context(|| format!("reading {}", src.display()))?)?;
	}
	zip.finish()?;
	Ok(())
}

/// Forward-slash relative paths of every file under `root`. The installed
/// `.vsixmanifest` is excluded — it is lifted to the archive root instead of
/// travelling in the `extension/` payload (matching what `vsce` packages).
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
	for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
		let path = entry?.path();
		if path.is_dir() {
			collect_files(root, &path, out)?;
		} else if path.is_file() {
			let rel = path
				.strip_prefix(root)
				.expect("walked path is under root")
				.components()
				.map(|c| c.as_os_str().to_string_lossy().into_owned())
				.collect::<Vec<_>>()
				.join("/");
			if rel != ".vsixmanifest" {
				out.push(rel);
			}
		}
	}
	Ok(())
}

/// The OPC `[Content_Types].xml`: a `<Default>` per file extension present,
/// plus an `<Override>` for extension-less files. VSCode itself doesn't read
/// it on install, but it keeps the archive a valid vsix for other tooling.
fn content_types(files: &[String]) -> String {
	let mut defaults: BTreeMap<String, &'static str> = BTreeMap::new();
	defaults.insert("vsixmanifest".to_string(), "text/xml");
	let mut overrides = String::new();
	for rel in files {
		let name = rel.rsplit('/').next().unwrap_or(rel);
		match name
			.rsplit_once('.')
			.map(|(_, e)| e.to_ascii_lowercase())
			.filter(|e| !e.is_empty())
		{
			Some(ext) => {
				let ct = mime_for(&ext);
				defaults.entry(ext).or_insert(ct);
			}
			None => {
				overrides.push_str(&format!(
					"\t<Override PartName=\"/extension/{}\" ContentType=\"application/octet-stream\" />\n",
					xml_escape(rel)
				));
			}
		}
	}
	let mut out = String::from(
		"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n",
	);
	for (ext, ct) in &defaults {
		out.push_str(&format!(
			"\t<Default Extension=\"{}\" ContentType=\"{ct}\" />\n",
			xml_escape(ext)
		));
	}
	out.push_str(&overrides);
	out.push_str("</Types>\n");
	out
}

fn mime_for(ext: &str) -> &'static str {
	match ext {
		"json" => "application/json",
		"js" | "mjs" | "cjs" => "application/javascript",
		"md" => "text/markdown",
		"txt" => "text/plain",
		"xml" | "vsixmanifest" => "text/xml",
		"html" | "htm" => "text/html",
		"css" => "text/css",
		"svg" => "image/svg+xml",
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"gif" => "image/gif",
		_ => "application/octet-stream",
	}
}

fn xml_escape(s: &str) -> String {
	s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write(path: std::path::PathBuf, content: &str) {
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, content).unwrap();
	}

	#[test]
	fn repack_builds_a_valid_vsix_with_manifest_at_root() {
		let tmp = tempfile::tempdir().unwrap();
		let ext = tmp.path().join("local.demo-0.1.0");
		write(ext.join(".vsixmanifest"), "<PackageManifest />");
		write(ext.join("package.json"), r#"{"name":"demo"}"#);
		write(ext.join("out/main.js"), "exports.activate = () => {};");
		write(ext.join("LICENSE"), "MIT");

		let out = tmp.path().join("bundle/local.demo-0.1.0.vsix");
		repack(&ext, &out).unwrap();

		let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
		let names: Vec<String> = (0..zip.len())
			.map(|i| zip.by_index(i).unwrap().name().to_string())
			.collect();
		assert!(names.contains(&"extension.vsixmanifest".to_string()), "{names:?}");
		assert!(names.contains(&"[Content_Types].xml".to_string()), "{names:?}");
		assert!(names.contains(&"extension/package.json".to_string()), "{names:?}");
		assert!(names.contains(&"extension/out/main.js".to_string()), "{names:?}");
		// The manifest is lifted to the root, not repeated in the payload.
		assert!(!names.contains(&"extension/.vsixmanifest".to_string()), "{names:?}");

		let mut body = String::new();
		std::io::Read::read_to_string(&mut zip.by_name("extension/package.json").unwrap(), &mut body).unwrap();
		assert_eq!(body, r#"{"name":"demo"}"#);

		let mut types = String::new();
		std::io::Read::read_to_string(&mut zip.by_name("[Content_Types].xml").unwrap(), &mut types).unwrap();
		assert!(
			types.contains(r#"<Default Extension="json" ContentType="application/json" />"#),
			"{types}"
		);
		assert!(
			types.contains(r#"<Override PartName="/extension/LICENSE" ContentType="application/octet-stream" />"#),
			"{types}"
		);
	}

	#[test]
	fn repack_requires_the_installed_vsixmanifest() {
		let tmp = tempfile::tempdir().unwrap();
		let ext = tmp.path().join("local.bare-0.1.0");
		write(ext.join("package.json"), "{}");
		let err = repack(&ext, &tmp.path().join("x.vsix")).unwrap_err();
		assert!(err.to_string().contains("no .vsixmanifest"), "{err}");
	}
}
