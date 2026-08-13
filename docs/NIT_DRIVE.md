# NIT Drive storage notes

NIT Drive v1 uses a normal mounted filesystem. It does not define a filesystem,
block format, kernel driver, synchronization protocol, or host-side plaintext
cache.

Its on-volume layout is:

```text
.nit-drive/
├── header
└── vault/
    ├── header
    ├── root.0
    ├── root.1
    ├── lock
    └── objects/
```

The Drive header contains only the format version, random Drive identity, fixed
Vault location, and random Vault identity. The Drive identity is also stored in
the authenticated encrypted Vault catalog, so replacing or editing the visible
header is detected during unlock. Workspace names, entries, IDs, Roadmaps and
ID sequences remain inside authenticated ciphertext.

## Filesystem expectations

exFAT is the interoperability target for media used on Windows and Linux, but
it provides weaker crash guarantees than native journaled filesystems:

- Vault objects are immutable and written through a same-directory temporary
  file before rename.
- The current state uses two alternating, authenticated root files. If writing
  the newest root is interrupted, the previous authenticated root remains.
- Files are flushed before publication. Directory flush is requested on Unix;
  Windows does not expose the same portable directory `fsync` behavior.
- A completed syscall cannot guarantee that a removable device's controller
  has physically committed every cache line. The user must still use the OS
  safe-eject operation when possible.
- Advisory file locks coordinate processes on one host. They are not a
  distributed lock and are not intended for simultaneous access by machines.
- Removing media during provisioning or the first Vault initialization can
  leave an unformatted or partially formatted device. It cannot create a local
  `.nit/` fallback.

The Session Agent binds an unlock to the current mount generation on Linux.
Removal revokes and drops the in-memory `Nit`/Master Key; reinserting the same
device requires the password again. Windows uses conservative path/volume
presence polling and receives a platform-specific implementation behind the
same API.

Provisioning always performs fresh discovery, rejects internal/system/boot,
read-only and ambiguous disks, requires a confirmation containing identifier,
model and byte capacity, invokes programs with separated arguments, and checks
the device identity again. CI uses a fake command executor and never formats a
real disk.
