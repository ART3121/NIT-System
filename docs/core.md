# NIT Core

`nit-core` is the domain and persistence foundation shared by every NIT System
interface. It is a Rust library, not an executable, HTTP API, daemon, database,
or background process.

## Why Core exists

Without Core, the CLI, TUI, and NIT Cat could each interpret IDs, paths, and
storage differently. Core gives every frontend one authoritative definition of
a workspace and one validated route to durable data.

Its public `Nit` facade is the system's internal application API:

```rust
let nit = nit_core::Nit::discover()?;
let active = nit.load(nit_core::View::Active)?;
let id = nit.create(nit_core::Kind::Note, None, "Architecture".into())?;
```

The API is public between workspace crates in version 0.4, but it is not yet
published to crates.io and does not carry an external stability guarantee.

## Responsibilities

Core owns:

- discovery and initialization of directory-scoped `.nit/` workspaces;
- workspace paths and nearest-ancestor selection;
- the `Entry`, `EntryId`, `Kind`, `Horizon`, `Roadmap`, and `View` models;
- classification validation and human-readable ID allocation;
- active and archived collection loading;
- parsing and canonical serialization;
- search across titles, bodies, IDs, and Roadmaps;
- create, archive, import, migration, and Roadmap attachment operations;
- layout, uniqueness, and cross-file integrity validation;
- preservation rules for legacy migration and backups.

Core does not own terminal rendering, keybindings, command-line syntax, Ollama
requests, confirmation prompts, or editor process selection.

## Internal modules

| Module | Purpose |
|---|---|
| `model.rs` | Domain values and classification rules |
| `workspace.rs` | Workspace discovery, creation, paths, privacy helpers, and legacy migration |
| `repository.rs` | Layout-aware persistence, global validation, search, and storage-layout migration |
| `storage.rs` | Text parsing and serialization for compact collection files |
| `ids.rs` | Per-class counters and collision prevention |
| `commands.rs` | Domain operations composed from repository, storage, and IDs |
| `fsutil.rs` | Workspace locks, bounded reads, private temporary files, and durable atomic replacement |
| `lib.rs` | The public `Nit` facade and exported types |

Only the facade and selected domain types cross the crate boundary. Repository,
storage, and ID-sequence implementations stay private so callers cannot bypass
invariants with a low-level write.

## Workspace lifecycle

`Workspace::discover` starts at the current directory and walks toward the
filesystem root. The nearest `.nit/` directory wins. Discovery never creates a
workspace as a side effect; initialization is an explicit command.

Opening a `Nit` instance validates the current layout and may perform a safe,
known layout migration. Legacy `.notes` migration and ID migrations remain
explicit because they have user-visible consequences.

## Persistence contract

Core treats the filesystem as the durable source of truth. Notes are individual
Markdown documents. Ideas, Items, and To-dos use compact text collections.
Active and archived data are separate but validated together so an ID cannot
silently exist twice.

Before a collection is saved, Core validates its classifications, IDs, Roadmap
shape, and destination layout. Interfaces receive an error rather than a
partially interpreted success.

Readers use a shared workspace lock and mutations use an exclusive lock. ID
allocation, reload, mutation, and persistence therefore cannot race with
another NIT process. The `Nit` facade also remembers the collection snapshot
given to a frontend and refuses to save it after another process has changed
the same view. Multi-view mutations validate both collections, verify the
result after writing, and restore the previous data when an ordinary write
failure occurs.

Before a mutation, Core creates a lightweight write-ahead snapshot in
`.nit/.transaction/`, using same-filesystem hard links when available. A later
process automatically restores that snapshot if the previous process was
interrupted before commit. The journal is removed only after validation and
durability steps complete.

Storage reads are bounded, replacement files are private random temporary
files in the destination filesystem, and durable writes synchronize the file
and parent directory on Unix. Symbolic links are rejected for administrative
workspace paths rather than followed implicitly.

## How other modules use Core

- **CLI** translates arguments into `Nit` calls and formats results.
- **TUI** loads collections through `Nit`, keeps only transient interface state,
  and sends persistent actions back through the facade.
- **NIT Cat** uses Core only when an argument is a Note ID; ordinary Markdown
  files remain independent of a workspace.
- **AI** accepts Core's `Entry` type and returns Core's `Roadmap` type, but has
  no permission to persist it.
- **Editor** has no Core dependency at all; its caller decides how returned text
  maps to a domain value.

## Dependency rule

`nit-core` must not depend on Ratatui, Crossterm, Pulldown-Cmark, Ollama, an
external editor, or interface-specific state. This inward dependency direction
is the architectural rule that keeps NIT from becoming a monolith.

See [Architecture](architecture.md) for complete runtime flows and
[Philosophy](philosophy.md) for the relationship between this boundary and Unix
design principles.
