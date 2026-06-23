# jbsync — notes for future work

Rust CLI that applies JetBrains IDE settings from one JSON config, cross-platform.
Replaces JetBrains Settings Sync with a single Git-tracked source of truth.

## Build / test / lint
Use the Runfile (`run :list` to see all targets) — not raw cargo:
```bash
run build           # cargo build  (run build --release for optimized)
run build:release   # optimized release binary
run test            # unit + integration tests
run lint            # cargo fmt --check + clippy -D warnings
run ci              # lint + test (pre-push)
run cli <args>      # run the binary, e.g. run cli list
run release patch   # bump version (major|minor|patch), tag, push -> CI builds release
```

## Project conventions (mirrors the sibling `runfile` project)
- `rustfmt.toml` uses **hard tabs**, max width 120 — run `run fmt` after edits.
- `rust-toolchain.toml` pins the toolchain (clippy + rustfmt).
- `.editorconfig` / `.gitattributes` (LF) enforce the same in editors.
- `.github/workflows/ci.yml` — lint + test (ubuntu/windows) via `run`, using the
  `Skiley/runfile` setup action to install the `run` CLI.
- `.github/workflows/release.yml` — on a `v*.*.*` tag, calls the reusable
  `Skiley/rust-binary-publish` workflow to build cross-platform binaries + GitHub
  release (npm publish disabled). The `release` Runfile target creates the tag.

## Layout
- `src/xmlpatch.rs` — the core: surgical, byte-minimal patching of JetBrains
  `options/*.xml` via quick-xml event streams. **Most correctness risk lives here.**
- `src/appliers/` — map config slices to `FileChange`s (pure: old content → new content),
  plus `PluginInstall` actions. `build_plan` returns a `Plan { files, installs }`.
  - **`PatchSet`** (in `mod.rs`): option-patched files go through ONE accumulator so
    several appliers can touch the same file (e.g. `editor.xml` ← `editor_behavior` +
    `named_settings`) without the last write clobbering the others. Any new
    option-patching applier must use `ps.patch(...)`, NOT read the file independently.
  - `keymap.rs` — per-OS keymap generation + the `mod`/`ctrl` (Ctrl/Cmd) transform.
  - `plugins.rs` — disable (file) + ensure-install (detect installed IDs by reading
    `META-INF/plugin.xml` unpacked or inside `lib/*.jar`, then `installPlugins` the missing).
- `src/launcher.rs` — find the IDE launcher (override → PATH → Toolbox `scripts/`).
- `src/discovery.rs` — config base (`JBSYNC_CONFIG_HOME`) + data/plugins base (`JBSYNC_DATA_HOME`).
- `src/extract.rs` — `create`: reverse of apply. Reads scalars (via `xmlpatch::get_*`),
  reverses the keymap into bindings, unions installed plugins, and orchestrates scheme merge.
  Read-only w.r.t. IDEs; only writes the output dir. `Config` derives `Serialize` for this.
- `src/scheme_merge.rs` — merge same-named color schemes (union `<colors>`/`<attributes>` by
  option name) and code styles (union top-level children by tag + name/language). First source
  wins conflicts; handles self-closing/empty roots.
- `src/settings.rs` — registry of flat `<option>` settings (key → file/component/option/kind).
  Drives both apply (`appliers/named_settings.rs`) and capture (`extract::extract_settings`).
  **Adding a setting = one row here + one line in the schema's `settings` block.**
- `src/appliers/files.rs` — verbatim copy of `config.files` (whole self-contained settings
  files/dirs: menus, templates, inspections, grazie, …). `create` collects a curated set.
- `src/cli.rs` — `apply` / `check` / `create` / `list` / `keymap`; diff, backup, atomic write, run installs.
- `schema/jbsync.schema.json` — user-facing JSON Schema (keep in sync with `src/config.rs`).
- `tests/fixtures/` — real (sanitized) JetBrains files used by integration tests.

## Conventions / gotchas
- Self-closing tags must be `<.. />` (space before `/>`) to match JetBrains; see `empty_tag`.
- Preserve a file's existing trailing-newline style (vmoptions/plugins) to stay idempotent.
- Appliers must be idempotent: `apply` then `check` must report in-sync.
- Keymap filename sanitisation mirrors JetBrains: non `[A-Za-z0-9 _.-]` → `_`.

## Known non-goals (prototype)
Plugin *uninstall* (installPlugins CLI can't) and exhaustive `options/*.xml`
coverage. Add new settings by curating a stable
scalar mapping in an applier + schema; treat big artifacts as managed files.
