# NIT Drive v1

NIT Drive is a removable-media lifecycle around a mandatory NIT Vault. It
answers where encrypted data lives and how the device is discovered, prepared,
validated, initialized, and monitored. Cryptography remains in `nit-core`.

NIT Drive uses a normal filesystem. It is not a custom filesystem, block
encryption layer, kernel driver, synchronization protocol, or portable runtime.

## On-volume layout

```text
<mounted-device>/
└── .nit-drive/
    ├── header
    └── vault/
        ├── header
        ├── lock
        ├── root.0
        ├── root.1
        └── objects/
            └── <opaque-id>
```

The JSON Drive header contains only:

- explicit Drive format version (`1`);
- random 128-bit Drive identity;
- fixed internal Vault directory name;
- random Vault identity.

The Drive identity is also stored as an authenticated binding inside the
encrypted Vault catalog. Unlock validates the visible Vault ID and authenticated
Drive binding, so changing or replacing the visible header fails closed.
Workspace names, entries, IDs, Roadmaps, and sequences remain ciphertext.

## Opening and unlocking

```bash
nit -unlock
```

The CLI discovers mounted removable devices and recognizes valid NIT Drive
roots. One Drive and one workspace are selected automatically. If several
Drives or workspaces exist, the CLI presents their models or names as a numbered
choice. Users do not need to type a mount path or remember an internal ID.

The explicit form remains available for scripts and diagnostics:

```bash
nit -unlock <mounted-drive-root> [workspace-id]
```

The explicit path must identify the mounted root containing `.nit-drive`, not
`.nit-drive/vault`. `NitDrive::open` validates the version, IDs, fixed relative
layout, file sizes, and symlink constraints before a password is used.

After unlock, CLI and TUI use the shared Session Agent. Removal changes the
session to `Unavailable`; reinserting the device requires the password again.
The Drive remains the canonical source. NIT never creates a local `.nit/`
fallback or sync copy.

## Multiple workspaces

One Vault can contain several independent workspaces. Each has a random stable
`VaultWorkspaceId`; drive letters and mount paths are never identities. Normal
CLI selection is by human-readable workspace name and number. The ID remains an
internal identity and an optional advanced CLI argument. `Nit::vault_workspaces`
and the initialized Drive result expose it to Rust integrations.

## Device discovery

Discovery is read-only and separate from formatting. It returns:

```text
RemovableDevice {
    id,
    model,
    capacity_bytes,
    mount_points,
    removable,
    system_disk,
    read_only,
}
```

Every destructive phase performs fresh discovery. A previously displayed
record is never trusted indefinitely.

### Linux

Linux reads `/sys/class/block` and `/proc/self/mountinfo`. It:

- excludes known virtual block classes;
- maps partitions to physical disks;
- follows `slaves/` dependencies through device-mapper/LVM/RAID-style layers;
- propagates `/`, `/boot`, and `/boot/efi` mounts to underlying disks;
- reads removable/read-only flags, model, capacity, device number, and mounts;
- treats malformed, cyclic, orphaned, or excessive topology as ambiguous.

### Windows

Windows runs a fixed non-interactive PowerShell/CIM discovery script. It
collects physical-drive identity, model, capacity, mount points, removable bus
type, read-only state, and `IsSystem`/`IsBoot`. User values are not interpolated
into the script.

## Provisioning safety

`Provisioner::dry_run(device_id)` performs no mutation and returns an ordered
plan plus an exact confirmation string:

```text
ERASE <device-id> <model> <capacity-bytes>
```

Execution refuses:

- fixed/internal media;
- system, root, boot, or EFI disks;
- read-only media;
- missing model, ID, or capacity;
- devices smaller than 64 MiB;
- invalid or ambiguous identifiers;
- absent or duplicate discovery records;
- any device whose ID/model/capacity changes between confirmation and execution.

The Rust layer enforces these rules even if a future GUI omits a check.
No device is selected automatically and “the first USB” is never accepted.

## Formatting

System commands are invoked directly with separated arguments. No command line
is constructed through a shell and no arbitrary path becomes a block-device ID.
Every exit status is checked; failure aborts remaining operations.

### Linux plan

1. unmount discovered mount points;
2. `wipefs --all <device>`;
3. create a GPT and one full-size partition with `parted --script`;
4. refresh/settle device state with `partprobe` and `udevadm`;
5. create an exFAT filesystem labeled `NIT_DRIVE` with `mkfs.exfat`;
6. settle device state again and mount the new partition with `udisksctl`.

Accepted whole-disk identifiers are conservatively limited to known `/dev/sd*`,
`vd*`, `xvd*`, `hd*`, `nvme*n*`, and `mmcblk*` forms. Formatting normally
requires administrator privileges and the corresponding system tools.

### Windows plan

