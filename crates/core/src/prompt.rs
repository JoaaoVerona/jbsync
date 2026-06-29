//! Shared interactive-prompt helpers (text / select / multi-select / confirm),
//! used by every editor's interactive command wizard so the UX is consistent.
//! A thin wrapper over `dialoguer`; the only place the TUI dependency lives.

use anyhow::{Context, Result};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect, Select};
use std::io::IsTerminal;

/// True when both stdin and stdout are a terminal — i.e. we can prompt safely.
/// Commands fall back to interactive mode only when this holds, so piped/CI use
/// never hangs waiting for input.
pub fn is_interactive() -> bool {
	std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn theme() -> ColorfulTheme {
	ColorfulTheme::default()
}

/// Prompt for required free text (non-empty).
pub fn text(prompt: &str) -> Result<String> {
	Input::<String>::with_theme(&theme())
		.with_prompt(prompt)
		.interact_text()
		.context("reading input")
}

/// Prompt for free text with a pre-filled default (Enter accepts it).
pub fn text_default(prompt: &str, default: &str) -> Result<String> {
	Input::<String>::with_theme(&theme())
		.with_prompt(prompt)
		.default(default.to_string())
		.interact_text()
		.context("reading input")
}

/// Prompt for optional free text — an empty answer yields `None`.
pub fn text_optional(prompt: &str) -> Result<Option<String>> {
	let s: String = Input::with_theme(&theme())
		.with_prompt(prompt)
		.allow_empty(true)
		.interact_text()
		.context("reading input")?;
	let s = s.trim();
	Ok((!s.is_empty()).then(|| s.to_string()))
}

/// Yes/no prompt with a default.
pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
	Confirm::with_theme(&theme())
		.with_prompt(prompt)
		.default(default)
		.interact()
		.context("reading confirmation")
}

/// Single-choice menu; returns the chosen index.
pub fn select(prompt: &str, items: &[String], default: usize) -> Result<usize> {
	Select::with_theme(&theme())
		.with_prompt(prompt)
		.items(items)
		.default(default.min(items.len().saturating_sub(1)))
		.interact()
		.context("reading selection")
}

/// Multi-choice menu (space to toggle, Enter to accept); returns chosen indices.
pub fn multiselect(prompt: &str, items: &[String]) -> Result<Vec<usize>> {
	if items.is_empty() {
		return Ok(vec![]);
	}
	MultiSelect::with_theme(&theme())
		.with_prompt(prompt)
		.items(items)
		.interact()
		.context("reading selection")
}
