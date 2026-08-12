# NIT System Architecture

NIT System is a coordinated Cargo Workspace composed of focused libraries and
two installed executables. The system is the composition; no individual crate
reimplements the whole product.

## Products, libraries, and adapters

| Component | Form | Primary responsibility |
|---|---|---|
| NIT Core (`nit-core`) | Rust library | Domain model, workspace identity, IDs, persistence, validation, migrations, and mutations |
| NIT CLI (`nit-cli`) | Rust library behind `nit` | Argument parsing, scriptable command behavior, stdout/stderr, confirmations, and dispatch |
| NIT TUI (`nit-tui`) | Rust library invoked by `nit` | Interactive browsing, selection, filtering, dialogs, and orchestration |
| NIT Cat (`nitcat`) | Rust library and `nitcat` binary | Focused Markdown/Note viewing and reusable rendering state |
| NIT AI (`nit-ai`) | Internal adapter library | Optional Ollama communication and Roadmap generation |
| NIT Editor (`nit-editor`) | Internal adapter library | External editor discovery, temporary edit buffers, and safe result collection |
| Distribution facade (`nit-system`) | Root package | Builds and ships the `nit` and `nitcat` executables as one release |

Core, CLI, TUI, and NIT Cat are the user-facing architectural products. AI and
Editor are deliberately smaller adapters: they connect optional external
capabilities without moving those concerns into Core.

## Dependency direction

Dependencies point inward toward domain rules and sideways only where a
capability is intentionally reused.

```text
nit binary
└── nit-cli
    ├── nit-core
    ├── nit-tui
    │   ├── nit-core
    │   ├── nitcat      (Markdown renderer + ViewerState)
    │   ├── nit-ai
    │   └── nit-editor
    ├── nit-ai
    └── nit-editor

nitcat binary
└── nitcat
    ├── nit-core        (only when resolving a Note ID)
    └── nit-editor      (only when editing a Note)

nit-ai ───────> nit-core domain types
nit-editor      no NIT domain dependency
nit-core        no terminal, renderer, editor, or Ollama dependency
```

The most important rule is negative: Core does not know that Ratatui,
Crossterm, Pulldown-Cmark, Ollama, Neovim, or even a full-screen interface
exists. It can therefore remain the single source of truth for every frontend.

## Runtime composition

### Fast capture

```text
shell arguments
    ↓
nit executable
    ↓
nit-cli parses text and classification code
    ↓
Nit::discover locates the nearest workspace
    ↓
nit-core validates the classification and allocates an ID
    ↓
repository serializes the correct collection
    ↓
.nit/ human-readable storage
```

The CLI does not calculate IDs or write storage files itself. It translates
the user's command into a Core operation and reports the result.

### Interactive management

```text
nit with no arguments
    ↓
nit-cli dispatches to nit-tui::run(&Nit)
    ↓
TUI loads active/archive data through Core
    ↓
keyboard/mouse actions update TUI state
    ↓
persistent actions return through the Nit facade
```

The TUI owns ephemeral UI state: focus, selection, filters, scroll positions,
dialogs, command input, and background-job presentation. Core owns durable
state. Closing the TUI discards UI state but never changes the meaning of stored
entries.

### Reading a Markdown file

```text
nitcat README.md
    ↓
NIT Cat resolves an ordinary filesystem path
    ↓
Pulldown-Cmark events
    ↓
generic Markdown renderer + ViewerState
    ↓
Crossterm/Ratatui terminal presentation
```

No workspace or Core operation is required for this path. This is what makes
NIT Cat useful as a standalone terminal Markdown viewer.

### Reading a Note by ID

```text
nitcat N-0001
    ↓
NIT Cat recognizes a Note ID
    ↓
Nit::discover + Nit::find_by_id
    ↓
Note body and Roadmap become a Markdown document
    ↓
the same generic renderer displays it
```

ID-shaped input takes precedence over a path. `./N-0001` explicitly selects a
file. Editing is enabled only for Note sources and flows through NIT Editor,
then back through Core persistence.

### Embedded viewing in the TUI

The TUI does not maintain a second Markdown implementation. It imports NIT
Cat's renderer and `ViewerState`, then adds NIT-specific context around them:
the tree, selected entry, command mode, archive actions, and AI panel. The
viewer can evolve independently without allowing two renderers to diverge.

### AI Roadmap generation

```text
selected Entry from Core
    ↓
CLI or TUI requests nit-ai generation
    ↓
nit-ai sends bounded input to local Ollama
    ↓
structured response is parsed and validated
    ↓
user reviews the proposal
    ↓ accept                         ↓ reject/error
Core attaches Roadmap          storage remains unchanged
```

AI never writes a workspace directly. It produces a domain `Roadmap`; a caller
must obtain user acceptance and ask Core to persist it. This keeps an unreliable
text generator outside the trusted persistence boundary.

### External editing

CLI, TUI, and NIT Cat call the same `nit-editor` adapter. It creates a temporary
buffer, tries `nvim`, `vim`, `vi`, and `nano` in order, rejects an empty result,
and returns edited text. The caller interprets that text for its own use case
and Core performs any durable save.

## Core internal layers

Although consumers see the `Nit` facade, Core remains internally divided:

| Internal module | Responsibility |
|---|---|
| `model` | Entry types, horizons, IDs, Roadmaps, and collection values |
| `workspace` | `.nit/` discovery, initialization, paths, privacy helpers, and legacy migration |
| `repository` | Layout-aware loading/saving, cross-file validation, search, and layout migration |
| `storage` | Parsing and canonical serialization of human-readable collection files |
| `ids` | Persistent per-class sequence allocation and reconciliation |
| `commands` | Domain use cases such as create, archive, import, and Roadmap attachment |
| `lib` | Public `Nit` facade and intentionally exported domain surface |

Repository and storage implementations are private. Interfaces cannot bypass
validation by importing a low-level writer.

## State ownership

| State | Owner | Lifetime |
|---|---|---|
| Entries, IDs, Roadmaps, archive placement | `.nit/`, mediated by Core | Durable |
| Workspace location | `Workspace` value | One command/session |
| TUI focus, filters, selection, scroll | NIT TUI | Interactive session |
| Viewer scroll and search matches | NIT Cat `ViewerState` | Viewer session |
| Ollama model residency | Ollama | External process policy |
| Editor temporary buffer | NIT Editor | One edit operation |

This table prevents accidental ownership expansion. For example, AI must not
become a second entry store, and the TUI must not create a private persistence
format for its selection state.

## Error and safety boundaries

- Workspace discovery fails explicitly rather than creating storage during an
  unrelated command.
- Core validates IDs, classifications, layout, and parsed data before writes.
- Migrations preserve backups and avoid overwriting conflicting destinations.
- AI output is schema-validated and is not durable before acceptance.
- Editor failures return an error instead of saving empty data.
- Terminal frontends restore terminal state on their normal error paths.
- Ordinary NIT Cat files are read-only; only a resolved Note is editable.

## Distribution model

All crates use one workspace version and are released together. Release assets
may contain the complete system or an individual executable, but both binaries
are built from the same source revision and Core contract. This offers Unix-like
tool separation without dependency-version drift inside a release.

The Core API is currently workspace-internal in stability terms. It is a Rust
API, not an HTTP service, plugin protocol, daemon, or crates.io compatibility
commitment. Its boundary is nevertheless deliberate so future interfaces can
reuse the same rules without redesigning storage.
