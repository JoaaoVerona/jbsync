//! A registry of flat `<option>` settings: one table row per setting drives
//! both `apply` (write the option) and `create` (read it back). Adding a new
//! synced setting is a single entry here plus a line in the JSON schema.
//!
//! Only flat `component → option → value` settings live here. Nested/structural
//! files (menus, templates, inspection profiles, Grazie, …) are synced verbatim
//! via the `files` mechanism instead.

use anyhow::{bail, Result};
use serde_json::Value;

#[derive(Clone, Copy)]
pub enum Kind {
	Bool,
	Int,
	Float,
	Str,
	/// Friendly value <-> stored value pairs (e.g. "ask" <-> "ASK").
	Enum(&'static [(&'static str, &'static str)]),
}

pub struct Def {
	pub key: &'static str,
	pub file: &'static str,
	pub component: &'static str,
	pub option: &'static str,
	pub kind: Kind,
}

const CLOSE_CONFIRM: &[(&str, &str)] = &[("ask", "ASK"), ("terminate", "TERMINATE"), ("disconnect", "DISCONNECT")];

/// The full set of synced flat settings. Keep in sync with the `settings`
/// section of `schema/jbsync.schema.json`.
pub const SETTINGS: &[Def] = &[
	// --- appearance / UI (ui.lnf.xml UISettings) ---
	b(
		"ui.openInPreviewTab",
		"options/ui.lnf.xml",
		"UISettings",
		"OPEN_IN_PREVIEW_TAB_IF_POSSIBLE",
	),
	b(
		"ui.showInplaceComments",
		"options/ui.lnf.xml",
		"UISettings",
		"SHOW_INPLACE_COMMENTS",
	),
	b(
		"ui.dndWithAltOnly",
		"options/ui.lnf.xml",
		"UISettings",
		"DND_WITH_PRESSED_ALT_ONLY",
	),
	i(
		"ui.editorTabPlacement",
		"options/ui.lnf.xml",
		"UISettings",
		"EDITOR_TAB_PLACEMENT",
	),
	i(
		"ui.editorTabLimit",
		"options/ui.lnf.xml",
		"UISettings",
		"EDITOR_TAB_LIMIT",
	),
	i(
		"ui.lookupListHeight",
		"options/ui.lnf.xml",
		"UISettings",
		"MAX_LOOKUP_LIST_HEIGHT",
	),
	// --- appearance (other.xml / markdown.xml) ---
	f(
		"appearance.presentationModeScale",
		"options/other.xml",
		"NotRoamableUiSettings",
		"presentationModeIdeScale",
	),
	i(
		"markdown.previewFontSize",
		"options/markdown.xml",
		"MarkdownSettings",
		"fontSize",
	),
	// --- editor display (editor.xml EditorSettings) ---
	b(
		"editor.animatedScrolling",
		"options/editor.xml",
		"EditorSettings",
		"IS_ANIMATED_SCROLLING",
	),
	b(
		"editor.dragAndDrop",
		"options/editor.xml",
		"EditorSettings",
		"IS_DND_ENABLED",
	),
	// --- editor code assistance ---
	b(
		"editor.codeVision",
		"options/editor.xml",
		"CodeVisionSettings",
		"enabled",
	),
	b(
		"editor.foldOneLineMethods",
		"options/editor.xml",
		"JavaCodeFoldingSettings",
		"collapseOneLineMethods",
	),
	b(
		"editor.autoPopupJavadoc",
		"options/editor.xml",
		"CodeInsightSettings",
		"AUTO_POPUP_JAVADOC_INFO",
	),
	b(
		"editor.paramHintsOnCompletion",
		"options/editor.xml",
		"CodeInsightSettings",
		"SHOW_PARAMETER_NAME_HINTS_ON_COMPLETION",
	),
	b(
		"editor.addUnambiguousImports",
		"options/editor.xml",
		"CodeInsightSettings",
		"ADD_UNAMBIGIOUS_IMPORTS_ON_THE_FLY",
	),
	b(
		"kotlin.addUnambiguousImports",
		"options/editor.codeinsight.xml",
		"KotlinCodeInsightSettings",
		"addUnambiguousImportsOnTheFly",
	),
	// --- general / startup (ide.general.xml GeneralSettings) ---
	b(
		"general.showTipsOnStartup",
		"options/ide.general.xml",
		"GeneralSettings",
		"showTipsOnStartup",
	),
	b(
		"general.confirmExit",
		"options/ide.general.xml",
		"GeneralSettings",
		"confirmExit",
	),
	i(
		"general.confirmOpenNewProject",
		"options/ide.general.xml",
		"GeneralSettings",
		"confirmOpenNewProject2",
	),
	e(
		"general.processCloseConfirmation",
		"options/ide.general.xml",
		"GeneralSettings",
		"processCloseConfirmation",
		CLOSE_CONFIRM,
	),
	b(
		"general.reopenLastProject",
		"options/ide.general.xml",
		"GeneralSettings",
		"reopenLastProject",
	),
	b(
		"general.saveOnFrameDeactivation",
		"options/ide.general.xml",
		"GeneralSettings",
		"saveOnFrameDeactivation",
	),
	b(
		"general.autoSaveIfInactive",
		"options/ide.general.xml",
		"GeneralSettings",
		"autoSaveIfInactive",
	),
	// --- updates ---
	b(
		"updates.checkNeeded",
		"options/updates.xml",
		"UpdatesConfigurable",
		"CHECK_NEEDED",
	),
	b(
		"updates.thirdPartyPluginsAllowed",
		"options/updates.xml",
		"UpdatesConfigurable",
		"THIRD_PARTY_PLUGINS_ALLOWED",
	),
	// --- refactoring (baseRefactoring.xml) ---
	b(
		"refactoring.safeDeleteWhenDelete",
		"options/baseRefactoring.xml",
		"BaseRefactoringSettings",
		"SAFE_DELETE_WHEN_DELETE",
	),
	b(
		"refactoring.renameTests",
		"options/baseRefactoring.xml",
		"RefactoringSettings",
		"RENAME_TESTS",
	),
	// --- terminal ---
	b(
		"terminal.optionAsMeta",
		"options/terminal.xml",
		"TerminalOptionsProvider",
		"useOptionAsMetaKey",
	),
	s(
		"terminal.shellPath",
		"options/terminal.xml",
		"TerminalOptionsProvider",
		"myShellPath",
	),
];

const fn b(key: &'static str, file: &'static str, component: &'static str, option: &'static str) -> Def {
	Def {
		key,
		file,
		component,
		option,
		kind: Kind::Bool,
	}
}
const fn i(key: &'static str, file: &'static str, component: &'static str, option: &'static str) -> Def {
	Def {
		key,
		file,
		component,
		option,
		kind: Kind::Int,
	}
}
const fn f(key: &'static str, file: &'static str, component: &'static str, option: &'static str) -> Def {
	Def {
		key,
		file,
		component,
		option,
		kind: Kind::Float,
	}
}
const fn s(key: &'static str, file: &'static str, component: &'static str, option: &'static str) -> Def {
	Def {
		key,
		file,
		component,
		option,
		kind: Kind::Str,
	}
}
const fn e(
	key: &'static str,
	file: &'static str,
	component: &'static str,
	option: &'static str,
	map: &'static [(&'static str, &'static str)],
) -> Def {
	Def {
		key,
		file,
		component,
		option,
		kind: Kind::Enum(map),
	}
}

pub fn find(key: &str) -> Option<&'static Def> {
	SETTINGS.iter().find(|d| d.key == key)
}

