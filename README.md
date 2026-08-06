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
  <a href="https://github.com/ART3121/NIT-System/releases/latest"><img src="https://img.shields.io/github/v/release/ART3121/NIT-System?style=flat-square&color=dcdcaa" alt="Latest release"></a>
</p>

NIT System is a fast, local-first note and task manager for the terminal. It is designed for immediate capture, keyboard-driven organization, and plain-text ownership.

The name represents the system's three primary concepts:

- **N**otes
- **I**deas
- **T**o-dos

**Items** are also supported as an additional entry type, but they are not part of the NIT acronym.

NIT stores data in human-readable Markdown files. It requires no account, database, synchronization service, or background process.

## Philosophy

NIT is built around a simple principle: a note system should reduce the distance between having a thought and preserving it.

Many note-taking workflows gradually become systems that must be maintained. They introduce templates, dashboards, notifications, complex hierarchies, and organizational rules that can demand more attention than the information itself. NIT takes the opposite approach. It provides just enough structure to make entries useful while keeping capture immediate.

The system follows a few core ideas:

- **Capture should become muscle memory.** A thought can be stored with one short command, without opening a workspace or navigating menus.
- **Organization should remain lightweight.** Types describe what an entry is, while horizons describe when it matters. This avoids deep folder trees and elaborate taxonomies.
- **Uncertainty is acceptable.** An entry does not need a perfect classification before it can be saved. Capture codes are optional, and an unclassified entry falls back to a short-term to-do.
- **Archived does not mean completed.** Ideas, notes, items, and to-dos can all leave the active view without being forced into a universal “done” state.
- **The user owns the data.** Notes remain local, readable, portable, and editable without NIT.
- **The interface should stay out of the way.** The direct CLI is optimized for capture, the TUI for review and maintenance, and an external editor for longer changes.

NIT is not intended to prescribe a complete productivity methodology. It is a small foundation that can adapt to different workflows without becoming another workflow to manage.

## Features

- Capture entries directly from the command line without quotes.
- Organize entries by type and time horizon.
- Browse and manage entries through a keyboard-driven TUI.
- Automatically scroll through collections larger than the terminal window.
- Archive and restore entries without treating them as completed tasks.
- Edit entries with Neovim, Vim, Vi, or Nano.
- Keep active and archived data in portable Markdown files.
- Import existing files that follow the supported heading structure.
- Run as a small standalone binary; Rust is only required to build from source.

## Table of contents

