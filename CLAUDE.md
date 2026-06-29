# idesync — notes for future work

Rust CLI that applies IDE/editor settings from per-editor JSON configs,
cross-platform. Replaces each editor's built-in settings sync with a single
Git-tracked source of truth. **Pluggable**: each editor/IDE family is its own
crate implementing a core `Editor` trait; the binary just registers them.

> The repo root folder is still named `jbsync/` (history), but the project,
> crates, binary, env vars, and configs are all `idesync`.

## CLI shape

```
idesync --help | --version
idesync list                 # discover installed editors/IDEs (all families)
idesync jb  apply|check|create|keymap …   # JetBrains
idesync vsc apply|check|create …          # VSCode family
```

Each editor namespace owns its own args + config format. Configs are
**per-editor files** (`idesync jb apply jb.json`, `idesync vsc apply vsc.json`) —
there is no shared/unified config. `create` writes an editor-shaped output dir.

## Build / test / lint

Use the Runfile (`run :list` to see all targets) — not raw cargo. All targets are
workspace-wide:

```bash
run build           # cargo build (whole workspace; run build --release for optimized)
run test            # unit + integration tests across all crates
run lint            # FORMAT (cargo fmt) + clippy -D warnings  (modifies files)
run check           # VERIFY (no changes): cargo check + fmt --check + clippy
run cli <args>      # run the binary, e.g. run cli list  /  run cli jb check ./idesync.json
run install         # build release + copy the idesync binary to ~/.local/bin
run release patch   # bump version (major|minor|patch), tag, push -> CI builds release
```

## Workspace layout

A Cargo workspace (`[workspace]` root `Cargo.toml`; shared version via
`[workspace.package]`, deps via `[workspace.dependencies]`):

- **`crates/core`** (`idesync-core`) — the pluggable substrate, editor-agnostic:
    - `editor.rs` — the **`Editor` trait** (`key`/`name`/`discover`/`command`/`run`)
      + `Discovered`. Adding an editor = a new crate implementing this + one line in
      the binary's `editors()` registry.
    - `change.rs` — `FileChange` (the shared unit of work: path → desired content).
    - `runner.rs` — `print_diff` / `write_change` (atomic write + `.idesync-backups/`).
      Both editor crates reuse this so apply/dry-run/backup behave identically.
    - `platform.rs` — `Os` (host/parse/label + JetBrains-y `settings_subdir`/
      `primary_modifier`, kept here as the one shared platform type).
    - `prompt.rs` — shared interactive-prompt helpers (`text`/`text_default`/
      `text_optional`/`confirm`/`select`/`multiselect` + `is_interactive`), the one
      place the `dialoguer` TUI dep lives. Each editor's command wizard calls these so
      the UX is consistent. `is_interactive` = stdin AND stdout are a TTY.
- **`crates/jetbrains`** (`idesync-jetbrains`) — the JetBrains plugin (`key = "jb"`).
  `lib.rs` exposes `editor()` + the `Editor` impl; everything else is the original
  engine (see below). Env overrides: `IDESYNC_JB_CONFIG_HOME`, `IDESYNC_JB_DATA_HOME`,
  `IDESYNC_JB_LAUNCHER`. Ships `schema/idesync-jetbrains.schema.json`.
- **`crates/vscode`** (`idesync-vscode`) — the VSCode-family plugin (`key = "vsc"`).
  Env overrides: `IDESYNC_VSC_CONFIG_HOME` (config base), `IDESYNC_VSC_HOME` (home for
  `extensions/`). Ships `schema/idesync-vscode.schema.json`.
- **`crates/idesync`** (`idesync`, the binary) — `main.rs` only: holds
  `editors() -> Vec<Box<dyn Editor>>`, builds the clap `Command` by attaching each
  editor's `command()`, dispatches by key, and implements global `list`. The
  end-to-end integration tests live here (`tests/integration.rs` = JetBrains via
  `idesync jb …`; `tests/vscode.rs` = `idesync vsc …`; `tests/fixtures/`), driving the
  real binary with the env overrides for hermetic redirection.

### Adding a new editor

