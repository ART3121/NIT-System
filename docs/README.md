# NIT System Documentation

This directory documents the current NIT System architecture, including its
two persistence modes: human-readable Plain Storage and encrypted Vault
Storage. NIT Drive is the removable-media lifecycle built around a mandatory
Vault; NIT Session is the local, ephemeral bridge that lets CLI, TUI, and
desktop clients reuse one unlock.

## Reading order

1. [Philosophy](philosophy.md) — human-first, local-first, Unix-oriented design.
2. [Architecture](architecture.md) — crates, dependencies, runtime flows, and
   trust boundaries.
3. [NIT Core](core.md) — domain API and the Plain/Vault persistence boundary.
4. [Vault format](vault.md) — cryptography, authenticated records, commits,
   integrity, and limitations.
5. [Session Agent](session.md) — IPC, key lifetime, locking, and removal.
6. [NIT Drive](NIT_DRIVE.md) — removable discovery, provisioning, format, and
   operational safety.
7. [NIT CLI](cli.md) — capture, workspace selection, unlock, lock, and status.
8. [NIT TUI](tui.md) — interactive state over the shared `NitApi`.
9. [NIT Cat](nitcat.md) — standalone and embedded Markdown viewing.
10. [NIT AI](ai.md) and [NIT Editor](editor.md) — optional adapters.
11. [Changelog](../CHANGELOG.md) — release and unreleased changes.

## Architectural contract at a glance

```text
CLI / TUI / Desktop
        │
        ├── Plain ────────> Nit ──> .nit/ text files
        │
        └── Vault ─> Session Client ─IPC─> Session Agent
                                          └── Nit ──> encrypted Vault

removable device ──> NIT Drive metadata ──> mandatory Vault
```

Core remains the only owner of domain rules. Plain and Vault vary only at the
persistence boundary. The Session Agent owns unlocked key material in memory;
it is not a database, network service, synchronization service, or source of
domain truth.

The root [README](../README.md) is the user guide. These documents provide the
deeper operational and architectural contracts.