- [Philosophy](#philosophy)
- [Core concepts](#core-concepts)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Directory-scoped workspaces](#directory-scoped-workspaces)
- [Fast capture](#fast-capture)
- [Command reference](#command-reference)
- [Terminal interface](#terminal-interface)
- [Storage](#storage)
- [Archiving and deletion](#archiving-and-deletion)
- [Importing notes](#importing-notes)
- [Manual editing](#manual-editing)
- [Brand assets](#brand-assets)
- [Architecture](#architecture)
- [Development](#development)
- [Troubleshooting](#troubleshooting)

## Core concepts

Every entry has a **type** and a **time horizon**.

### Entry types

| Type | Code | Suggested use |
|---|---:|---|
| Idea | `i` | Possibilities, hypotheses, and early project concepts |
| Note | `n` | Knowledge, context, observations, and reference text |
| Item | `x` | Links, tools, people, books, and other resources |
| To-do | `t` | Actions that need to be performed |

### Time horizons

| Horizon | Code | Suggested use |
|---|---:|---|
| Short | `s` | Immediate or near-term entries |
| Medium | `m` | Entries to revisit after current priorities |
| Long | `l` | Long-term goals, plans, and references |

A capture code combines the horizon first and the type second. For example, `-st` means a short-term to-do, `-mn` means a medium-term note, and `-li` means a long-term idea.

## Installation

### Prebuilt release

Install the latest release with one command:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ART3121/NIT-System/releases/latest/download/install.sh | sh
```

The installer detects the supported platform, downloads the prebuilt binary, verifies its SHA-256 checksum, and installs it to `~/.local/bin/nit`. It does not require Rust or administrator privileges.

Ensure `~/.local/bin` is included in `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Release binaries do not require Rust or Cargo.

To install a specific version or use another destination:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/ART3121/NIT-System/releases/latest/download/install.sh | \
  NIT_VERSION=0.1.0 NIT_INSTALL_DIR="$HOME/bin" sh
```

To uninstall a release installation:

```bash
rm "$HOME/.local/bin/nit"
```

### Build from source

Requirements:

- Rust and Cargo;
- at least one supported terminal editor: Neovim, Vim, Vi, or Nano;
- a terminal with RGB color support for the complete TUI theme.

Clone the repository, enter its directory, and run:

```bash
cargo install --path .
```

Cargo normally installs the executable at `~/.cargo/bin/nit`. If the command is not found, add that directory to `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

To reinstall after updating the source:

```bash
cargo install --path . --force
```

To uninstall:

```bash
cargo uninstall nit-system
```

## Quick start

Open the terminal interface in the current directory:

```bash
nit
```

Capture an entry immediately:

```bash
nit Review the deployment checklist -st
```

List active entries:

```bash
nit list
```

NIT uses the current working directory as its workspace. Running it in different directories creates independent collections:

```text
project-a/.notes
project-b/.notes
```

Run NIT from the same directory whenever you want to access that collection.

## Directory-scoped workspaces

NIT does not force every entry into one global database. The current working directory defines the active workspace, allowing unrelated parts of life and work to remain separate without accounts, profiles, or configuration files.

### General notes in the home directory

Run NIT from the home directory to maintain a general-purpose collection:

```bash
cd "$HOME"
nit Buy a replacement keyboard -st
nit Save an article about personal finance -sx
```

These entries are stored in `$HOME/.notes`. This workspace is useful for everyday ideas, reminders, shopping items, personal references, and anything that does not belong to a specific project.

### Project-specific notes

Run NIT from the root of a project or repository to create an isolated collection for that context:

```bash
cd "$HOME/projects/example-project"
nit Add retry handling to the API client -st
nit Consider a plugin-based architecture -mi
```

The project now has its own files:

```text
example-project/
├── .notes
├── .notes.archive
└── project files...
```

Implementation ideas, technical references, open questions, and project tasks stay close to the work they describe instead of being mixed with general notes. Changing directories naturally changes context:

```text
$HOME/.notes                          # general and personal context
$HOME/projects/example-project/.notes # project context
$HOME/projects/another-project/.notes # another independent context
```

Run NIT from the project root consistently. Running it from a nested directory creates another independent `.notes` file in that directory.

### Private or shared project notes

For private project notes, add the storage files to the project's `.gitignore`:

```gitignore
.notes
.notes.archive
.notes.legacy.*.bak
```

Alternatively, the Markdown files can be committed when a team intentionally wants to share project context through the repository. NIT does not impose either policy; each workspace can be private or version-controlled independently.

## Fast capture

The fastest input format is:

```text
nit <text> -<horizon><type>
```

Examples:

```bash
nit Explore a new caching strategy -si
nit Summary of the architecture meeting -mn
nit Kubernetes documentation -mx
nit Learn container orchestration -lt
```

All supported codes:

| Horizon | Idea | Note | Item | To-do |
|---|---:|---:|---:|---:|
| Short | `-si` | `-sn` | `-sx` | `-st` |
| Medium | `-mi` | `-mn` | `-mx` | `-mt` |
| Long | `-li` | `-ln` | `-lx` | `-lt` |

Quotes are not required. NIT joins everything before the final capture code into one entry. If no code is supplied, the entry defaults to a short-term to-do:

```bash
nit Buy coffee
```

An unknown three-character code produces an error with the accepted values.

## Command reference

### Open the TUI

```bash
nit
nit tui
```

### Create entries with explicit subcommands

```bash
nit idea long Explore event-driven architecture
nit note medium Meeting summary
nit item short API documentation
nit todo long Learn Kubernetes
```

The optional horizon is one of `short`, `medium`, or `long`. Fast capture codes are recommended for everyday use.

### List entries

```bash
nit list
nit list idea
nit list idea long
nit list --archived
nit list todo short --archived
```

The output uses the same structured Markdown format as the storage files.

### Show an entry

```bash
nit show deployment checklist
nit show Kubernetes --archived
```

NIT first looks for a case-insensitive exact match. If no exact match exists, it searches for a text fragment. The query must identify exactly one entry.

### Edit an entry

```bash
nit edit deployment checklist
nit edit Kubernetes --archived
```

NIT starts the first available editor in this order:

```text
nvim → vim → vi → nano
```

Save and close the editor to update the entry. Empty content is rejected.

### Archive an entry

```bash
nit archive Buy coffee
```

The entry is moved from `.notes` to `.notes.archive`. Any entry type can be archived.

### Import a file

```bash
nit import path/to/notes.md
```

See [Importing notes](#importing-notes) for the accepted structure and backup behavior.

### Display help and version information

```bash
nit --help
nit --version
nit list --help
```

## Terminal interface

Run `nit` without arguments to open the TUI. The interface includes:

- a header showing the active filters and current view;
- an **Entries** column with type-based colors and the current position;
- a synchronized **Class** column showing `horizon/type`;
- a **Selected** panel with the full entry text;
- a command and status line at the bottom.

When the number of entries exceeds the available height, the list scrolls automatically to keep the current selection visible. The **Entries** and **Class** columns always use the same scroll offset.

### Navigation and filters

| Key | Action |
|---|---|
| `↑` / `k` | Select the previous entry |
| `↓` / `j` | Select the next entry |
| `1` | Show all entry types |
| `2` | Show Ideas only |
| `3` | Show Notes only |
| `4` | Show Items only |
| `5` | Show To-dos only |
| `h` | Show all horizons |
| `s` | Show short-term entries |
| `m` | Show medium-term entries |
| `l` | Show long-term entries |
| `r` | Reload the current storage file |

### Command-line capture inside the TUI

Press `:` and enter:

```text
:w New note -sn
```

Press `Enter` to save. The `w` prefix is required. If archived entries are currently displayed, the new entry is saved to the active collection and the TUI returns to the active view.

While entering a command:

| Key | Action |
|---|---|
| `Enter` | Execute `:w` or `:q` |
| `Backspace` | Delete the previous character |
| `Esc` | Cancel the command line without closing NIT |

The TUI only exits through:

```text
:q
```

Pressing `q` by itself or pressing `Esc` outside the command line does not close the application.

### Actions on the selected entry

| Key | Action |
|---|---|
| `Enter` / `e` | Edit the selected entry |
| `c` | Create an entry in the external editor using the active filters |
| `a` | Archive the selected active entry |
| `v` | Switch between active and archived entries |
| `u` | Restore the selected archived entry and return to the active view |
| `dd` | Permanently delete the selected entry |

When no filters are active, `c` creates a short-term to-do. Permanent deletion requires pressing `d` twice. Any different key between those presses cancels deletion.

## Storage

NIT keeps two hidden files in the directory where it runs:

```text
.notes           # active entries
.notes.archive   # archived entries
```

Both files are ordinary Markdown and remain readable without NIT. NIT writes canonical English headings and continues to recognize localized legacy headings when reading older files. A supported document can look like this:

```markdown
# NIT System

## Short Term

### Notes
- Review the service boundaries

### To-dos
- Complete the first prototype

## Long Term

### Ideas
- Build a terminal-focused knowledge system
```

Empty sections are omitted. Entries retain their insertion order within each section. NIT writes to a temporary file before replacing the destination, reducing the chance of leaving partially written data.

The current format stores only the entry type, horizon, and text. It does not yet provide persistent IDs, timestamps, tags, due dates, or automatic synchronization.

## Archiving and deletion

Archiving does not delete data. It moves an entry to `.notes.archive`:

```bash
nit list --archived
```

Inside the TUI, press `v` to view archived entries and `u` to restore the selected entry.

Deletion with `dd` is permanent. NIT does not currently provide a trash directory or automatic recovery, so keep backups when storing important information.

## Importing notes

The importer recognizes:

- `Short`, `Medium`, and `Long Term` headings;
- localized legacy horizon headings for backward compatibility;
- `Ideas`, `Notes`, `Items`, and `To-do`/`To-dos` sections;
- entries beginning with `- `;
- continuation lines indented by at least two spaces.

Import another file into the current collection:

```bash
nit import /absolute/path/to/notes.md
```

When source and destination are different files, imported entries are appended to the current collection. Duplicate entries are not removed automatically.

To normalize the current `.notes` file:

```bash
nit import .notes
```

Before rewriting the same file, NIT creates a backup named `.notes.legacy.<process-id>.bak` in the current directory.

## Manual editing

The storage files can be edited with any text editor as long as the expected structure is preserved:

- a recognized horizon heading appears before its entries;
- a recognized type heading appears inside the horizon section;
- every entry begins with `- `;
- additional lines belonging to the same entry are indented by at least two spaces.

Example of a multiline entry:

```markdown
### Notes
- First line
  Continuation of the same note
```

Content outside recognized sections may be ignored and removed during the next canonical rewrite. Back up the file before experimenting with custom layouts.

If a file is changed while the TUI is open, press `r` to reload it.

## Brand assets

The official banner and square icon are stored in the repository and can be reused when presenting or packaging NIT System.

| Banner | Icon |
|---|---|
| <img src="assets/nit-system-banner.jpeg" alt="NIT System banner" width="560"> | <img src="assets/nit-system-icon.jpeg" alt="NIT System icon" width="220"> |

- [`assets/nit-system-banner.jpeg`](assets/nit-system-banner.jpeg) — 1600×900 project banner.
- [`assets/nit-system-icon.jpeg`](assets/nit-system-icon.jpeg) — 1254×1254 square icon.

## Architecture

The executable is separated from the application core. `main.rs` only starts the library, while each module owns a specific responsibility:

```text
src/
├── main.rs          # executable entry point
├── lib.rs           # core composition
├── cli.rs           # argument parsing and command dispatch
├── commands.rs      # entry operations
├── editor.rs        # external editor selection
├── model.rs         # domain types
├── storage.rs       # Markdown persistence
└── tui/
    ├── mod.rs       # interface state, events, and actions
    └── ui.rs        # interface rendering
```

Internal interfaces use crate-level visibility. This keeps the core testable without exposing implementation details as a stable public API.

## Development

Build the debug executable:

```bash
cargo build
```

Run the test suite:

```bash
cargo test
```

Format the source:

```bash
cargo fmt
```

Build an optimized binary:

```bash
cargo build --release
```

The optimized executable is written to `target/release/nit`.

### Publishing a release

Release tags must match the version in `Cargo.toml`. To publish version `0.1.0` after the repository has been configured:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow runs the tests, builds the optimized Linux x86-64 binary, generates a SHA-256 checksum, embeds the repository address into the installer, and publishes all three files on the GitHub Release. GitHub supports stable links to assets from the latest release, which allows the installation command to remain unchanged across versions.

## Troubleshooting

### `nit: command not found`

Check whether Cargo installed the executable:

```bash
ls "$HOME/.cargo/bin/nit"
```

Add the containing directory to `PATH` and start a new shell.

### Entries do not appear

Check the current directory:

```bash
pwd
```

Each directory has its own `.notes` collection. Run NIT from the directory containing the expected file.

### A text query is ambiguous

Entries with similar text require a more specific query:

```bash
nit show deployment checklist for staging
```

TUI actions operate on the selected entry and do not require unique text.

### The terminal remains in an altered state after interruption

If the process is terminated externally while the TUI is using raw mode, restore the terminal with:

```bash
reset
```

### An imported file is not recognized

Verify that the file contains a recognized horizon heading, a recognized entry-type heading, and entries beginning with `- `. Files without recognized entries are rejected.
