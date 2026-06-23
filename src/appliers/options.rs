//! Appliers backed by `options/*.xml` surgical patches. Each patches into the
//! shared `PatchSet` so files touched by several appliers compose correctly.

use super::{bool_str, fmt_f, PatchSet};
use crate::config::{Config, FontCfg};
use crate::xmlpatch::{ensure, ensure_option};
use anyhow::Result;

pub fn editor_font(cfg: &Config, ps: &mut PatchSet) -> Result<()> {
	if let Some(font) = cfg.editor.as_ref().and_then(|e| e.font.as_ref()) {
		ps.patch("options/editor-font.xml", |xml| font_xml(xml, "DefaultFont", font))?;
	}
	Ok(())
}

pub fn terminal_font(cfg: &Config, ps: &mut PatchSet) -> Result<()> {
	if let Some(font) = cfg.terminal.as_ref().and_then(|t| t.font.as_ref()) {
		ps.patch("options/terminal-font.xml", |xml| {
			font_xml(xml, "TerminalFontOptions", font)
		})?;
	}
	Ok(())
}

pub fn console_font(cfg: &Config, ps: &mut PatchSet) -> Result<()> {
	if let Some(font) = cfg.console.as_ref().and_then(|c| c.font.as_ref()) {
		ps.patch("options/console-font.xml", |xml| font_xml(xml, "ConsoleFont", font))?;
	}
	Ok(())
}

fn font_xml(xml: &str, component: &str, f: &FontCfg) -> Result<String> {
	let mut s = ensure_option(xml, component, "VERSION", "1")?;
	if let Some(sz) = f.size {
		s = ensure_option(&s, component, "FONT_SIZE", &format!("{}", sz as i64))?;
		s = ensure_option(&s, component, "FONT_SIZE_2D", &fmt_f(sz))?;
	}
	if let Some(fam) = &f.family {
		s = ensure_option(&s, component, "FONT_FAMILY", fam)?;
	}
	if let Some(ls) = f.line_spacing {
		s = ensure_option(&s, component, "LINE_SPACING", &fmt_f(ls))?;
	}
	if let Some(lig) = f.ligatures {
		s = ensure_option(&s, component, "USE_LIGATURES", bool_str(lig))?;
	}
	if let Some(w) = &f.regular_weight {
		s = ensure_option(&s, component, "FONT_REGULAR_SUB_FAMILY", w)?;
	}
	if let Some(w) = &f.bold_weight {
		s = ensure_option(&s, component, "FONT_BOLD_SUB_FAMILY", w)?;
	}
	Ok(s)
}

pub fn ui(cfg: &Config, ps: &mut PatchSet) -> Result<()> {
	let Some(ui) = cfg.ui.as_ref() else {
		return Ok(());
	};

	// laf.xml — active theme (LAF). Best-effort: sets <laf themeId=..>.
	if let Some(theme) = &ui.theme {
		ps.patch("options/laf.xml", |xml| {
			ensure(xml, "LafManager", "laf", None, &[("themeId", theme)])
		})?;
	}

	// ui.lnf.xml — UISettings toggles
	if ui.compact_tree_indents.is_some()
		|| ui.merge_main_menu_into_toolbar.is_some()
		|| ui.contrast_scrollbars.is_some()
	{
		ps.patch("options/ui.lnf.xml", |xml| {
			let mut s = xml.to_string();
			if let Some(b) = ui.compact_tree_indents {
				s = ensure_option(&s, "UISettings", "compactTreeIndents", bool_str(b))?;
			}
			if let Some(b) = ui.merge_main_menu_into_toolbar {
				let v = if b {
					"MERGED_WITH_MAIN_TOOLBAR"
				} else {
					"SEPARATE_TOOLBAR"
				};
				s = ensure_option(&s, "UISettings", "SHOW_MAIN_MENU_MODE", v)?;
			}
			if let Some(b) = ui.contrast_scrollbars {
				s = ensure_option(&s, "UISettings", "CONTRAST_SCROLLBARS", bool_str(b))?;
			}
			Ok(s)
		})?;
	}

	// other.xml — UI font lives in NotRoamableUiSettings
	if let Some(font) = ui.font.as_ref() {
		ps.patch("options/other.xml", |xml| {
			let mut s = xml.to_string();
			if let Some(fam) = &font.family {
				s = ensure_option(&s, "NotRoamableUiSettings", "fontFace", fam)?;
			}
			if let Some(sz) = font.size {
				s = ensure_option(&s, "NotRoamableUiSettings", "fontSize", &fmt_f(sz))?;
			}
			s = ensure_option(&s, "NotRoamableUiSettings", "overrideLafFonts", "true")?;
			Ok(s)
		})?;
	}

	// ide.general.xml — Registry entries (experimental UI + power-user escape hatch)
	let mut registry: Vec<(String, String)> = Vec::new();
	if let Some(b) = ui.experimental_ui {
		registry.push(("ide.experimental.ui".into(), bool_str(b).into()));
	}
	for (k, v) in &ui.registry {
		registry.push((k.clone(), v.clone()));
	}
	if !registry.is_empty() {
		ps.patch("options/ide.general.xml", |xml| {
			let mut s = xml.to_string();
			for (k, v) in &registry {
				s = ensure(&s, "Registry", "entry", Some(("key", k)), &[("value", v)])?;
			}
			Ok(s)
		})?;
	}

	Ok(())
}

pub fn editor_behavior(cfg: &Config, ps: &mut PatchSet) -> Result<()> {
	let Some(b) = cfg.editor_behavior.as_ref() else {
		return Ok(());
	};

	if b.soft_wrap.is_some()
		|| b.show_breadcrumbs.is_some()
		|| b.show_sticky_lines.is_some()
		|| b.ensure_newline_at_eof.is_some()
	{
		ps.patch("options/editor.xml", |xml| {
			let mut s = xml.to_string();
			if let Some(sw) = b.soft_wrap {
				let v = if sw { "MAIN_EDITOR" } else { "" };
				s = ensure_option(&s, "EditorSettings", "USE_SOFT_WRAPS", v)?;
				if sw {
					s = ensure_option(&s, "EditorSettings", "SOFT_WRAP_FILE_MASKS", "*")?;
				}
			}
			if let Some(v) = b.show_breadcrumbs {
				s = ensure_option(&s, "EditorSettings", "SHOW_BREADCRUMBS", bool_str(v))?;
			}
			if let Some(v) = b.show_sticky_lines {
				s = ensure_option(&s, "EditorSettings", "SHOW_STICKY_LINES", bool_str(v))?;
			}
			if let Some(v) = b.ensure_newline_at_eof {
				s = ensure_option(&s, "EditorSettings", "IS_ENSURE_NEWLINE_AT_EOF", bool_str(v))?;
			}
			Ok(s)
		})?;
	}

	if let Some(emmet) = b.emmet {
		ps.patch("options/emmet.xml", |xml| {
			let mut s = ensure_option(xml, "EmmetOptions", "emmetEnabled", bool_str(emmet))?;
			s = ensure_option(&s, "CssEmmetOptions", "cssEmmetEnabled", bool_str(emmet))?;
			s = ensure_option(&s, "JsxEmmetOptions", "emmetEnabled", bool_str(emmet))?;
			Ok(s)
		})?;
	}

	if let Some(pf) = b.postfix_templates {
		ps.patch("options/postfixTemplates.xml", |xml| {
			ensure_option(xml, "PostfixTemplatesSettings", "postfixTemplatesEnabled", bool_str(pf))
		})?;
	}

	Ok(())
}
