# NIT CLI

NIT CLI is the immediate and scriptable interface exposed by `nit`. Argument
parsing and dispatch live in `nit-cli`; durable domain operations always return
through `NitApi`.

## Capture

```bash
nit Capture this thought -n
nit Fix transaction recovery -st
nit Explore portable storage -li
```

Supported codes are `-si`, `-mi`, `-li`, `-st`, `-mt`, `-lt`, `-n`, and `-x`.
Operation names begin with `-`, so ordinary words remain capture text.

## Workspace selection

For domain actions, the CLI checks the local Session Agent first:

1. `Unlocked` — use the Vault workspace through IPC;
2. `Unavailable` — fail and require reconnect plus unlock;
3. `Locked` or no agent — discover the nearest Plain `.nit/`.

This ordering is a safety boundary. A removed NIT Drive never causes the CLI to
save into a nearby Plain workspace. Explicit Plain initialization and legacy
migration remain separate administrative operations.

## Vault session commands

```bash
nit -unlock <drive-path> <workspace-id>
nit -session-status
nit -lock
```

`-unlock` expects the mounted NIT Drive root, not its internal Vault directory.
The CLI starts the Session Agent on demand, prompts without echoing the password,
opens the Drive, authenticates its Vault binding, and selects the supplied
32-character workspace ID.

After unlock, normal commands and `nit -tui` reuse the same session:

```bash
nit A portable note -n
nit -ls
nit -search portable
nit -status
nit -tui
```

`nit -status` identifies a Vault workspace without exposing a host storage
path. `nit -root` and `nit -path` fail for Vault because paths are not the
workspace identity and object names are intentionally opaque.

`nit -lock` discards the active `Nit` and Master Key. If no Agent is running,
it still reports a locked state. `nit -session-status` reports:

- Agent not running;
- Vault locked;
- Vault unlocked, including Vault/workspace identity;
- Drive unavailable and requiring a new unlock.

## Command reference

| Command | Purpose |
|---|---|
| `nit` / `nit -tui` | Open the TUI for the selected Plain/Vault context |
| `nit <text> <code>` | Create an entry |
| `nit -ls [code] [--archived]` | List entries |
| `nit -search <query> [code] [--archived\|--all]` | Search entries |
| `nit -show <query> [--archived]` | Display one entry |
| `nit -edit <query> [--archived]` | Edit one Plain entry; disabled for Vault |
| `nit -archive <query>` | Archive an entry |
| `nit -import <path>` | Import a compatible collection |
| `nit -status` | Show selected context and counts |
| `nit -root` / `nit -path` | Print Plain paths; unavailable for Vault |
| `nit -init [--private\|--tracked]` | Explicitly create Plain Storage |
| `nit -migrate` | Explicitly migrate legacy Plain storage |
| `nit -assign-ids` | Plain-only ID maintenance |
| `nit -migrate-timeless` | Plain-only legacy ID migration |
| `nit -drive-create` | Discover, explicitly select, format, and initialize a NIT Drive |
| `nit -drive-create --dry-run <device-id>` | Preview a validated plan without running commands |
| `nit -drive-create --initialize <device-id> <mount>` | Initialize formatted and mounted media without reformatting |
| `nit -unlock <drive> <workspace-id>` | Unlock/reuse a NIT Drive session |
| `nit -lock` | Destroy the active Vault session |
| `nit -session-status` | Inspect session state |
| `nit -ai-roadmap <ID>` | Generate and review a local Roadmap |
| `nit -completions <bash\|zsh\|fish>` | Generate completion definitions |

Plain ID-maintenance (`-assign-ids` and `-migrate-timeless`) refuses to run
while a Vault session is active or an active Drive is unavailable. Explicit
`-init` and legacy `-migrate` always target the current local directory; they are
deliberate administrative operations, not a fallback selected for a failed
Drive action.

## Session Agent startup

The same `nit` executable has a hidden internal mode used to host the Agent.
Users should not invoke it directly. The CLI starts it with detached standard
streams, waits for its local endpoint, and then sends the unlock request. The
separate `nit-session-agent` binary exists for integration/development, but it
does not represent a second NIT product or database.

## Output and composition

Normal output goes to stdout; errors use a failing exit status. Broken pipes are
treated normally. NIT favors visible text and explicit errors, but it does not
promise a stable JSON schema for all commands.

Shell completion remains available for Bash, Zsh, and Fish. Static completion
covers session commands; dynamic entry-ID completion follows the currently
selected context through the same CLI dispatch rules.

See [Session Agent](session.md) and [Architecture](architecture.md).
