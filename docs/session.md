# NIT Session Agent

NIT Session lets independent CLI, TUI, and desktop processes share one unlocked
Vault without persisting its Master Key. It is local, ephemeral, and activated
only for Vault Storage. Plain Storage does not need it.

## Lifecycle

```text
first Vault command
  └── password prompt
      └── Agent opens NIT Drive + Vault
          └── Master Key and Nit remain in Agent memory
              ├── CLI client
              ├── TUI client
              └── Desktop client

nit -lock or physical removal
  └── drop Nit/Vault
      └── zeroize owned key material
          └── subsequent operations require unlock
```

The Agent stores only one active session: Vault identity, selected workspace,
the in-memory `Nit`, removal state, and minimal connection metadata. It does not
store entries separately, keep a database, use TCP/HTTP, access the internet,
or persist key material.

## Session states

| State | Meaning |
|---|---|
| `Locked` | Agent exists but owns no unlocked Vault |
| `Unlocked` | One Vault/workspace is usable through `NitApi` |
| `Unavailable` | The unlocked backing path/device disappeared; old key is invalidated |

`Unavailable` is intentionally distinct from `Locked`. It prevents clients
from falling back to a local `.nit/` after a NIT Drive disappears. Reinsertion
never restores the old state; the user must provide the password again.

## IPC

Requests and responses are bounded, newline-delimited JSON carrying a protocol
version. The protocol exposes domain operations, not raw Master Keys or storage
files. The Agent validates the version, executes through `Nit`, and returns
typed results or explicit errors.

### Linux and Unix

- Unix Domain Socket in `/tmp/nit-session-<effective-uid>/`;
- runtime directory must be owned by the actual effective UID with mode `0700`;
- socket mode is restricted to `0600`;
- both server and client verify peer effective UID from kernel credentials;
- a stale socket can be reclaimed, but a regular file/symlink is never deleted;
- only one live Agent can bind the endpoint.

The endpoint does not trust `$UID` for authorization.

### Windows

- local Named Pipe through the same transport abstraction;
- both sides obtain the peer process ID from the pipe;
- process tokens are opened with query-only access;
- Windows SIDs are compared before a request is accepted;
- no TCP port, HTTP server, or remotely addressed pipe is created.

Platform-specific transport code is isolated in `transport.rs`; lifecycle and
domain protocol logic are shared.

## Secrets and messages

The CLI reads passwords without terminal echo. A password crosses IPC only in
the unlock request, is moved immediately into `SecretString`, and input/output
JSON buffers are zeroized after use. The KEK exists only during Vault opening.
The Agent owns the resulting `Nit`/Vault until lock, replacement, removal, or
shutdown.

The protocol may carry plaintext entries because frontends must display and
edit in memory. IPC is restricted to the same local OS user. No plaintext
domain database or cache is written by Session.

## Concurrency

Multiple clients reuse one Agent. Client-specific snapshots preserve optimistic
save semantics: a stale client cannot overwrite a newer change. Each domain
operation ultimately uses the Vault lock and repository transaction.

Malformed clients, disconnected writers, and competing Agent starts do not
terminate the live Agent. An Agent crash loses the session key and requires a
new password; it does not make ciphertext invalid.

## Removal detection

- Linux binds the session to canonical path, mount ID, device number, and mount
  point. A new mount generation at the same path is not the same connection.
- Windows binds it to the canonical path and volume identity. Reusing a drive
  letter with another or reinserted volume does not preserve the session.
- A monitor checks presence every 100 ms and drops the owned `Nit` when the
  token disappears. Synchronous status/operation checks also validate the
  availability marker.

If removal occurs during an active filesystem operation, that operation can
still fail at the OS/storage boundary. The monitor revokes the key as soon as
it can acquire the session state; new operations fail. There is no local save
fallback. Frontends may retain an unsaved draft only in their own memory.

## Commands and integration

```bash
nit -unlock
nit -session-status
nit -lock
```

The CLI starts the same `nit` executable in its hidden Agent mode when needed.
It discovers the mounted Drive and the Agent selects a unique workspace. When
there are multiple candidates, the CLI presents numbered device/workspace names.
`nit -unlock <drive-root> [workspace-id]` remains an advanced explicit form.
Desktop integrations should reuse `SessionClient` and `NitApi`, not duplicate
Vault opening or Session logic.

`SessionClient::unlock_drive` is the normal NIT Drive route. A lower-level
`SessionClient::unlock` can host a Vault located in an ordinary directory for
Core integration and future local-Vault workflows; the current CLI deliberately
exposes only Drive unlock so it cannot confuse an arbitrary directory with a
prepared NIT Drive.

See [NIT CLI](cli.md) and [NIT Drive](NIT_DRIVE.md).
