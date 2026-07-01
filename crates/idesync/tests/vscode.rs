//! End-to-end tests for the `vsc` CLI namespace, driving the real `idesync`
//! binary against a seeded VSCode user dir in a temp directory. Discovery is
//! redirected via IDESYNC_VSC_CONFIG_HOME / IDESYNC_VSC_HOME so nothing touches
//! the user's actual editors.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_idesync")
}

fn write(path: PathBuf, content: &str) {
	if let Some(p) = path.parent() {
		fs::create_dir_all(p).unwrap();
	}
	fs::write(path, content).unwrap();
}

fn read(p: PathBuf) -> String {
	fs::read_to_string(p).unwrap()
}

/// Run `idesync vsc <args>` with the given extra env (e.g. the redirect dirs).
fn vsc(extra: &[(&str, &str)], args: &[&str]) -> Output {
	let mut c = Command::new(bin());
	c.arg("vsc").args(args);
	for (k, v) in extra {
		c.env(k, v);
	}
	c.output().expect("failed to run idesync")
}

const CONFIG: &str = r#"{
  "settings": { "editor.fontSize": 15, "editor.fontFamily": "JetBrains Mono" },
  "keybindings": [ { "key": "ctrl+s", "command": "workbench.action.files.save" } ]
}"#;

/// `vsc apply`: settings.json is merged surgically (comments + unmanaged keys
/// kept), keybindings.json is owned, and a follow-up `check` reports in sync.
#[test]
fn apply_merges_settings_and_owns_keybindings_idempotently() {
	let tmp = tempfile::tempdir().unwrap();
	let vs_base = tmp.path().join("vscode-config");
	let user = vs_base.join("Code/User");
	// Seed an existing settings.json with a comment + an unmanaged machine-local key.
	write(
		user.join("settings.json"),
		"{\n  // mine\n  \"telemetry.telemetryLevel\": \"off\",\n  \"editor.fontSize\": 12\n}\n",
	);

	let cfg = tmp.path().join("dotfiles/idesync.json");
	write(cfg.clone(), CONFIG);
	let extra = [("IDESYNC_VSC_CONFIG_HOME", vs_base.to_str().unwrap())];

	let out = vsc(&extra, &["apply", cfg.to_str().unwrap()]);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

	let settings = read(user.join("settings.json"));
	assert!(settings.contains(r#""editor.fontSize": 15"#), "replaced: {settings}");
	assert!(
		settings.contains(r#""editor.fontFamily": "JetBrains Mono""#),
		"inserted: {settings}"
	);
	assert!(settings.contains("// mine"), "comment preserved: {settings}");
	assert!(
		settings.contains(r#""telemetry.telemetryLevel": "off""#),
		"unmanaged key preserved: {settings}"
	);

	let kb = read(user.join("keybindings.json"));
	assert!(kb.starts_with("// Managed by idesync"), "{kb}");
	assert!(kb.contains("workbench.action.files.save"));

	// Idempotent: a second pass reports in sync.
	let chk = vsc(&extra, &["check", cfg.to_str().unwrap()]);
	let stdout = String::from_utf8_lossy(&chk.stdout);
	assert!(chk.status.success(), "expected in sync, got: {stdout}");
	assert!(stdout.contains("Code (VSCode): in sync"), "{stdout}");
}

/// `vsc --product Code apply` targets a single editor; an unknown editor errors.
#[test]
fn product_targets_one_editor_and_rejects_unknown() {
	let tmp = tempfile::tempdir().unwrap();
	let vs_base = tmp.path().join("vscode-config");
	fs::create_dir_all(vs_base.join("Code/User")).unwrap();
	let cfg = tmp.path().join("idesync.json");
	write(cfg.clone(), CONFIG);
	let extra = [("IDESYNC_VSC_CONFIG_HOME", vs_base.to_str().unwrap())];

	let ok = vsc(&extra, &["apply", cfg.to_str().unwrap(), "--product", "Code"]);
	assert!(ok.status.success(), "stderr: {}", String::from_utf8_lossy(&ok.stderr));
	assert!(vs_base.join("Code/User/settings.json").exists());

	let bad = vsc(&extra, &["apply", cfg.to_str().unwrap(), "--product", "Nope"]);
	assert!(!bad.status.success(), "unknown editor should error");
	assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown VSCode editor"));
}

/// `vsc create` snapshots a discovered editor's settings + keybindings + the
/// installed extensions into a portable config whose `$schema` points at the
/// latest release (no local schema copy).
#[test]
fn create_captures_settings_keybindings_and_extensions() {
	let tmp = tempfile::tempdir().unwrap();
	let vs_base = tmp.path().join("vscode-config");
	let user = vs_base.join("Code/User");
	write(
		user.join("settings.json"),
		"{\n  // mine\n  \"editor.tabSize\": 2,\n  \"editor.fontSize\": 13,\n}\n",
	);
	write(
		user.join("keybindings.json"),
		r#"[ { "key": "ctrl+k", "command": "editor.action.deleteLines" } ]"#,
	);
	// A fake extensions manifest under IDESYNC_VSC_HOME (Code → ~/.vscode/extensions).
	let home = tmp.path().join("home");
	write(
		home.join(".vscode/extensions/extensions.json"),
		r#"[{"identifier":{"id":"rust-lang.rust-analyzer"}}]"#,
	);

	let out_dir = tmp.path().join("out");
	let extra = [
		("IDESYNC_VSC_CONFIG_HOME", vs_base.to_str().unwrap()),
		("IDESYNC_VSC_HOME", home.to_str().unwrap()),
	];
	let out = vsc(&extra, &["create", "--out", out_dir.to_str().unwrap()]);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

	let captured = read(out_dir.join("idesync.json"));
	assert!(captured.contains(r#""editor.fontSize": 13"#), "{captured}");
	assert!(captured.contains(r#""editor.tabSize": 2"#), "{captured}");
	assert!(captured.contains("editor.action.deleteLines"), "{captured}");
	assert!(captured.contains("rust-lang.rust-analyzer"), "{captured}");
	// No local schema copy is written; `$schema` points at the latest release.
	assert!(!out_dir.join("idesync-vscode.schema.json").exists());
	assert!(
		captured.contains(
			r#""$schema": "https://github.com/JoaaoVerona/idesync/releases/latest/download/idesync-vscode.schema.json""#
		),
		"{captured}"
	);
}

/// A `mod` token in the config expands on apply straight into `key`, resolved
/// for the host running `apply` — no synthetic `mac`/`linux`/`win` field, since
/// VSCode's user keybindings.json doesn't support per-entry platform overrides.
/// A follow-up `check` reports in sync (idempotent).
#[test]
fn mod_token_expands_on_apply_and_stays_in_sync() {
	let tmp = tempfile::tempdir().unwrap();
	let vs_base = tmp.path().join("vscode-config");
	fs::create_dir_all(vs_base.join("Code/User")).unwrap();
	let cfg = tmp.path().join("vsc.json");
	write(
		cfg.clone(),
		r#"{ "keybindings": [ { "key": "mod+d", "command": "editor.action.addSelectionToNextFindMatch" } ] }"#,
	);
	let extra = [("IDESYNC_VSC_CONFIG_HOME", vs_base.to_str().unwrap())];

	let out = vsc(&extra, &["apply", cfg.to_str().unwrap()]);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

	let kb = read(vs_base.join("Code/User/keybindings.json"));
	// CI hosts (Linux/Windows) resolve `mod` to `ctrl`; only macOS resolves to `cmd`.
	assert!(kb.contains(r#""key": "ctrl+d""#), "expanded key: {kb}");
	assert!(!kb.contains(r#""mac""#), "no synthetic mac field: {kb}");

	let chk = vsc(&extra, &["check", cfg.to_str().unwrap()]);
	assert!(
		chk.status.success(),
		"mod expansion must be idempotent: {}",
		String::from_utf8_lossy(&chk.stdout)
	);
}

/// `create --portable-keymap` folds a captured `ctrl` key + matching `cmd` mac
/// override back into the `mod` token. The realistic captured shape is a bare
/// `key` (no `mac`) using the host primary modifier — on the CI hosts (Linux /
/// Windows) that's `ctrl`. A literal non-primary modifier (`alt`) is left alone.
#[test]
fn create_portable_keymap_folds_host_primary_into_mod() {
	let tmp = tempfile::tempdir().unwrap();
	let vs_base = tmp.path().join("vscode-config");
	write(
		vs_base.join("Code/User/keybindings.json"),
		r#"[ { "key": "ctrl+shift+k", "command": "del" }, { "key": "alt+up", "command": "moveUp" } ]"#,
	);
	let out_dir = tmp.path().join("out");
	let extra = [("IDESYNC_VSC_CONFIG_HOME", vs_base.to_str().unwrap())];

	let out = vsc(
		&extra,
		&["create", "--out", out_dir.to_str().unwrap(), "--portable-keymap"],
	);
	assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

	let captured = read(out_dir.join("idesync.json"));
	assert!(
		captured.contains(r#""mod+shift+k""#),
		"host primary folded to mod: {captured}"
	);
	assert!(!captured.contains("ctrl+shift+k"), "ctrl should be gone: {captured}");
	assert!(
		captured.contains(r#""alt+up""#),
		"non-primary modifier left alone: {captured}"
	);
}

/// Off a TTY (output is captured), `vsc apply` with no config errors instead of
/// hanging on the interactive prompt.
#[test]
fn apply_without_config_off_tty_errors_not_hangs() {
	let out = vsc(&[], &["apply"]);
	assert!(!out.status.success(), "missing config off a TTY must error");
	assert!(
		String::from_utf8_lossy(&out.stderr).contains("config path required"),
		"stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);
}