A fixed PowerShell block accepts only a validated numeric physical-drive index
and expected byte size. It reopens the disk and rejects system, boot, read-only,
size-mismatched, or non-USB/SD/MMC media before `Clear-Disk`, GPT initialization,
partition creation, drive-letter assignment, and exFAT formatting.

## Drive initialization

After formatting and mounting, `NitDriveInitializer::initialize(...)` again:

1. discovers the exact device;
2. repeats all provisioning-target validation;
3. confirms that the supplied directory is a discovered mount point;
4. refuses existing `.nit-drive` metadata or symlinks;
5. stages a new Vault on the removable filesystem;
6. creates the first random-ID workspace;
7. creates and authenticates the Drive binding;
8. writes and synchronizes the Drive header;
9. atomically renames the staging directory into `.nit-drive`.

Initialization never creates `.nit/`. Interruption may leave a hidden staging
directory or partially prepared media, but cannot install a valid-looking Drive
before all authenticated metadata is ready.

The discovery, dry-run, execution, and initialization APIs exist in the
`nit-drive` Rust crate and are exposed by one explicit CLI workflow:

```bash
# Interactive discovery, selection, confirmation, formatting, and initialization
nit -drive-create

# Same destructive flow with an already known exact device ID
nit -drive-create /dev/sdb

# Read-only validation and command preview
nit -drive-create --dry-run /dev/sdb
```

The interactive path never selects a device automatically. It displays model,
capacity, mount points, and safety state; asks for the initial workspace and
password twice; then requires the exact confirmation string from the fresh
dry-run. Immediately before execution, `Provisioner` repeats discovery,
fingerprint comparison, and every P0 target validation.

If the filesystem was created but could not be mounted automatically, mount the
volume using the operating system and initialize it without formatting again:

```bash
nit -drive-create --initialize /dev/sdb /media/user/NIT_DRIVE
```

`--initialize` repeats removable-device validation and verifies that the path is
a discovered mount point of that exact device. It refuses existing NIT Drive
metadata. On Windows, use the physical identifier reported by the wizard (for
example `\\.\PHYSICALDRIVE2`) and the assigned drive root (for example `E:\`).

Formatting requires administrator privileges and the platform tools documented
below. No test or CI path executes real formatting commands.

### Rust API sequence

Discovery and preview are separate from mutation:

```rust,no_run
use nit_drive::{discover_devices, Provisioner};

for device in discover_devices()? {
    println!("{} {} {}", device.id, device.model, device.capacity_bytes);
}

let provisioner = Provisioner::default();
let plan = provisioner.dry_run("/dev/sdb")?;
println!("type exactly: {}", plan.confirmation);

// Only after an independent, explicit user confirmation:
let verified = provisioner.execute(&plan.device.id, &plan.confirmation)?;
# Ok::<(), anyhow::Error>(())
```

After the OS exposes the formatted filesystem as a mount point, initialize the
Drive and capture the first workspace ID:

```rust,no_run
use nit_drive::NitDriveInitializer;
use secrecy::SecretString;
use std::path::Path;

let password = SecretString::from("prompted password".to_owned());
let initialized = NitDriveInitializer::default().initialize(
    "/dev/sdb",
    Path::new("/media/user/NIT_DRIVE"),
    &password,
    "Portable workspace",
)?;
println!("workspace: {}", initialized.workspace.id);
# Ok::<(), anyhow::Error>(())
```

Applications must not hard-code the demonstration device IDs or passwords.
They must display fresh discovery data, obtain secrets through a non-echoing
input, and preserve the exact confirmation boundary.

## Removal detection

- Linux captures canonical path, mount ID, device number, and mount point.
- Windows captures canonical path and stable volume identity.
- The token is valid for one physical connection only.
- A monitor checks every 100 ms and destroys the unlocked session on absence.
- Reusing the same mount path or Windows drive letter does not revive a session.

If media disappears during a read/write, the operation fails through the
filesystem/Vault boundary. There is no fallback or merge. A frontend may retain
an unsaved draft only in memory until reconnect and a new unlock.

## exFAT and durability

exFAT is the interoperability target for Windows/Linux, not a durability
guarantee equivalent to ext4, XFS, APFS, or NTFS:

- Vault objects are immutable and published through same-directory replacement;
- alternating authenticated roots retain a previous valid generation after a
  partial newest-root write;
- files are synchronized before publication;
- Unix requests parent-directory synchronization;
- Windows lacks the same portable directory `fsync` contract;
- controller caches and abrupt physical removal can still lose acknowledged
  writes;
- advisory locks coordinate one host only.

Use the operating system's safe-eject action whenever possible. Never modify
one Drive simultaneously from multiple machines.

## Testing contract

CI never formats real media. Provisioning is tested with fake device sources
and command executors, including exact confirmation, repeated validation,
failure aborts, internal/system/read-only/ambiguous rejection, and platform
parsing. Drive initialization tests use temporary directories and fake discovery.

See [Vault](vault.md), [Session](session.md), and
[Architecture](architecture.md).
