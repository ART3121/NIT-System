# NIT System Documentation

This directory documents NIT System as a composition of focused products and
internal adapters. Start with philosophy to understand the constraints, then
use architecture to follow dependencies and runtime flows.

## Reading order

1. [Philosophy](philosophy.md) — why NIT is local-first, human-first, and
   influenced by Unix design.
2. [Architecture](architecture.md) — how every component depends on and
   collaborates with the others.
3. [NIT Core](core.md) — the domain, workspace, and persistence boundary.
4. [NIT CLI](cli.md) — immediate commands, script behavior, and dispatch.
5. [NIT TUI](tui.md) — interactive state and composed management features.
6. [NIT Cat](nitcat.md) — standalone and embedded Markdown viewing.
7. [NIT AI](ai.md) — optional Ollama Roadmap generation.
8. [NIT Editor](editor.md) — shared delegation to terminal editors.
9. [Changelog](../CHANGELOG.md) — release highlights, compatibility notes, and
   migration-impacting changes.

## Architectural contract at a glance

```text
interfaces and adapters
        ↓ intentions, text, and typed proposals
     NIT Core
        ↓ validated persistence
       .nit/
```

Core is the only authoritative owner of domain rules and workspace persistence.
Interfaces own interaction. Adapters own communication with external tools.
The filesystem owns durable data.

The root [README](../README.md) remains the complete user guide, including
installation, commands, keybindings, storage, migration, release, and
troubleshooting.
