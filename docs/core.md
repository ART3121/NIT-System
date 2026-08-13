# NIT Core

`nit-core` is the domain and persistence foundation shared by NIT interfaces.
It is a Rust library, not an executable, network service, UI, or device manager.

## Public application boundary

The `Nit` facade implements `NitApi`, the interface consumed by CLI and TUI.
`SessionClient` also implements `NitApi`, so a frontend can use the same domain
operations against an in-process Plain workspace or an unlocked remote Vault
session.

```rust
let nit = nit_core::Nit::discover()?; // nearest Plain .nit/
let id = nit.create(nit_core::Kind::Note, None, "Architecture".into())?;
```

For Vault Storage, callers open an authenticated `Vault`, select a stable
`VaultWorkspaceId`, and construct `Nit::open_vault(...)`. Normal desktop and CLI
flows keep this value inside the Session Agent instead of copying its Master
Key into every process.

## Domain invariants

Core owns exactly one domain model for both persistence modes:

- `Entry`, `EntryId`, `Kind`, `Horizon`, `Roadmap`, `Notes`, and `View`;
- horizons only for Ideas and To-dos;
- classification-specific, collision-resistant human-readable sequences;
- search, create, archive, import, save, and Roadmap attachment;
- active/archive consistency and stale-snapshot rejection;
- validation before every durable mutation.

There are no `EncryptedEntry`, `PortableEntry`, or Drive-specific domain types.
Encryption and media placement remain persistence concerns.

## Storage model

```text
Repository
├── PlainRepository  ──> Workspace ──> .nit/
└── VaultRepository  ──> Vault ─────> authenticated ciphertext
```

The repository selects a small backend enum and shares the surrounding domain
operations. Create, search, archive, ID allocation, import, and Roadmap rules
are not duplicated by backend.

### Plain Storage

Plain is the existing official storage mode. `Workspace::discover` walks from
the current directory toward the filesystem root and selects the nearest valid
`.nit/`. It never initializes implicitly. Notes remain individual Markdown
documents and other entry classes remain compact text collections.

Plain mutations reuse the established guarantees:

- shared/exclusive workspace locking;
- bounded reads and symlink rejection for administrative paths;
- private, same-filesystem temporary files and atomic replacement;
- file and Unix parent-directory synchronization;
- a `.nit/.transaction/` write-ahead snapshot;
- recovery after interrupted multi-file changes;
- post-write validation and stale frontend detection.

Existing Plain workspaces are not encrypted or migrated automatically, never
require a password, and do not require the Session Agent.

### Vault Storage

Vault stores an authenticated, versioned catalog containing one or more
workspaces. Each workspace has a random 128-bit path-independent identity,
name, active/archive collections, and ID sequences. The same repository state
is serialized with `postcard` and committed as an encrypted Vault object.

Vault uses an exclusive file lock around read/modify/write transactions,
immutable objects, and alternating authenticated roots. Plaintext catalog
buffers and temporary key material are zeroized where practical. It never
extracts a `.nit/` tree to a temporary host directory.

See [Vault format](vault.md) for the exact cryptographic and commit contract.

## Internal modules

| Module | Purpose |
|---|---|
| `model.rs` | Domain values and classification rules |
| `workspace.rs` | Plain workspace discovery, initialization, and migration |
| `repository.rs` | Shared operations and the small Plain/Vault backend boundary |
| `storage.rs` | Plain text parsing and canonical serialization |
| `ids.rs` | Per-class sequences and collision prevention |
| `commands.rs` | Domain use cases |
| `fsutil.rs` | Plain locks, journal, bounded reads, and atomic replacement |
| `vault.rs` | Vault v1 cryptography, records, locking, and commits |
| `vault_repository.rs` | Encrypted catalog and stable Vault workspaces |
| `lib.rs` | `Nit`, `NitApi`, and exported domain surface |

## Frontend rules

- CLI and TUI call `NitApi`; they do not parse or write storage directly.
- Session owns unlocked Vault state but delegates domain work to `Nit`.
- Drive discovers and prepares media but does not define domain rules.
- AI returns a validated `Roadmap`; only Core can attach it.
- External editor operations are permitted for Plain Storage only. Vault
  editing through a plaintext host temporary file is deliberately rejected.
- NIT Cat currently discovers Plain workspaces when resolving Note IDs; ordinary
  Markdown file viewing remains storage-independent.

## Compatibility and stability

Plain Storage remains a supported first-class mode. Vault is additive and does
not change existing `.nit/` data. The Rust API is currently workspace-internal
and is not an HTTP API, plugin protocol, or crates.io stability commitment.

See [Architecture](architecture.md), [Session Agent](session.md), and
[NIT Drive](NIT_DRIVE.md) for the surrounding runtime.
