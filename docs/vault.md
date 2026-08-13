# NIT Vault v1

NIT Vault is the encrypted persistence backend implemented inside `nit-core`.
It answers how data is stored. It is independent of USB: a Vault can live in an
ordinary directory, while every NIT Drive is required to contain a Vault.

## Cryptographic design

```text
password
   │
   └── Argon2id + random salt ──> Key Encryption Key
                                      │
                                      └── decrypt wrapped random Master Key
                                                              │
                                                              └── XChaCha20-Poly1305
                                                                  Vault records
```

The password never encrypts domain objects directly. Creation generates a
random 256-bit Master Key with the operating-system CSPRNG. Argon2id derives a
temporary Key Encryption Key (KEK), which wraps only the Master Key. This format
allows a future password-change operation to derive a new KEK and rewrap the
Master Key without re-encrypting every object.

Current production KDF parameters are recorded in the authenticated header and
validated against hard upper bounds before derivation. Passwords, KEKs, and the
unwrapped Master Key are never persisted in plaintext. Sensitive buffers use
`secrecy` and `zeroize` where practical and are never included in `Debug` output.

Vault v1 currently creates headers with Argon2 version 0x13, 64 MiB memory,
three iterations, and parallelism four. Opening accepts only Argon2id v0x13 and
rejects memory outside 8 KiB–1 GiB, time cost outside 1–32, or parallelism
outside 1–64 before allocating the KDF workload. These limits are format
validation, not a promise that future versions will keep the same creation
parameters.

## Directory format

```text
vault/
├── header
├── lock
├── root.0
├── root.1
└── objects/
    ├── <32-byte-random-id-as-hex>
    └── <32-byte-random-id-as-hex>
```

Every record begins with a fixed magic value and an explicit format version.
The binary payload uses `postcard`; record size and field values are bounded
before use.

### Header

The header contains only what is required to unlock:

- Vault format version and random 128-bit `vault_id`;
- Argon2id salt and KDF parameters;
- Master-Key wrapping algorithm;
- wrapping nonce;
- authenticated encrypted Master Key.

It does not contain a password, plaintext key, derived key, entry metadata, or
workspace name. Header AAD binds the wrapping operation to the version,
Vault identity, KDF configuration, salt, algorithm, and nonce. An incorrect
password and authenticated header damage both fail closed.

### Objects

Objects are immutable and use random opaque 256-bit identifiers. Filenames do
not reveal Entry IDs, kinds, titles, or workspace names. Each object has a fresh
192-bit XChaCha20 nonce and authenticated ciphertext. AAD binds:

- Vault format version;
- `vault_id`;
- object identifier;
- object algorithm and nonce.

Changing, truncating, renaming, replacing, or moving an object into a different
Vault causes authentication or contextual validation to fail.

### Root records

`root.0` and `root.1` alternate by monotonically increasing generation. They
are independently encrypted and authenticated and point to the current object.
An object is fully persisted before the next root is published. If the newest
root is partially written, the previous authenticated generation remains
readable. If neither root authenticates, opening the committed state fails.

## Catalog and workspaces

The latest object contains the authenticated catalog v1:

- optional external binding, used by NIT Drive identity verification;
- up to 1,024 workspaces;
- random stable `VaultWorkspaceId` and validated name for each workspace;
- active/archive entries and independent ID sequences.

Workspace identity never uses a mount point or host path. Several independent
workspaces can share one Vault while retaining independent IDs and state.

## Commit and concurrency contract

A transaction obtains the Vault file lock for the complete read/modify/write
cycle, decrypts only the latest required catalog, applies a Core operation,
serializes the next state, writes a new immutable encrypted object, and finally
publishes an alternating root with atomic same-directory replacement.

Files are synchronized before publication. Parent-directory synchronization is
requested on Unix. A failed operation is not reported as committed. Immutable
orphan objects left by an interrupted commit do not become current state.

Vault does not materialize Plain `.nit/` data on the host. Decrypted payloads
exist only in process memory during an operation. External editor integration
is disabled because the current editor adapter relies on a plaintext temporary
file.

## Detected failures

Vault rejects or detects:

- incorrect password;
- unsupported format/KDF/algorithm versions;
- hostile or resource-exhausting KDF parameters;
- malformed, oversized, empty, or truncated records;
- altered headers, roots, object IDs, nonces, AAD, or ciphertext;
- missing referenced objects;
- symbolic links in administrative Vault paths;
- invalid authenticated catalog/domain metadata;
- stale frontend saves and concurrent repository changes.

Authenticated ciphertext is never converted into a default Entry on failure.

## Limitations

- Alternating roots provide crash recovery, not protection against a complete
  malicious rollback of the entire media to an older valid snapshot. Preventing
  offline rollback requires an external trusted monotonic state, which NIT does
  not maintain.
- File locking coordinates processes on one host; a Vault must not be mounted
  and modified simultaneously by multiple machines.
- Secure memory erasure is best-effort in a general-purpose Rust process and OS;
  it does not guarantee elimination of every compiler, kernel, swap, or crash
  artifact.
- Vault protects data at rest. An unlocked same-user frontend can request
  plaintext domain data through the authenticated local Session IPC.

## Minimal Rust API

The low-level API is useful to storage integrations and tests. Normal CLI/TUI
usage should prefer the Session Agent so the key is shared only in its process.

```rust,no_run
use nit_core::{vault::Vault, Kind, Nit};
use secrecy::SecretString;
use std::{path::Path, sync::Arc};

let password = SecretString::from("prompted password".to_owned());
let vault = Arc::new(Vault::create(Path::new("vault"), &password)?);
let workspace = Nit::create_vault_workspace(&vault, "Portable")?;
let nit = Nit::open_vault(vault, workspace.id)?;
nit.create(Kind::Note, None, "Encrypted note".into())?;
# Ok::<(), anyhow::Error>(())
```

`Vault::create` requires a missing or empty directory and refuses overwrite.
Production applications must obtain the password securely and must not embed it
as shown by the type-only example.

See [NIT Core](core.md), [Session Agent](session.md), and
[NIT Drive](NIT_DRIVE.md).