New crate `crates/<x>` with a `lib.rs` exposing `pub fn editor() -> impl Editor`
(or a unit struct). Implement `Editor`: pick a unique `key`, build your `command()`
(clap-derive internally via `Cmd::augment_subcommands(Command::new(key))`, then set
`.about()` AFTER augment so the enum doc doesn't override it), and `run()` via
`Cmd::from_arg_matches`. Add `Box::new(idesync_x::editor())` to `editors()` in the
binary + the crate to the workspace members + binary deps. Use `idesync_core`'s
`FileChange`/`runner`/`Os` so your apply path matches the others.

## JetBrains engine (`crates/jetbrains/src`)

- `xmlpatch.rs` — the core: surgical, byte-minimal patching of JetBrains
  `options/*.xml` via quick-xml event streams. **Most JetBrains correctness risk
  lives here.**
- `appliers/` — map config slices to `FileChange`s (pure: old content → new content),
  plus `PluginInstall` actions. `build_plan` returns a `Plan { files, installs }`.
    - **`PatchSet`** (in `mod.rs`): option-patched files go through ONE accumulator so
      several appliers can touch the same file (e.g. `editor.xml` ← `editor_behavior` +
      `named_settings`) without the last write clobbering the others. Any new
      option-patching applier must use `ps.patch(...)`, NOT read the file independently.
    - `keymap.rs` — per-OS keymap generation + the `mod`/`ctrl` (Ctrl/Cmd) transform.
    - `plugins.rs` — disable (file) + ensure-install (detect installed IDs by reading
      `META-INF/plugin.xml` unpacked or inside `lib/*.jar`, then `installPlugins` the missing).
- `launcher.rs` — find the IDE launcher (override → PATH → Toolbox `scripts/`).
- `default_keymap.rs` — read JetBrains' *bundled* default keymaps (`keymaps/<name>.xml`
  inside a `lib/*.jar` of the IDE install, located via the launcher). Used only by
  `create --portable-keymap`: a user keymap file stores only *deviations* from its parent,
  so inherited bindings (Find = Ctrl+F) never appear and the Ctrl→Cmd port can't act on them.
  `extract::resolve_default_chain` walks the parent chain into a flat binding set and
  materialises the inherited **primary-modifier** bindings explicitly (only those change
  meaning across platforms — function keys / Alt-combos stay inherited). User overrides always
  win. A SECOND source: plugins/components declare default shortcuts *inline* in action defs
  (`<action id=…><keyboard-shortcut keymap="$default" first-keystroke=…/></action>`) scattered
  across every `lib/`+`plugins/` jar — Git push (Ctrl+Shift+K), most VCS/refactor actions —
  NOT in `keymaps/*.xml`. `component_shortcuts` scans all jars for these and
  `extract::merge_component_defaults` fills empty/absent keymap-file actions from them (a real
  keymap-file binding wins). Adds ~4s to `create`. NB: `keystroke_to_spec` matches the modifier
  *family* (`ctrl`/`control`, `meta`/`cmd`) since the jars use the long AWT spelling;
  `keymap::resolve_keystroke` preserves key case so named keys (`MINUS`, `F2`, `ENTER`) survive.
  Jar discovery tries the launcher first, then scans well-known install roots
  (`locate_keymap_jar`) — Windows Toolbox lives under `%LOCALAPPDATA%`, NOT the roaming dir
  `find_launcher` derives, so the root scan is what makes a Windows `create` work. If discovery
  still fails, `create` warns loudly and the user can set `IDESYNC_JB_LAUNCHER`; only explicit
  overrides are captured in that case.
- `discovery.rs` — config base (`IDESYNC_JB_CONFIG_HOME`) + data/plugins base
  (`IDESYNC_JB_DATA_HOME`). Android Studio is a Google product → sibling `Google` vendor dir.
- `extract.rs` — `create`: reverse of apply. Reads scalars (via `xmlpatch::get_*`),
  reverses the keymap into bindings, unions installed plugins, and orchestrates scheme merge.
  Read-only w.r.t. IDEs; only writes the output dir. `Config` derives `Serialize` for this.
- `scheme_merge.rs` — merge same-named color schemes (union `<colors>`/`<attributes>` by
  option name) and code styles (union top-level children by tag + name/language). First source
  (primary) wins conflicts; handles self-closing/empty roots. **Attribute gotcha:** an
  `<attributes>` option is either a real color (`<value>`) or a bare `baseAttributes`
  inheritance pointer (an IDE-specific default, not a portable color). `resolve_attribute`
  prefers concrete colors over pointers, and *drops* a pointer when sources disagree so each
  IDE falls back to its own default — otherwise the primary's pointer repaints that token in
  other IDEs (was the "wrong colors in WebStorm" bug).
- `settings.rs` — registry of flat `<option>` settings (key → file/component/option/kind).
  Drives both apply (`appliers/named_settings.rs`) and capture (`extract::extract_settings`).
  **Adding a setting = one row here + one line in the schema's `settings` block.**
- `appliers/files.rs` — verbatim copy of `config.files` (shared, into every IDE) PLUS
  per-target `Target.files` (IDE-specific) sourced from `targets/<product>/<path>`. On
  `create`: shared `MANAGED_FILES` = user-content dirs only (`templates`, `fileTemplates`,
  `inspectionProfiles`); per-target `PER_TARGET_FILES` = every IDE-specific `options/*.xml`
  snapshotted per IDE (they differ per IDE). `window.state.xml` (per-monitor geometry) is excluded.
- `cli.rs` — the `jb` namespace (`apply`/`check`/`create`/`keymap`); `command()` +
  `dispatch()`. `apply`/`check` apply to the config's `targets` PLUS every other IDE discovered
  on the machine (`extend_with_discovered`) so an IDE absent from the config still gets the
  SHARED settings. `--targets-only` restricts to configured targets; `--product X` targets just X.
  **Interactive mode:** required inputs (config / `--out`) are `Option`; when missing (or `-i`)
  AND `prompt::is_interactive()`, a `wizard_*` fills the args via `core::prompt` (product/OS
  menus, `exclude` multi-select from `Section::value_variants()`, text out-dir). `want_interactive`
  gates it so off a TTY a missing input is a normal error, never a hang. `vsc cli.rs` mirrors this.

