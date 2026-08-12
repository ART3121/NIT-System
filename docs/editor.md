# NIT Editor

`nit-editor` is the shared external-editor adapter used by NIT CLI, NIT TUI,
and NIT Cat. It has one responsibility: turn initial text into edited text by
delegating interaction to an installed terminal editor.

## Operation

1. Create a randomly named private Markdown buffer with automatic cleanup.
2. Write the caller's initial text.
3. Try `nvim`, `vim`, `vi`, and `nano` in that order.
4. Wait for the first available editor to exit successfully.
5. Read and trim the result.
6. Reject an empty result.
7. Drop the temporary buffer and return the text.

The buffer is created through the operating system's secure temporary-file
facility instead of a predictable PID-based path. On Unix it is created with
owner-only permissions, preventing another local account from reading an entry
while it is being edited.

The adapter does not know whether the text represents a Note, Idea, Item,
To-do, or ordinary document. It does not discover a workspace or save durable
data. Each caller parses the returned text according to its use case and asks
Core to persist any accepted change.

## Why it is separate

Keeping process discovery outside Core prevents editor preferences and terminal
process errors from entering the domain layer. Keeping it shared prevents CLI,
TUI, and NIT Cat from developing different fallback orders or empty-edit
behavior.

This is a small Unix-oriented adapter: NIT delegates text editing to an existing
specialized program instead of recreating an editor inside every interface.
