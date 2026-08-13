# NIT Cat

NIT Cat is the focused terminal reader distributed as the `nitcat` executable.
It is both an independent Markdown viewer and the reusable viewing engine
embedded by NIT TUI.

This dual role is deliberate: the capability is useful on its own, and the NIT
System composes it instead of maintaining a second renderer.

## Two source modes

### Ordinary Markdown file

```bash
nitcat README.md
```

An ordinary path requires no NIT workspace and is always read-only. NIT Cat
reads the file, parses Markdown events, produces terminal lines, and maintains
only viewer state such as scroll and search matches.

### NIT Note ID

```bash
nitcat N-0001
```

An ID is resolved through `Nit::discover` and `Nit::find_by_id` across active
and archived Notes. The Note body and optional Roadmap are presented as one
Markdown document. Editing with `e` is enabled because Core can safely persist
the resolved Note.

Note-ID resolution currently targets Plain `.nit/` discovery. NIT Cat does not
connect to the Vault Session Agent. Vault Notes remain readable inside the TUI,
which uses `NitApi` through the active session. Ordinary Markdown files remain
independent of either backend.

Recognized IDs take precedence over paths. Use `./N-0001` when a real filename
resembles an ID. IDs belonging to Ideas, Items, or To-dos are rejected because
NIT Cat's NIT-aware contract is specifically long-form Note reading.

## Internal separation

NIT Cat contains three focused layers:

| Layer | Responsibility |
|---|---|
| `markdown` | Convert Pulldown-Cmark events into styled, wrapped Ratatui lines |
| `ViewerState` | Track scrolling, viewport size, query, matches, and selected match |
| `terminal` | Resolve sources, own Crossterm lifecycle, process events, and draw the standalone reader |

The Markdown renderer and `ViewerState` do not need to know about `.nit/`, IDs,
archives, Ollama, or the NIT tree. Source resolution is kept at the terminal
edge, which allows the TUI to reuse the generic layers.

## Relationship to Core and Editor

For a normal file, NIT Cat uses neither Core persistence nor Editor. For a Note
ID, Core supplies the source and identity. If the user presses `e`, NIT Editor
collects the edited document and Core performs the durable save.

This edit route is Plain-only. A Vault Note is never exported to NIT Editor's
plaintext temporary buffer.

NIT Cat never allocates IDs, archives entries, generates AI content, or exposes
the workspace navigator. Those responsibilities remain with the management
interfaces.

## Interaction model

NIT Cat supports scrolling, paging, Markdown styling, search, match navigation,
reload, mouse-wheel input, contextual Help, and safe terminal restoration. Its
small command surface reflects the Unix-oriented goal of a dedicated reader
rather than a second full NIT manager.

Rendered Markdown is cached by source revision, width, and search query. A
normal frame clones only the visible viewport instead of parsing and allocating
the complete document again. Scrolling slices by `usize`, so it is not capped
at the terminal widget's 16-bit scroll coordinate. Input files are bounded to
32 MiB to keep accidental or hostile inputs from exhausting memory.

## Shell completion

Use:

```bash
nitcat -completions bash
nitcat -completions zsh
nitcat -completions fish
```

Completion combines filesystem paths with Note IDs from the nearest Plain
workspace.
Dynamic ID lookup is best-effort: outside a workspace, file completion still
works and no workspace is created.

See [Architecture](architecture.md) for standalone and embedded viewer flows.
