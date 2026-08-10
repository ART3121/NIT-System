<p align="center">
  <img src="assets/nit-system-banner.jpeg" alt="NIT System banner" width="900">
</p>

<h1 align="center">NIT System</h1>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-2021-000000?style=flat-square&logo=rust&logoColor=white" alt="Rust 2021">
  <img src="https://img.shields.io/badge/Ratatui-0.29-569cd6?style=flat-square" alt="Ratatui 0.29">
  <img src="https://img.shields.io/badge/Crossterm-0.28-4ec9b0?style=flat-square" alt="Crossterm 0.28">
  <img src="https://img.shields.io/badge/Platform-Linux%20x86--64-f44747?style=flat-square&logo=linux&logoColor=white" alt="Linux x86-64">
  <img src="https://img.shields.io/badge/Storage-Markdown-c586c0?style=flat-square&logo=markdown&logoColor=white" alt="Markdown storage">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-dcdcaa?style=flat-square" alt="MIT License"></a>
  <a href="https://github.com/ART3121/NIT-System/releases/latest"><img src="https://img.shields.io/github/v/release/ART3121/NIT-System?style=flat-square&color=dcdcaa" alt="Latest release"></a>
</p>

NIT System is a fast, local-first notes and task manager for the terminal. It combines immediate command-line capture, a keyboard-driven TUI, readable IDs, directory-scoped workspaces, and optional local AI Roadmaps.

The name represents the three primary concepts:

- **N**otes
- **I**deas
- **T**o-dos

**Items** are available as an additional entry type for resources and references.

NIT stores its data in ordinary text files. It requires no account, database, synchronization service, or NIT background process.

## Philosophy

NIT is built around a simple principle: a note system should reduce the distance between having a thought and preserving it.

The system provides enough structure to make entries useful without turning organization into a separate maintenance task:

- **Capture should become muscle memory.** A complete entry can be stored with one short command.
- **Structure should remain lightweight.** Types describe what an entry is; horizons describe when an Idea or To-do matters.
- **Commands should not compete with natural language.** Operations begin with `-`, so ordinary words remain valid entry text.
- **IDs should remain readable.** Entries use classification-based IDs instead of opaque random identifiers.
- **Archiving should remain neutral.** Moving an entry out of the active view does not force it into a universal “done” state.
- **Workspace boundaries should follow context.** Independent directories can keep unrelated collections separate.
- **Data ownership comes first.** Storage remains local, readable, portable, and manually editable.
- **AI should remain optional.** Local generation assists an entry without becoming required for capture, storage, or navigation.

NIT is a small foundation rather than a prescribed productivity methodology.

## Features

- Capture entries without quoting the text.
- Classify Ideas and To-dos by short, medium, or long horizon.
- Keep Notes and Items timeless.
- Address entries with IDs such as `ST-0001`, `LI-0003`, `N-0001`, and `X-0001`.
- Discover the nearest `.nit/` workspace from nested directories.
- Browse, filter, edit, archive, restore, and delete through the TUI.
- Wrap long content and scroll collections larger than the terminal.
- Edit with Neovim, Vim, Vi, or Nano.
- Import compatible Markdown collections.
- Generate optional Roadmaps through a local Ollama model.
- Run from a standalone release binary; Rust is required only when building from source.

## Contents

