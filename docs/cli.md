# NIT CLI

NIT CLI is the immediate and scriptable interface exposed by the `nit`
executable. The executable itself is a thin root-package entrypoint; argument
parsing and dispatch live in the `nit-cli` library.

```bash
nit Capture this thought -n
nit -ls -n
nit -search parser --all
nit -show N-0001
```

## Responsibility

The CLI owns:

- recognition of commands, capture text, options, and classification codes;
- stdout/stderr behavior intended for terminal and script use;
- interactive confirmation for destructive or optional actions;
- dispatch to Core, TUI, AI, and Editor capabilities;
- generation of Bash, Zsh, and Fish completion definitions.

It does not parse workspace storage, allocate IDs, validate persisted layouts,
render the full-screen interface, or communicate with Ollama directly.

## Dispatch model

The CLI separates actions that do not need a workspace from those that do:

- help, version, and completion generation run anywhere;
- initialization and legacy migration operate on an explicit directory;
- capture, list, search, edit, archive, status, and AI actions discover the
  nearest workspace once and reuse the resulting `Nit` context;
- no arguments dispatch to `nit_tui::run(&Nit)` instead of starting another
  process.

This keeps workspace selection consistent across one operation. Submodules do
not repeatedly rediscover paths behind the caller's back.

## Capture and operations

Capture remains natural-language-first:

```bash
nit Review architecture boundaries -n
nit Fix parser validation -st
```

Only operation names occupy the leading command position and begin with `-`.
Words such as `list`, `show`, and `archive` remain valid capture text.

`nit -ls` preserves the categorized listing for every entry type. Notes are
condensed to their ID and title so an ID can be copied directly into
`nitcat N-0001`; Note bodies are not printed by the listing command.

## Output and composition

Commands such as `nit -root` and `nit -path` print only the requested path so
shell scripts can consume it. Normal results go to stdout and failures go to
stderr. The CLI favors explicit exit failure over silently selecting an
ambiguous entry.

NIT is not yet a complete text-filter language: not every command accepts stdin
or emits a stable machine schema. Its Unix-oriented contract is narrower and
deliberate—small commands, visible text output, meaningful exit status, and
durable files that other tools can inspect.

## Integration with other modules

| Operation | Modules involved |
|---|---|
| Capture or archive | CLI → Core |
| `nit` / `nit -tui` | CLI → TUI → Core |
| `nit -edit` | CLI → Editor → Core |
| `nit -ai-roadmap` | CLI → Core → AI → user confirmation → Core |
| `nit -ls` | CLI → Core → canonical text renderer |
| Completion IDs | shell definition → hidden CLI query → Core |

The CLI is an orchestrator. Durable mutations still belong to Core, and optional
adapters cannot write storage directly.

## Shell completion

Generate completion definitions with:

```bash
nit -completions bash
nit -completions zsh
nit -completions fish
```

Static completion includes commands, capture codes, filters, and shell names.
Contextual completion queries active and archived IDs from the nearest
workspace only when a command accepts an entry. The release installer places
definitions in the normal user directories; direct generation also supports a
Cargo installation or one shell session.

## Relationship to the TUI and NIT Cat

The `nit` command remains responsible for managing a workspace. With no
arguments it embeds the TUI as a library. Long-form reading is intentionally
available through the separate `nitcat` executable. The removed `nit -v`
command is not retained as a second viewer route.

See [Architecture](architecture.md) for end-to-end command flows.
