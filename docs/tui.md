# NIT TUI

`nit-tui` is the interactive management interface of NIT System. It is a Rust
library invoked by `nit`; it intentionally does not install a second executable.
This makes the TUI one presentation of the same `Nit` application context used
by the CLI, rather than a separate program with separate storage behavior.

## Responsibility

The TUI owns transient interaction state:

- focus between navigator, entry list, viewer, command line, and AI panel;
- selected rows, filters, active/archive view, and scrolling;
- terminal layout, colors, wrapping, keyboard, and mouse handling;
- command-mode input and confirmation dialogs;
- background AI progress and proposal review;
- integration of the reusable Markdown viewer into NIT context.

The TUI does not own ID allocation, parsing, workspace discovery, persistence,
or migration. Those operations remain in Core.

## Session lifecycle

1. NIT CLI discovers and opens one workspace.
2. It passes `&Nit` to `nit_tui::run`.
3. The TUI loads active and archived collections through that facade.
4. UI actions update in-memory state immediately.
5. Persistent actions call Core under a workspace lock and refresh the relevant
   view. If another process changed the loaded snapshot, Core refuses the stale
   save instead of overwriting it.
6. On exit, Crossterm restores terminal state; only accepted Core mutations
   remain durable.

Because the workspace is supplied by the caller, active entries, archive data,
the tree, and the viewer cannot accidentally resolve against different
directories during one session.

## Browser and tree

The tree expresses the storage-oriented organization of the selected workspace.
The browser expresses the currently filtered collection. Hiding the tree changes
layout only; it does not create another data view. Selection, filters, and
scrolling belong to the TUI and are never serialized into `.nit/`.

## Embedded Note Viewer

The TUI reuses NIT Cat's generic Markdown renderer and `ViewerState`. It does
not fork or copy that implementation. Around the generic viewer it adds:

- workspace tree visibility;
- restoration of the previous browser context;
- selected Note identity;
- NIT command mode;
- archive and editing actions;
- Roadmap and AI integration.

This is an example of composition within the workspace: NIT Cat can remain a
focused reader while the TUI can embed the same capability in a larger manager.

## Editing

When an action requires an external editor, the TUI calls `nit-editor`. The
adapter returns text; the TUI interprets the edited Note or entry and asks Core
to save it. Editor discovery and storage writing therefore remain separate.

## AI panel

The TUI sends a selected Core `Entry` to `nit-ai` on a background worker and
keeps rendering progress. It polls for a structured proposal, then asks the user
to accept with `Y` or reject with `N`/`Esc`. Only acceptance calls Core to attach
the Roadmap. A timeout, cancellation, invalid response, or rejection leaves the
workspace unchanged.

## Terminal safety

The event loop handles only key-press events for ordinary shortcuts, separates
modified keys from unmodified actions, and treats `Ctrl+C` explicitly. Raw mode,
the alternate screen, cursor visibility, and mouse capture are owned by the
terminal session rather than by Core or the viewer model. The session uses an
RAII cleanup guard, so setup, drawing, editor, or event errors still make a
best-effort restoration. Starting the TUI without an interactive terminal
returns a direct diagnostic instead of a low-level device error.

## Dependency boundary

`nit-tui` may depend on Core and focused adapters. Core must never depend back
on the TUI. The TUI also must not import private repository or storage modules;
all durable operations return through `Nit`.

See [NIT Cat](nitcat.md) for the reusable viewer boundary and
[Architecture](architecture.md) for complete module composition.
