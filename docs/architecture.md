# NIT System Architecture

NIT System is a coordinated Cargo Workspace with one domain model, two storage
backends, focused terminal products, and local adapters. Vault adds encrypted
persistence without replacing or silently migrating Plain `.nit/` workspaces.

## Components

| Component | Form | Responsibility |
|---|---|---|
| `nit-core` | Library | Domain, IDs, `NitApi`, Plain repository, Vault repository, cryptography |
| `nit-cli` | Library behind `nit` | Arguments, output, selection of Plain/Session, prompts, dispatch |
| `nit-tui` | Library invoked by `nit` | Interactive state over `&dyn NitApi` |
| `nitcat` | Library and binary | Markdown viewing and Plain Note-ID resolution |
| `nit-session` | Library and internal agent binary | Same-user local IPC and unlocked Vault lifetime |
| `nit-drive` | Library | Device discovery, removal, provisioning, Drive metadata |
| `nit-ai` | Adapter | Optional loopback Ollama Roadmap generation |
| `nit-editor` | Adapter | External editor delegation for plaintext-capable flows |
| `nit-system` | Root package | Ships `nit` and `nitcat` |

## Dependency direction

```text
nit
└── nit-cli
    ├── nit-core
    ├── nit-session ──> nit-core + nit-drive
    ├── nit-tui ──────> nit-core + nitcat + nit-ai + nit-editor
    ├── nit-ai ───────> nit-core domain types
    └── nit-editor

nit-drive ──> nit-core::vault + NIT Drive lifecycle

nitcat
├── nit-core       (Plain Note-ID resolution)
└── nit-editor
```

Core has no dependency on Session, Drive, terminal UI, Markdown rendering,
Ollama, or editors. Drive does not define an encrypted domain variant. Session
does not implement storage or device formatting.

## Persistence architecture

```text
                         Nit / NitApi
                              │
                    shared domain operations
                              │
                         Repository
                         /        \
                  Plain             Vault
                    │                 │
               Workspace       encrypted catalog
                    │                 │
                  .nit/          Vault objects
```

Plain is human-readable and directory-scoped. Vault encrypts the entire catalog
with stable path-independent workspace IDs. Both use the same `Entry`, IDs,
classification validation, search, archive, import, and Roadmap rules.

## Runtime flows

### Plain capture

```text
nit text -n
  └── Session absent/Locked
      └── discover nearest .nit/
          └── Nit::create
              └── Plain repository + transaction journal
```

No Agent starts for Plain. Existing usage remains unchanged.

### Unlock and Vault operations

```text
nit -unlock <drive> <workspace-id>
  ├── start/reuse local Session Agent
  ├── prompt password
  └── Agent
      ├── NitDrive::open
      ├── Vault::open + authenticate Drive binding
      ├── Nit::open_vault(workspace-id)
      └── retain unlocked Nit in memory

later nit text -n / nit -ls / nit -tui
  └── SessionClient::NitApi over local IPC
      └── Agent Nit
          └── Vault repository transaction
```

The password is required once per physical connection/session, not once per CLI
process. Clients never receive the Master Key.

### Physical removal

```text
mount/volume token disappears
  └── Session monitor drops Nit/Vault
      └── Master Key storage is destroyed/zeroized
          └── state = Unavailable
              ├── no new operations
              ├── no .nit/ fallback
              └── reinsertion requires password
```

### NIT Drive preparation

```text
read-only discovery
  └── select exact removable device
      └── dry-run safety plan
          └── exact ERASE <id> <model> <bytes> confirmation
              └── fresh discovery + validation
                  └── platform formatter (exFAT)
                      └── verify selected device
                          └── initialize .nit-drive/ + Vault + workspace
```

The provisioning and initialization APIs are implemented in `nit-drive`.
The CLI exposes them through the explicit `nit -drive-create` administrative
workflow. It remains separate from capture/workspace dispatch and is never
exercised against real media in CI.

### TUI

The CLI chooses a `NitApi` provider first, then passes `&dyn NitApi` into the
TUI. UI state remains ephemeral. For Vault, all loads and mutations cross IPC;
for Plain, they execute in process. The TUI contains no backend-specific domain
rules.

### AI and external editing

AI receives one selected Core `Entry`, returns an untrusted `Roadmap`, and only
Core persists it after acceptance. It works through `NitApi` in either backend.

The current external editor adapter creates a plaintext temporary buffer. It is
therefore permitted for Plain and deliberately disabled for Vault. This avoids
persisting decrypted Note contents on the host.

NIT Cat can view arbitrary Markdown independently. Note-ID lookup currently
uses Plain discovery rather than the Session protocol.

## Ownership and lifetime

| State | Owner | Lifetime |
|---|---|---|
| Domain entries/IDs/Roadmaps | Plain files or authenticated Vault catalog | Durable |
| Password | Prompt/client request, then `SecretString` | Unlock only |
| KEK | Vault opening routine | Key unwrap only |
| Master Key and unlocked `Nit` | Session Agent for normal Vault use | Until lock/removal/crash |
| Drive identity | Visible Drive header + authenticated Vault binding | Durable on media |
| Plain workspace location | `Workspace` | Command/TUI session |
| TUI focus/filter/scroll/draft | TUI | Interactive session |
| Viewer state | NIT Cat/TUI | Viewer session |
| Device inventory | `nit-drive` discovery result | Read-only snapshot; revalidated |

## Security boundaries

- Core validates domain data regardless of backend.
- Vault uses Argon2id and XChaCha20-Poly1305; no custom cryptography.
- Object names are random and metadata-minimal.
- Session IPC is local and authenticates the same OS user on Unix and Windows.
- Secrets are not logged or persisted and are zeroized where practical.
- Drive removal invalidates the session; a reused path does not restore it.
- CLI treats `Unavailable` as a hard error before Plain discovery.
- Provisioning rejects fixed, system/root/boot, read-only, ambiguous, absent,
  changed, duplicate, and undersized targets.
- Programs receive separate arguments; no user-built shell command is executed.
- Real formatting requires an exact identity-bearing confirmation.

## Atomicity and filesystem realities

Plain reuses its write-ahead transaction journal for multi-file readable
storage. Vault uses immutable ciphertext objects plus alternating authenticated
root publication. Both use locking, bounded reads, same-filesystem temporary
files, atomic rename where supported, and synchronization calls.

exFAT is the interoperability target for NIT Drive, but removable controllers
and exFAT cannot provide the same durability as a journaled local filesystem.
Safe eject remains required. See [NIT Drive](NIT_DRIVE.md) for details.

## Explicit non-goals

The architecture contains no synchronization, cloud, account, server, TCP/HTTP
Session protocol, FUSE/filesystem driver, block encryption, portable runtime,
automatic local copy, or permanent auto-unlock. The Drive is canonical.

## Distribution and platform support

All crates share one workspace version. CI builds/tests Linux, macOS, and
Windows; NIT Drive discovery, removal, and provisioning are implemented for
Linux and Windows. Other platforms retain Plain features but return an explicit
unsupported error for Drive lifecycle operations.

See [NIT Core](core.md), [Vault](vault.md), [Session](session.md), and
[NIT Drive](NIT_DRIVE.md).