- [Installation](#installation)
- [Quick start](#quick-start)
- [Core concepts](#core-concepts)
- [Workspaces](#workspaces)
- [Fast capture](#fast-capture)
- [Command reference](#command-reference)
- [Terminal interface](#terminal-interface)
- [Local AI Roadmaps](#local-ai-roadmaps)
- [Storage format](#storage-format)
- [Legacy migration](#legacy-migration)
- [Importing and manual editing](#importing-and-manual-editing)
- [Architecture](#architecture)
- [Development](#development)
- [Troubleshooting](#troubleshooting)
- [License](#license)

## Installation

### Prebuilt release

Install the latest Linux x86-64 release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ART3121/NIT-System/releases/latest/download/install.sh | sh
```

The installer downloads the release archive, verifies its SHA-256 checksum, and installs `nit` to `~/.local/bin`. Rust, Cargo, and administrator privileges are not required.

If necessary, add the installation directory to `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Install a specific version or select another destination:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ART3121/NIT-System/releases/latest/download/install.sh | \
  NIT_VERSION=0.2.0 NIT_INSTALL_DIR="$HOME/bin" sh
```

Remove a release installation:

```bash
rm "$HOME/.local/bin/nit"
```

### Build from source

Requirements:

- Rust and Cargo;
- a supported terminal editor for editor-based actions;
- a terminal with color support for the complete TUI theme.

Build and install the current source:

```bash
git clone https://github.com/ART3121/NIT-System.git
cd NIT-System
cargo install --path .
```

Cargo normally installs the executable to `~/.cargo/bin/nit`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Reinstall after updating the source:

```bash
cargo install --path . --force
```

## Quick start

Create a directory-scoped workspace:

```bash
mkdir example-project
cd example-project
nit -init
```

Capture entries immediately:

```bash
nit Review the deployment checklist -st
nit Record the service boundaries -n
nit Explore an event-driven design -mi
```

List active entries:

```bash
nit -list
```

Open the terminal interface:

```bash
nit
```

Initialization creates:

```text
example-project/
└── .nit/
    ├── notes
    ├── archive
    └── next-ids
```

Commands executed from a nested directory continue using the nearest workspace:

```bash
mkdir -p src/parser
cd src/parser
nit -list
```

## Core concepts

Every entry has a type. Ideas and To-dos also have a time horizon; Notes and Items are intentionally timeless.

### Entry types

| Type | Code | Intended role |
|---|---:|---|
| Idea | `i` | Possibilities, hypotheses, and concepts to explore |
| Note | `n` | Knowledge, context, observations, and reference text |
| Item | `x` | Links, tools, books, components, and other resources |
| To-do | `t` | Concrete actions that need to be performed |

### Time horizons

| Horizon | Code | Intended role |
|---|---:|---|
| Short | `s` | Current or near-term Ideas and To-dos |
| Medium | `m` | Entries to revisit after current priorities |
| Long | `l` | Future Ideas and long-term actions |

Temporal capture codes place the horizon before the type:

```text
-st  short To-do
-mi  medium Idea
-li  long Idea
```

Timeless types use a single type code:

```text
-n   Note
-x   Item
```

### Entry IDs

Every new entry receives an ID derived from its classification:

```text
SI-0001  Short Idea
MI-0001  Medium Idea
LI-0001  Long Idea
N-0001   Note
X-0001   Item
ST-0001  Short To-do
MT-0001  Medium To-do
LT-0001  Long To-do
```

Each classification has an independent numeric sequence. Four digits are the minimum padding; numbering continues naturally beyond `9999`. IDs are case-insensitive, and shorter numeric input is accepted:

```bash
nit -show ST-0001
nit -show st-1
nit -edit LI-0003
nit -archive N-0012
```

Editing, archiving, and restoring preserve the ID. Deleted numbers are not reused. Commands that accept a query prioritize an ID before searching entry text.

## Workspaces

A `.nit/` directory is the official boundary of a NIT workspace. It keeps one collection independent from collections stored in other directory trees.

### Hierarchical discovery

NIT begins at the current directory and searches each parent directory for `.nit/`. The first valid `.nit/` directory wins:

```text
project/                  ← workspace root
├── .nit/
└── src/
    └── parser/           ← command executed here
```

This allows capture, listing, and the TUI to work from any nested path. Nested workspaces are supported; the nearest one always takes precedence. NIT never creates a workspace implicitly.

Inspect the workspace selected for the current directory:

```bash
nit -root
nit -path
nit -status
```

`-root` and `-path` print only a path, making them suitable for scripts.

### Workspace scopes

Create separate workspaces wherever independent context is useful. For example, a repository can contain its own project entries while another directory tree maintains a separate collection:

```text
workspace-a/.nit/notes
workspace-b/.nit/notes
```

No global database connects these collections, and changing directory naturally selects the applicable context.

### Private and tracked workspaces

Create a workspace and append `.nit/` to `.gitignore`:

```bash
nit -init --private
```

The existing `.gitignore` content is preserved and the rule is not duplicated. A Git repository is not required.

Create a workspace without changing `.gitignore`:

```bash
nit -init --tracked
```

Tracked mode allows `.nit/` to remain available for version control. If `.nit/` already appears to be ignored, NIT reports a warning without modifying the file. Git integration is optional; NIT does not implement synchronization.

## Fast capture

Capture syntax is intentionally direct:

```text
nit <text> -<horizon><type>  # Idea or To-do
nit <text> -<type>           # Note or Item
```

Examples:

```bash
nit Explore a new caching strategy -si
nit Summary of the architecture meeting -n
nit Container runtime documentation -x
nit Review the release process -lt
```

Supported codes:

| Type | Short | Medium | Long | Timeless |
|---|---:|---:|---:|---:|
| Idea | `-si` | `-mi` | `-li` | — |
| Note | — | — | — | `-n` |
| Item | — | — | — | `-x` |
| To-do | `-st` | `-mt` | `-lt` | — |

Quotes are unnecessary. NIT joins every argument before the final classification code into one entry. Missing or unknown codes are rejected.

Operation names always begin with `-`. Ordinary words such as `list`, `show`, `edit`, and `archive` therefore remain valid entry text:

```bash
nit list of parser improvements -n
nit show the prototype during review -st
```

Use `nit -list` and `nit -show` to invoke those operations.

## Command reference

### Workspace commands

| Command | Description |
|---|---|
| `nit -init` | Create `.nit/notes`, `.nit/archive`, and `.nit/next-ids` |
| `nit -init --private` | Create a workspace and add `.nit/` to `.gitignore` |
| `nit -init --tracked` | Create a workspace without changing `.gitignore` |
| `nit -root` | Print the discovered workspace root |
| `nit -path` | Print the discovered `.nit` directory |
| `nit -status` | Show workspace paths and entry counts |
| `nit -migrate` | Migrate legacy `.notes` storage explicitly |
| `nit -assign-ids` | Assign IDs to imported or legacy entries that lack them |
| `nit -migrate-timeless` | Convert legacy timed Note and Item IDs safely |

Initialization never overwrites an existing workspace file.

### Entry commands

| Command | Description |
|---|---|
| `nit -list [code] [--archived]` | List and optionally filter a collection |
| `nit -show <ID or text> [--archived]` | Display one matching entry |
| `nit -edit <ID or text> [--archived]` | Edit one matching entry |
| `nit -archive <ID or text>` | Move an active entry to the archive |
| `nit -import <path>` | Import a compatible notes file |
| `nit -ai-roadmap <ID>` | Generate and review a local AI Roadmap |

Examples:

```bash
nit -list -li
nit -list -n --archived
nit -show ST-0001
nit -edit architecture boundaries
nit -archive N-0004
nit -import path/to/notes.md
```

Text lookup first checks for a case-insensitive exact match and then searches for a fragment. Ambiguous queries are rejected instead of selecting an entry silently.

### Application commands

| Command | Description |
|---|---|
| `nit` | Open the TUI |
| `nit -tui` | Open the TUI explicitly |
| `nit -help` | Display CLI help |
| `nit -version` | Display the installed version |

## Terminal interface

Run `nit` without arguments. The TUI contains:

- a header with entry count, filters, and active/archive view;
- an **Entries** area with type-based colors and automatic scrolling;
- a synchronized right-side **ID** column;
- a **Selected** area with the complete selected entry;
- a bottom command area when command mode is active;
- a gray bottom border with the red `[H]Help` indicator in the lower-right corner.

Long text wraps in **Entries**, **Selected**, and command mode. Wrapped entry rows remain aligned with their IDs. When the collection exceeds the available terminal height, scrolling keeps the selected entry visible.

### Navigation and filters

| Key | Action |
|---|---|
| `↑` / `k` | Select the previous entry |
| `↓` / `j` | Select the next entry |
| `Enter` | Expand or collapse the selected Roadmap |
| `1` | Show all entry types |
| `2` | Show Ideas |
| `3` | Show Notes |
| `4` | Show Items |
| `5` | Show To-dos |
| `h` | Show all horizons |
| `s` | Show the short horizon |
| `m` | Show the medium horizon |
| `l` | Show the long horizon |
| `v` | Switch between active and archived entries |
| `r` | Reload the current collection from disk |

### Selected-entry actions

| Key | Action |
|---|---|
| `c` | Create an entry through the external editor using the active filters |
| `e` | Edit the selected entry through the external editor |
| `a` | Archive the selected active entry |
| `u` | Restore the selected archived entry and return to the active view |
| `dd` | Permanently delete the selected entry |

Deletion requires two consecutive `d` presses. Any different key cancels the armed deletion. There is no automatic trash or recovery directory.

When no type filter is active, `c` creates a short To-do. Note and Item creation remains timeless regardless of the horizon filter.

The editor fallback order is:

```text
nvim → vim → vi → nano
```

### Help

The shortcut list is hidden during normal use. Press uppercase `H` or click `[H]Help` in the lower-right corner to open it. Press `H` or `Esc` to close the Help window. Lowercase `h` remains the horizon-filter shortcut.

### Command mode

Press `:` to open command mode.

Capture an entry:

```text
:w New note -n
```

Exit the application safely:

```text
:q
```

Available input controls:

| Key | Action |
|---|---|
| `Enter` | Execute the current command |
| `Backspace` | Delete the previous character |
| `Esc` | Cancel command mode without closing NIT |
| `Ctrl+C` | Exit safely |

`Esc` outside a dialog or command mode does not exit the TUI. Modified keys do not trigger ordinary single-key actions, and only key-press events are processed.

### AI action panel

Press `i` to open the AI panel on the right. Use `↑`/`↓` or `j`/`k` to select an action and press `Enter` to run it. Press `i` or `Esc` to close the panel.

`Generate Roadmap` is currently available. Additional operations are displayed as disabled previews and do not execute.

Generation runs in the background while the TUI displays the current stage, a spinner, and elapsed time. When the proposal is ready:

- press `Y` to accept and attach it;
- press `N` or `Esc` to reject it without changing storage;
- use `↑` and `↓` to scroll the proposal.

## Local AI Roadmaps

AI support is optional. Standard capture, search, editing, workspace discovery, and storage never require Ollama.

NIT can ask a local Ollama model to transform an active entry into a short, ordered Roadmap. Generated steps remain attached to the original entry as readable Markdown.

### Requirements

- Ollama installed and available in `PATH`;
- the configured model available locally or permission to download it;
- a local Ollama HTTP endpoint.

Generate from the CLI:

```bash
nit -ai-roadmap LI-0001
```

The default model is `qwen3:1.7b`. Select another compatible local model with:

```bash
NIT_AI_MODEL=your-model nit -ai-roadmap LI-0001
```

Runtime characteristics depend on the selected model, Ollama configuration, and execution environment. Benchmark the intended deployment environment when performance comparisons are required.

### Behavior

- Only the selected entry's classification and text are used as task input.
- Other workspace entries and previous generations are not sent as context.
- Model reasoning output is disabled for this operation.
- Responses use a structured schema and are validated before display.
- A Roadmap contains four or five ordered steps with execution guidance, rationale, and an observable completion condition.
- The model may remain loaded briefly after a request so nearby operations can reuse it; Ollama controls the eventual unload.
- A generation has a finite timeout and can be cancelled safely.
- Invalid, incomplete, or superficial output is rejected; one corrective generation may be attempted.
- Workspace files remain unchanged until the proposal is accepted.
- Existing Roadmaps are never replaced automatically.

If the Ollama server is unavailable, NIT attempts to start the standard `ollama serve` process. NIT does not create its own resident AI service. If the model is missing, an interactive request asks before downloading it; non-interactive execution does not download automatically.

The current Roadmap prompt requests Brazilian Portuguese output while preserving technical names from the source entry.

### Development diagnostics

Enable a compact metrics line on standard error:

```bash
NIT_AI_DEBUG=1 nit -ai-roadmap LI-0001
```

The metrics include prompt and output token counts, model load time, evaluation durations, total time, and generated tokens per second.

Run the repeatable development workload with:

```bash
scripts/benchmark-ai.sh
```

The benchmark requires `curl`, `jq`, a running Ollama endpoint, and the configured model. It measures cold and warm requests with fixed representative inputs; benchmark results vary by environment and should be interpreted comparatively rather than as universal performance targets.

## Storage format

The workspace directory contains:

```text
.nit/
├── notes      # active entries
├── archive    # archived entries
└── next-ids   # next sequence number for each classification
```

`notes` and `archive` are human-readable Markdown documents. NIT writes canonical English headings and recognizes supported legacy headings while reading older files.

Example:

```markdown
# NIT System

## Timeless

### Notes
- [N-0001] Review the service boundaries

### Items
- [X-0001] Container runtime documentation

## Short Term

### To-dos
- [ST-0001] Complete the parser validation

## Long Term

### Ideas
- [LI-0001] Add a plugin interface
  **Roadmap**
  1. Inspect extension boundaries
     Como fazer: Identify the components that can expose stable extension points.
```

Empty sections are omitted. Entries preserve insertion order within each section. NIT writes through a temporary file before replacing a destination.

`next-ids` is also ordinary text:

```text
# Next NIT entry IDs
SI 1
MI 1
LI 2
N 2
X 2
ST 2
MT 1
LT 1
```

NIT reconciles sequence counters with IDs in both active and archived collections before allocating a new ID. Do not lower these values manually; they also prevent deleted numbers from being reused.

The current format does not include timestamps, tags, due dates, authorship, commit metadata, synchronization state, or a persistent configuration file.

### Archiving and deletion

Archiving moves an entry from `.nit/notes` to `.nit/archive` without marking it completed:

```bash
nit -archive ST-0001
nit -list --archived
```

The TUI restores archived entries with `u`. Permanent deletion is available only through `dd` in the TUI.

## Legacy migration

### Migrating `.notes` files

When no `.nit/` exists in the current directory or its parents, NIT checks only the current directory for:

```text
.notes
.notes.archive
```

Legacy storage is never discovered in parent directories and never migrated automatically.

Run migration from the directory containing the legacy files:

```bash
nit -migrate
```

Migration validates the source, copies it into a temporary workspace, reloads and compares the interpreted entries, and then installs `.nit/`. Existing workspace destinations are never overwritten.

Original legacy files are preserved as backups when present:

```text
.notes.legacy.bak
.notes.archive.legacy.bak
```

If only one legacy file exists, the missing collection is created as an empty canonical document. IDs are not assigned silently; use:

```bash
nit -assign-ids
```

Before assigning IDs, NIT creates `notes.pre-ids.bak` and `archive.pre-ids.bak` inside `.nit/`.

### Migrating timed Note and Item IDs

Older workspaces may contain Note and Item IDs such as `SN-0001`, `MN-0001`, or `LX-0001`. These remain readable, but allocation and movement operations require explicit conversion:

```bash
nit -migrate-timeless
```

The command converts only timed Note and Item IDs to the current `N-…` and `X-…` forms. Idea and To-do IDs remain unchanged. Before replacement, NIT creates:

```text
notes.pre-timeless.bak
archive.pre-timeless.bak
next-ids.pre-timeless.bak
```

## Importing and manual editing

### Import

Import a compatible file into the active collection:

```bash
nit -import /path/to/notes.md
```

The importer recognizes:

- `Timeless`, `Short Term`, `Medium Term`, and `Long Term` sections;
- supported localized legacy horizon headings;
- `Ideas`, `Notes`, `Items`, and `To-do`/`To-dos` headings;
- entries beginning with `- `;
- continuation lines indented by at least two spaces;
- supported entry IDs and Roadmap blocks.

Entries without IDs receive new classification IDs. Unique current IDs are preserved. Conflicting IDs abort the import. Duplicate entry text is not removed automatically.

Normalize the current active file through the parser and serializer:

```bash
nit -import "$(nit -path)/notes"
```

Before rewriting the same file, NIT creates a process-specific backup inside `.nit/`.

### Manual editing

`.nit/notes` and `.nit/archive` can be edited with any text editor when the expected structure is preserved:

- Notes and Items belong under `## Timeless`;
- Ideas and To-dos belong under a recognized horizon;
- entry lines use `- [ID] text`;
- continuation lines are indented by at least two spaces;
- an optional Roadmap begins with the exact indented marker `  **Roadmap**`;
- Roadmap steps contain a sequential number, title, and indented description.

Example multiline entry:

```markdown
### Notes
- [N-0001] First line
  Continuation of the same note
```

Content outside recognized sections may be omitted during the next canonical rewrite. Keep a backup before experimenting with custom layouts. Press `r` in the TUI to reload files changed externally.

## Architecture

The executable entry point is intentionally small, while each module owns one responsibility:

```text
src/
├── ai.rs            # on-demand Ollama Roadmap generation
├── cli.rs           # argument parsing and command dispatch
├── commands.rs      # entry operations
├── editor.rs        # external editor selection
├── ids.rs           # ID allocation, migration, and validation
├── lib.rs           # application composition
├── main.rs          # executable entry point
├── model.rs         # domain types
├── storage.rs       # Markdown parsing and persistence
├── workspace.rs     # workspace discovery, initialization, and migration
└── tui/
    ├── mod.rs       # state, events, and actions
    └── ui.rs        # terminal rendering
```

Workspace discovery is centralized and passed into CLI and TUI operations. Storage receives explicit paths and remains responsible only for parsing, serialization, reading, and writing.

## Development

Run the required validation suite:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
```

Build an optimized executable:

```bash
cargo build --release --locked
```

The binary is written to `target/release/nit`.

### Release workflow

Release tags must match the version in `Cargo.toml`:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The GitHub Actions workflow validates the project, builds the supported release archive, generates a SHA-256 checksum, prepares the installer, and publishes the assets on GitHub Releases.

## Brand assets

| Banner | Icon |
|---|---|
| <img src="assets/nit-system-banner.jpeg" alt="NIT System banner" width="560"> | <img src="assets/nit-system-icon.jpeg" alt="NIT System icon" width="220"> |

- [`assets/nit-system-banner.jpeg`](assets/nit-system-banner.jpeg)
- [`assets/nit-system-icon.jpeg`](assets/nit-system-icon.jpeg)

## Troubleshooting

### `nit: command not found`

Check the installation location and ensure it is in `PATH`:

```bash
command -v nit
```

Release installations normally use `~/.local/bin`; Cargo installations normally use `~/.cargo/bin`.

### No workspace is found

Return to the intended root and initialize explicitly:

```bash
nit -init
```

NIT never creates storage during capture, listing, or TUI startup.

### Entries do not appear

Inspect the discovered context:

```bash
nit -root
nit -path
nit -status
```

The nearest `.nit/` directory wins.

### A query is ambiguous

Use the entry ID or a longer unique fragment:

```bash
nit -show ST-0001
nit -show deployment checklist for staging
```

### Ollama generation fails

Confirm that Ollama and the configured model are available:

```bash
ollama list
ollama show qwen3:1.7b
```

AI failures do not modify workspace data. Enable `NIT_AI_DEBUG=1` when timing information is needed for diagnosis.

### The terminal remains in raw mode

If the process is terminated externally while the TUI is active, restore the terminal:

```bash
reset
```

### An imported file is rejected

Verify that it contains recognized horizon and type headings and entries beginning with `- `. Files without recognized entries are rejected.

## License

NIT System is available under the [MIT License](LICENSE).