## VSCode engine (`crates/vscode/src`)

Pure pass-through, NOT translated from JetBrains: raw `settings.json` keys, the raw
`keybindings.json` array, and extension install IDs, applied identically to every targeted
editor (VS Code, Insiders, VSCodium, Cursor, Windsurf).

- `config.rs` — the standalone `VsCodeCfg` (flat top-level: `targets`, `settings`,
  `keybindings`, `extensions`). Field names mirror `idesync-vscode.schema.json`.
- `keymap.rs` — the `mod` token (VSCode analog of JetBrains' `mod`). VSCode keeps a
  single cross-platform `keybindings.json` with native `mac`/`linux`/`win` per-entry
  overrides, so there's no per-OS file generation; instead `expand` (run on every
  apply, before `render_keybindings`) rewrites `"key": "mod+d"` → `"key": "ctrl+d"` +
  `"mac": "cmd+d"` (an explicit `mac` is respected; `ctrl` stays literal). `collapse`
  is the reverse for `vsc create --portable-keymap`. Tokens match whole modifier
  segments across `+`-joins and space-separated chords. Expansion is deterministic →
  idempotent (serde_json `Map` is a BTreeMap, so inserted `mac` sorts stably).
- `jsonc.rs` — the VSCode counterpart to `xmlpatch.rs`: byte-minimal, comment-preserving
  JSONC edits. `merge_settings` does the surgical TOP-LEVEL-key set/replace (apply edits
  high-offset-first); `parse` strips comments/trailing-commas into a `serde_json::Value` for
  reads + capture. **Most VSCode correctness risk lives here.** Only top-level keys are
  patched — settings.json dotted keys (`"editor.fontSize"`) are single top-level keys, not
  nested paths; an object/array value is replaced wholesale (no deep-merge).
- `sync.rs` — the `Family` table (editor → config sub-dir `<config>/<AppDir>/User`, CLI
  command, home-relative `extensions/` dir), discovery (`IDESYNC_VSC_CONFIG_HOME` /
  `IDESYNC_VSC_HOME` overrides), `build_plan` (settings.json merge + keybindings.json
  whole-file), and `ExtensionInstall`. **settings.json is merged surgically** (other keys +
  comments preserved); **keybindings.json is OWNED wholesale** (like a generated keymap — seed
  with `create`). Extension detect/install parallels JetBrains `plugins.rs`: read the
  `extensions/extensions.json` manifest for installed IDs (case-insensitive), `--install-extension`
  the missing ones. NB: the extension CLI writes to the real `~/.vscode*` and CANNOT be
  sandboxed by env — an `apply` smoke test on a box with a real `code`/`cursor` actually installs.
- `cli.rs` — the `vsc` namespace (`apply`/`check`/`create`); `create` capture unions editors
  (first wins on settings, keybindings from first that has any, union of installed extensions).

## Project conventions (mirrors the sibling `runfile` project)

- `rustfmt.toml` uses **hard tabs**, max width 120 — `run lint` formats + lints.
- `rust-toolchain.toml` pins the toolchain (clippy + rustfmt).
- `.editorconfig` / `.gitattributes` (LF) enforce the same in editors.
- `.github/workflows/ci.yml` — lint + test (ubuntu/windows) via `run`, using the
  `Skiley/runfile` setup action to install the `run` CLI.
- `.github/workflows/release.yml` — on a `v*.*.*` tag, calls the reusable
  `Skiley/rust-binary-publish` workflow to build cross-platform binaries + GitHub
  release. `.publisher.json` sets `binary-name: idesync`, `env-prefix: IDESYNC` (so install
  scripts use `IDESYNC_INSTALL_DIR`), and attaches BOTH per-crate schemas as `extra-assets`
  (newline-separated) — so they're downloadable at
  `…/releases/latest/download/idesync-<editor>.schema.json`. `create` does NOT write a local
  schema copy; it sets each generated config's `$schema` to that latest-release URL (the
  `SCHEMA_URL` const in `extract.rs` / `vsc cli.rs`). The schema files stay in each crate's
  `schema/` dir as the source of truth. The `release` Runfile target bumps `[workspace.package].version`.

## Conventions / gotchas

- Self-closing tags must be `<.. />` (space before `/>`) to match JetBrains; see `empty_tag`.
- Preserve a file's existing trailing-newline style (vmoptions/plugins/JSONC) to stay idempotent.
- Appliers must be idempotent: `apply` then `check` must report in-sync.
- Keymap filename sanitisation mirrors JetBrains: non `[A-Za-z0-9 _.-]` → `_`.

## Known non-goals (prototype)

JetBrains: plugin _uninstall_ (installPlugins CLI can't) and exhaustive `options/*.xml`
coverage. VSCode: no translation of JetBrains keymaps/settings into VSCode commands/keys (the
vocabularies don't map cleanly), and no extension uninstall. Add new flat settings by curating a
stable scalar mapping in an applier + schema; treat big artifacts as managed files.