/// Convert a config JSON value into the IDE's stored string for this setting.
pub fn to_stored(def: &Def, v: &Value) -> Result<String> {
	let bad = |want: &str| anyhow::anyhow!("setting '{}' expects a {want}", def.key);
	Ok(match def.kind {
		Kind::Bool => v.as_bool().ok_or_else(|| bad("boolean"))?.to_string(),
		Kind::Int => v.as_i64().ok_or_else(|| bad("integer"))?.to_string(),
		Kind::Float => crate::appliers::fmt_f(v.as_f64().ok_or_else(|| bad("number"))? as f32),
		Kind::Str => v.as_str().ok_or_else(|| bad("string"))?.to_string(),
		Kind::Enum(map) => {
			let friendly = v.as_str().ok_or_else(|| bad("string"))?;
			match map.iter().find(|(f, _)| *f == friendly) {
				Some((_, stored)) => stored.to_string(),
				None => bail!(
					"setting '{}' must be one of: {}",
					def.key,
					map.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", ")
				),
			}
		}
	})
}

/// Convert the IDE's stored string back into a config JSON value.
pub fn to_value(def: &Def, stored: &str) -> Value {
	match def.kind {
		Kind::Bool => Value::Bool(stored == "true"),
		Kind::Int => stored.parse::<i64>().map(Value::from).unwrap_or(Value::Null),
		Kind::Float => stored.parse::<f64>().map(Value::from).unwrap_or(Value::Null),
		Kind::Str => Value::String(stored.to_string()),
		Kind::Enum(map) => {
			let friendly = map
				.iter()
				.find(|(_, st)| *st == stored)
				.map(|(f, _)| *f)
				.unwrap_or(stored);
			Value::String(friendly.to_string())
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_are_unique_and_dotted() {
		let mut seen = std::collections::BTreeSet::new();
		for d in SETTINGS {
			assert!(seen.insert(d.key), "duplicate key {}", d.key);
			assert!(d.key.contains('.'), "key {} should be dotted", d.key);
			assert!(d.file.starts_with("options/"));
		}
	}

	#[test]
	fn bool_round_trips() {
		let d = find("editor.codeVision").unwrap();
		assert_eq!(to_stored(d, &Value::Bool(false)).unwrap(), "false");
		assert_eq!(to_value(d, "false"), Value::Bool(false));
	}

	#[test]
	fn enum_maps_both_ways() {
		let d = find("general.processCloseConfirmation").unwrap();
		assert_eq!(to_stored(d, &Value::from("terminate")).unwrap(), "TERMINATE");
		assert_eq!(to_value(d, "TERMINATE"), Value::from("terminate"));
		assert!(to_stored(d, &Value::from("nope")).is_err());
	}

	#[test]
	fn type_mismatch_errors() {
		let d = find("ui.editorTabLimit").unwrap();
		assert!(to_stored(d, &Value::from("not a number")).is_err());
	}
}
