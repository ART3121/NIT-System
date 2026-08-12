# Changelog

All notable changes to NIT System are documented here. NIT follows a coordinated
version: the `nit` and `nitcat` executables and every workspace crate are released
from the same source revision.

## [0.4.0] - 2026-08-11

Version 0.4 turns NIT into a modular terminal note-management system while
preserving immediate capture, local ownership, and human-readable storage.

### Added

- A Cargo Workspace split into NIT Core, CLI, TUI, NIT Cat, AI, and Editor
  components with explicit responsibility boundaries.
- `nitcat`, a standalone terminal Markdown viewer that also opens Notes by ID
  and provides the reusable viewer embedded by the NIT TUI.
- Per-Note Markdown documents, a workspace tree, in-TUI Note viewing, search,
  tree visibility controls, and focused Note editing.
- Human-readable IDs for every entry: timeless `N-...` and `X-...` IDs for
  Notes and Items, plus horizon-aware Idea and To-do IDs.
- Shell completion for Bash, Zsh, and Fish in both `nit` and `nitcat`.
- Optional local Ollama Roadmap generation with progress, review, acceptance,
  cancellation, bounded requests, and explicit failure handling.
- Cross-platform CI for Linux, macOS, and Windows source builds and tests.
- Dedicated documentation for philosophy, architecture, Core, CLI, TUI,
  NIT Cat, AI, and Editor responsibilities.

### Changed

- Notes and Items are now timeless; horizons apply only to Ideas and To-dos.
- `nit -ls` replaces `nit -list` and prints Note IDs and titles while retaining
  the established summary behavior for the other entry types.
- `nitcat <NOTE-ID>` replaces the former `nit -v <NOTE-ID>` viewer command.
- Version 0.2 workspace data is migrated to typed collections and individual
  Note files only after validation, with retained backups.
- The release installer now installs both products and their completion files,
  with optional component selection.
- TUI lists, Markdown views, wrapping, scrolling, Help, AI selection, and
  command input were reorganized for large collections and narrow terminals.

### Reliability and security

- Workspace readers use shared locks and mutations use exclusive locks, so ID
  allocation and multi-file updates remain consistent across NIT processes.
- Mutations use bounded reads, private same-filesystem temporary files, atomic
  replacement, a recoverable transaction journal, and post-write validation.
- Stale frontend snapshots are rejected instead of overwriting newer changes.
- Structural Markdown headings are parsed exactly; entry text containing words
  such as `notes`, `ideas`, or `short term` is no longer mistaken for metadata.
- Administrative workspace symlinks and oversized storage/viewer inputs are
  rejected explicitly.
- Ollama access is restricted to loopback endpoints and applies finite process,
  request, header, and response limits.
- CLI and viewer output handle broken pipes normally, and terminal state is
  restored through scoped guards.
- The dependency graph was updated and is checked with RustSec during CI and
  release validation.

### Performance

- TUI entry wrapping is virtualized to the visible viewport.
- NIT Cat and the embedded viewer cache rendered Markdown by source revision,
  width, and search query, cloning only visible lines during normal frames.
- Search avoids rebuilding a combined document for every entry.
- Ollama calls disable normal-operation thinking, use bounded context/output,
  and keep the model available briefly for nearby requests.

### Compatibility notes

- Prebuilt release archives currently target Linux x86-64. Source builds are
  continuously checked on Linux, macOS, and Windows.
- Rust 1.88 or newer is required when building from source.
- Workspaces from version 0.2 remain supported through guarded migration; read
  the [migration guide](README.md#legacy-migration) before manual layout edits.

## [0.2.0] - 2026-08-10

- Introduced `.nit/` workspaces, hierarchical discovery, explicit
  initialization, legacy migration, human-readable entry IDs, the interactive
  TUI, local Roadmap generation, and the first installer-based release flow.

## [0.1.0] - 2026-08-06

- Initial public release of NIT System with terminal-first capture and
  human-readable local storage.

[0.4.0]: https://github.com/ART3121/NIT-System/compare/v0.2.0...v0.4.0
[0.2.0]: https://github.com/ART3121/NIT-System/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ART3121/NIT-System/releases/tag/v0.1.0
