//! Apply the registry-backed `settings` map: group by file, patch each option
//! into the shared `PatchSet` (so e.g. editor.* settings compose with the typed
//! editorBehavior edits on editor.xml).

use super::PatchSet;
use crate::config::Config;
use crate::settings;
use crate::xmlpatch::ensure_option;
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

pub fn settings(cfg: &Config, ps: &mut PatchSet) -> Result<()> {
	if cfg.settings.is_empty() {
		return Ok(());
	}
	// Resolve + validate every key first, grouped by the file it lives in.
	let mut by_file: BTreeMap<&str, Vec<(&'static settings::Def, String)>> = BTreeMap::new();
	for (key, value) in &cfg.settings {
		let def = settings::find(key).ok_or_else(|| anyhow!("unknown setting key '{key}'"))?;
		let stored = settings::to_stored(def, value)?;
		by_file.entry(def.file).or_default().push((def, stored));
	}

	for (file, edits) in by_file {
		ps.patch(file, |xml| {
			let mut s = xml.to_string();
			for (def, stored) in &edits {
				s = ensure_option(&s, def.component, def.option, stored)?;
			}
			Ok(s)
		})?;
	}
	Ok(())
}
