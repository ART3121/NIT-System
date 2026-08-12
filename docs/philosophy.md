# NIT Philosophy

NIT System exists to shorten the path between thinking, recording, finding,
and developing information. It is not a prescribed productivity method. It is
a small set of terminal tools that gives ordinary text enough structure to
remain useful without making maintenance the main activity.

## Human-first notes

The primary unit in NIT is an entry a person can recognize at a glance. IDs
such as `N-0001`, `ST-0007`, and `LI-0012` expose classification and remain
easy to type, read aloud, copy, and remember. They deliberately replace opaque
random identifiers.

The four entry types answer different human questions:

| Type | Question |
|---|---|
| Note | What knowledge or context should be preserved? |
| Idea | What possibility may deserve exploration? |
| To-do | What concrete action should happen? |
| Item | What resource or reference should remain available? |

Only Ideas and To-dos have time horizons. Notes and Items are timeless because
knowledge and references do not naturally become short-, medium-, or long-term
objects merely by being recorded.

## Local-first by construction

Local-first is not an offline mode added to a remote service. NIT's source of
truth is the `.nit/` directory on the user's filesystem. There is no account,
database server, mandatory synchronization provider, or NIT daemon.

This gives the user several practical guarantees:

- data remains available without a network connection;
- ordinary filesystem tools can inspect, copy, diff, back up, or version it;
- storage remains readable even if the NIT executable is unavailable;
- Git can be used, but Git is not required;
- optional integrations cannot become prerequisites for basic note access.

## Structure without ceremony

NIT applies structure at capture time through small codes such as `-n`, `-st`,
and `-li`. Operations begin with `-`, leaving ordinary words available as note
text. Workspace discovery follows the current directory upward, so context is
selected by location rather than by a global account or a constantly managed
configuration.

Archiving is intentionally neutral. An archived entry is outside the active
view; it is not automatically declared completed, obsolete, or successful.

## Relationship to the Unix philosophy

NIT follows Unix ideas as engineering guidance rather than as branding. The
parallel is strongest in responsibility, composition, textual storage, and the
absence of hidden resident state.

| Unix-oriented idea | NIT interpretation |
|---|---|
| Make each program do one thing well | `nit` manages entries; `nitcat` reads Markdown and Notes; Core owns rules; adapters own external integrations. |
| Build tools that work together | CLI, TUI, Cat, AI, and Editor compose through explicit Rust APIs and filesystem paths. Shell-visible commands expose useful stdout where appropriate. |
| Prefer text as a universal interface | Durable data is Markdown or compact human-readable text, not an opaque database. |
| Separate mechanism from policy | Core enforces storage and identity rules; interfaces decide how users interact with those rules. |
| Avoid needless global state | Workspaces are directory-scoped, model loading belongs to Ollama, and NIT runs no background daemon. |
| Make behavior observable | IDs, paths, workspace roots, storage files, errors, and migrations are explicit. |
| Compose instead of duplicating | The TUI embeds NIT Cat's Markdown engine and both interfaces reuse the same Editor adapter and Core API. |

### Intentional differences

NIT is not a literal collection of stdin-to-stdout filters. A full-screen TUI
needs terminal state, interactive navigation, and event handling. The crates
also exchange typed Rust values rather than reparsing text between every
internal boundary. This is intentional: Unix-style separation does not require
throwing away type safety or forcing an interactive program into a pipeline
model that does not fit it.

The project also ships one coordinated version. The modules are independently
bounded but released together so their internal API contracts cannot drift.
Modularity here means replaceable responsibilities and controlled dependencies,
not release fragmentation for its own sake.

## Optional power, mandatory simplicity

AI may interpret a selected entry, but capture, search, storage, editing, and
reading never depend on AI. The external editor makes long-form changes more
comfortable, but files remain manually editable. The TUI improves discovery,
but the CLI remains sufficient for scripted and rapid operations.

A feature belongs in NIT when it preserves these properties:

1. it reduces friction in capturing, finding, reading, or developing entries;
2. its responsibility has a clear owner;
3. failure does not silently damage user data;
4. it does not make an optional subsystem mandatory;
5. it preserves understandable local storage;
6. it does not turn NIT into a resident platform when a small tool is enough.

These constraints are the project's long-term defense against becoming a
monolithic note environment.
