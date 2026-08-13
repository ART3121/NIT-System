use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

use crate::{PlannedOperation, RemovableDevice};

const SYS_BLOCK: &str = "/sys/class/block";
const MOUNTINFO: &str = "/proc/self/mountinfo";
const MAX_DEVICES: usize = 256;

pub(crate) struct PresenceToken {
    canonical_path: PathBuf,
    mount_id: String,
    device_number: String,
    mount_point: PathBuf,
}

impl PresenceToken {
    pub(crate) fn capture(path: &Path) -> Result<Self> {
        Self::capture_from(path, Path::new(MOUNTINFO))
    }

    fn capture_from(path: &Path, mountinfo: &Path) -> Result<Self> {
        let canonical_path = path
            .canonicalize()
            .with_context(|| format!("Vault path is unavailable: {}", path.display()))?;
        let mounts = parse_presence_mounts(&fs::read_to_string(mountinfo)?)?;
        let mount = mounts
            .into_iter()
            .filter(|mount| canonical_path.starts_with(&mount.mount_point))
            .max_by_key(|mount| mount.mount_point.as_os_str().len())
            .ok_or_else(|| anyhow::anyhow!("could not identify the Vault mount"))?;
        Ok(Self {
            canonical_path,
            mount_id: mount.mount_id,
            device_number: mount.device_number,
            mount_point: mount.mount_point,
        })
    }

    pub(crate) fn is_present(&self) -> bool {
        self.is_present_from(Path::new(MOUNTINFO))
    }

    fn is_present_from(&self, mountinfo: &Path) -> bool {
        if self.canonical_path.canonicalize().ok().as_ref() != Some(&self.canonical_path) {
            return false;
        }
        fs::read_to_string(mountinfo)
            .ok()
            .and_then(|source| parse_presence_mounts(&source).ok())
            .is_some_and(|mounts| {
                mounts.into_iter().any(|mount| {
                    mount.mount_id == self.mount_id
                        && mount.device_number == self.device_number
                        && mount.mount_point == self.mount_point
                })
            })
    }
}

pub(crate) fn discover_devices() -> Result<Vec<RemovableDevice>> {
    discover_from(Path::new(SYS_BLOCK), Path::new(MOUNTINFO))
}

pub(crate) fn provisioning_operations(device: &RemovableDevice) -> Result<Vec<PlannedOperation>> {
    validate_linux_device_id(&device.id)?;
    let partition = first_partition_path(&device.id)?;
    let mut operations = Vec::new();
    if !device.mount_points.is_empty() {
        operations.push(PlannedOperation {
            program: "umount".into(),
            arguments: device
                .mount_points
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            destructive: false,
        });
    }
    operations.extend([
        PlannedOperation {
            program: "wipefs".into(),
            arguments: vec!["--all".into(), device.id.clone()],
            destructive: true,
        },
        PlannedOperation {
            program: "parted".into(),
            arguments: vec![
                "--script".into(),
                device.id.clone(),
                "mklabel".into(),
                "gpt".into(),
                "mkpart".into(),
                "primary".into(),
                "1MiB".into(),
                "100%".into(),
            ],
            destructive: true,
        },
        PlannedOperation {
            program: "partprobe".into(),
            arguments: vec![device.id.clone()],
            destructive: false,
        },
        PlannedOperation {
            program: "udevadm".into(),
            arguments: vec!["settle".into(), "--timeout=10".into()],
            destructive: false,
        },
        PlannedOperation {
            program: "mkfs.exfat".into(),
            arguments: vec!["-n".into(), "NIT_DRIVE".into(), partition],
            destructive: true,
        },
    ]);
    Ok(operations)
}

fn first_partition_path(device_id: &str) -> Result<String> {
    let name = device_id
        .strip_prefix("/dev/")
        .ok_or_else(|| anyhow::anyhow!("invalid Linux block device identifier"))?;
    Ok(if name.starts_with("nvme") || name.starts_with("mmcblk") {
        format!("{device_id}p1")
    } else {
        format!("{device_id}1")
    })
}

fn validate_linux_device_id(id: &str) -> Result<()> {
    let Some(name) = id.strip_prefix("/dev/") else {
        bail!("invalid Linux block device identifier");
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || !looks_like_whole_disk(name)
    {
        bail!("refusing an ambiguous Linux block device identifier");
    }
    Ok(())
}

fn looks_like_whole_disk(name: &str) -> bool {
    ["sd", "vd", "xvd", "hd"].iter().any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_lowercase())
        })
    }) || name.strip_prefix("nvme").is_some_and(|suffix| {
        suffix
            .split_once('n')
            .is_some_and(|(controller, namespace)| {
                !controller.is_empty()
                    && !namespace.is_empty()
                    && controller.bytes().all(|byte| byte.is_ascii_digit())
                    && namespace.bytes().all(|byte| byte.is_ascii_digit())
            })
    }) || name.strip_prefix("mmcblk").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn discover_from(sys_block: &Path, mountinfo: &Path) -> Result<Vec<RemovableDevice>> {
    let mut records = Vec::new();
    for item in fs::read_dir(sys_block).with_context(|| {
        format!(
            "could not inspect Linux block devices at {}",
            sys_block.display()
        )
    })? {
        if records.len() >= MAX_DEVICES {
            bail!("too many Linux block devices; discovery is ambiguous");
        }
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Linux block device name is not valid UTF-8"))?;
        if is_ignored_virtual_device(&name) {
            continue;
        }
        let path = item.path();
        let dev = read_trimmed_optional(&path.join("dev"))?;
        let slaves = read_directory_names_optional(&path.join("slaves"))?;
        records.push(BlockRecord {
            name,
            path,
            dev,
            partition: item.path().join("partition").is_file(),
            slaves,
        });
    }

    let top_names = records
        .iter()
        .filter(|record| !record.partition)
        .map(|record| record.name.as_str())
        .collect::<Vec<_>>();
    let mut dev_to_name = HashMap::new();
    for record in &records {
        let Some(dev) = &record.dev else {
            continue;
        };
        dev_to_name.insert(dev.clone(), record.name.clone());
    }

    let mounts = parse_mountinfo(&fs::read_to_string(mountinfo).with_context(|| {
        format!(
            "could not read Linux mount information from {}",
            mountinfo.display()
        )
    })?)?;
    let mut mounts_by_top: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for mount in mounts {
        let Some(name) = dev_to_name.get(&mount.device_number) else {
            continue;
        };
        let mut top_devices = HashSet::new();
        resolve_top_devices(
            name,
            &records,
            &top_names,
            &mut HashSet::new(),
            &mut top_devices,
        )?;
        for top in top_devices {
            mounts_by_top
                .entry(top)
                .or_default()
                .push(mount.mount_point.clone());
        }
    }

    let mut devices = Vec::new();
    for record in records.into_iter().filter(|record| !record.partition) {
        let size_sectors = parse_u64_optional(read_trimmed_optional(&record.path.join("size"))?)?;
        let capacity_bytes = size_sectors
            .and_then(|sectors| sectors.checked_mul(512))
            .unwrap_or(0);
        let removable =
            read_trimmed_optional(&record.path.join("removable"))?.as_deref() == Some("1");
        let read_only = read_trimmed_optional(&record.path.join("ro"))?.as_deref() == Some("1");
        let model = read_trimmed_optional(&record.path.join("device/model"))?
            .unwrap_or_else(|| "Unknown model".into());
        let mut mount_points = mounts_by_top.remove(&record.name).unwrap_or_default();
        mount_points.sort();
        mount_points.dedup();
        let system_disk = mount_points.iter().any(|mount| {
            mount == Path::new("/")
                || mount == Path::new("/boot")
                || mount == Path::new("/boot/efi")
        });
        devices.push(RemovableDevice {
            id: format!("/dev/{}", record.name),
            model,
            capacity_bytes,
            mount_points,
            removable,
            system_disk,
            read_only,
        });
    }
    devices.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(devices)
}

struct BlockRecord {
    name: String,
    path: PathBuf,
    dev: Option<String>,
    partition: bool,
    slaves: Vec<String>,
}

fn resolve_top_devices(
    name: &str,
    records: &[BlockRecord],
    top_names: &[&str],
    visiting: &mut HashSet<String>,
    output: &mut HashSet<String>,
) -> Result<()> {
    if !visiting.insert(name.to_owned()) {
        bail!("cyclic Linux block device dependency; discovery is ambiguous");
    }
    let record = records
        .iter()
        .find(|record| record.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown Linux block device dependency"))?;
    if record.partition {
        let parent = top_names
            .iter()
            .filter(|top| partition_belongs_to(name, top))
            .max_by_key(|top| top.len())
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("orphan Linux block partition; discovery is ambiguous")
            })?;
        resolve_top_devices(parent, records, top_names, visiting, output)?;
    } else if record.slaves.is_empty() {
        output.insert(name.to_owned());
    } else {
        for slave in &record.slaves {
            resolve_top_devices(slave, records, top_names, visiting, output)?;
        }
    }
    visiting.remove(name);
    Ok(())
}

struct MountRecord {
    device_number: String,
    mount_point: PathBuf,
}

struct PresenceMount {
    mount_id: String,
    device_number: String,
    mount_point: PathBuf,
}

fn parse_mountinfo(source: &str) -> Result<Vec<MountRecord>> {
    let mut mounts = Vec::new();
    for line in source.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 || !fields.contains(&"-") {
            bail!("invalid Linux mountinfo record");
        }
        let device_number = fields[2];
        if !valid_device_number(device_number) {
            bail!("invalid Linux mountinfo device number");
        }
        mounts.push(MountRecord {
            device_number: device_number.to_owned(),
            mount_point: PathBuf::from(decode_mount_field(fields[4])?),
        });
    }
    Ok(mounts)
}

fn parse_presence_mounts(source: &str) -> Result<Vec<PresenceMount>> {
    let mut mounts = Vec::new();
    for line in source.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 || !fields.contains(&"-") || !valid_device_number(fields[2]) {
            bail!("invalid Linux mountinfo record");
        }
        if !fields[0].bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("invalid Linux mount ID");
        }
        mounts.push(PresenceMount {
            mount_id: fields[0].to_owned(),
            device_number: fields[2].to_owned(),
            mount_point: PathBuf::from(decode_mount_field(fields[4])?),
        });
    }
    Ok(mounts)
}

fn valid_device_number(value: &str) -> bool {
    value.split_once(':').is_some_and(|(major, minor)| {
        !major.is_empty()
            && !minor.is_empty()
            && major.bytes().all(|byte| byte.is_ascii_digit())
            && minor.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn decode_mount_field(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if index + 3 >= bytes.len()
                || !bytes[index + 1..=index + 3]
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'7'))
            {
                bail!("invalid escape in Linux mountinfo path");
            }
            let decoded = (bytes[index + 1] - b'0') * 64
                + (bytes[index + 2] - b'0') * 8
                + (bytes[index + 3] - b'0');
            output.push(decoded);
            index += 4;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).context("Linux mount point is not valid UTF-8")
}

fn partition_belongs_to(partition: &str, disk: &str) -> bool {
    let Some(remainder) = partition.strip_prefix(disk) else {
        return false;
    };
    let remainder = remainder.strip_prefix('p').unwrap_or(remainder);
    !remainder.is_empty() && remainder.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_ignored_virtual_device(name: &str) -> bool {
    ["loop", "ram", "zram", "fd", "sr"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn read_trimmed_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn read_directory_names_optional(path: &Path) -> Result<Vec<String>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", path.display()))
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        if names.len() >= MAX_DEVICES {
            bail!("too many Linux block device dependencies; discovery is ambiguous");
        }
        names.push(
            entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Linux block dependency is not valid UTF-8"))?,
        );
    }
    Ok(names)
}

fn parse_u64_optional(value: Option<String>) -> Result<Option<u64>> {
    value
        .map(|value| value.parse().context("invalid Linux block device size"))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_device(
        root: &Path,
        name: &str,
        dev: &str,
        partition: bool,
        removable: &str,
        sectors: &str,
        model: Option<&str>,
    ) {
        let path = root.join(name);
        fs::create_dir_all(path.join("device")).unwrap();
        fs::write(path.join("dev"), dev).unwrap();
        fs::write(path.join("removable"), removable).unwrap();
        fs::write(path.join("size"), sectors).unwrap();
        fs::write(path.join("ro"), "0").unwrap();
        if partition {
            fs::write(path.join("partition"), "1").unwrap();
        }
        if let Some(model) = model {
            fs::write(path.join("device/model"), model).unwrap();
        }
    }

    #[test]
    fn recognizes_removable_and_marks_the_root_disk() {
        let temp = tempfile::tempdir().unwrap();
        let sys = temp.path().join("sys");
        fs::create_dir(&sys).unwrap();
        write_device(&sys, "sda", "8:0", false, "0", "1000", Some("Internal"));
        write_device(&sys, "sda1", "8:1", true, "0", "900", None);
        write_device(&sys, "sdb", "8:16", false, "1", "2000", Some("USB Disk"));
        write_device(&sys, "sdb1", "8:17", true, "1", "1900", None);
        let mountinfo = temp.path().join("mountinfo");
        fs::write(
            &mountinfo,
            "36 25 8:1 / / rw - ext4 /dev/sda1 rw\n37 25 8:17 / /media/NIT\\040Drive rw - exfat /dev/sdb1 rw\n",
        )
        .unwrap();

        let devices = discover_from(&sys, &mountinfo).unwrap();
        assert_eq!(devices.len(), 2);
        assert!(devices[0].system_disk);
        assert!(!devices[0].removable);
        assert_eq!(devices[1].model, "USB Disk");
        assert!(devices[1].removable);
        assert!(!devices[1].system_disk);
        assert_eq!(devices[1].capacity_bytes, 2000 * 512);
        assert_eq!(devices[1].mount_points, [PathBuf::from("/media/NIT Drive")]);
    }

    #[test]
    fn supports_nvme_partition_names_and_marks_missing_metadata_ambiguous() {
        let temp = tempfile::tempdir().unwrap();
        let sys = temp.path().join("sys");
        fs::create_dir(&sys).unwrap();
        write_device(&sys, "nvme0n1", "259:0", false, "0", "0", None);
        write_device(&sys, "nvme0n1p1", "259:1", true, "0", "10", None);
        let mountinfo = temp.path().join("mountinfo");
        fs::write(
            &mountinfo,
            "36 25 259:1 / /boot/efi rw - vfat /dev/nvme0n1p1 rw\n",
        )
        .unwrap();

        let devices = discover_from(&sys, &mountinfo).unwrap();
        assert!(devices[0].system_disk);
        assert!(devices[0].is_ambiguous());
    }

    #[test]
    fn marks_physical_disk_beneath_device_mapper_root_as_system() {
        let temp = tempfile::tempdir().unwrap();
        let sys = temp.path().join("sys");
        fs::create_dir(&sys).unwrap();
        write_device(&sys, "sdb", "8:16", false, "1", "2000", Some("USB Root"));
        write_device(&sys, "sdb1", "8:17", true, "1", "1900", None);
        write_device(&sys, "dm-0", "253:0", false, "0", "1800", Some("LVM"));
        fs::create_dir_all(sys.join("dm-0/slaves/sdb1")).unwrap();
        let mountinfo = temp.path().join("mountinfo");
        fs::write(
            &mountinfo,
            "36 25 253:0 / / rw - ext4 /dev/mapper/root rw\n",
        )
        .unwrap();

        let devices = discover_from(&sys, &mountinfo).unwrap();
        let physical = devices
            .iter()
            .find(|device| device.id == "/dev/sdb")
            .unwrap();
        assert!(physical.system_disk);
        assert_eq!(physical.mount_points, [PathBuf::from("/")]);
    }

    #[test]
    fn rejects_malformed_mount_information() {
        assert!(parse_mountinfo("not mountinfo").is_err());
        assert!(decode_mount_field("/bad\\xx").is_err());
    }

    #[test]
    fn presence_token_never_survives_a_mount_generation_change() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("media/NIT/vault");
        fs::create_dir_all(&vault).unwrap();
        let mountinfo = temp.path().join("mountinfo");
        fs::write(
            &mountinfo,
            format!(
                "41 25 8:17 / {} rw - exfat /dev/sdb1 rw\n",
                temp.path().join("media/NIT").display()
            ),
        )
        .unwrap();
        let token = PresenceToken::capture_from(&vault, &mountinfo).unwrap();
        assert!(token.is_present_from(&mountinfo));

        fs::write(
            &mountinfo,
            format!(
                "42 25 8:17 / {} rw - exfat /dev/sdb1 rw\n",
                temp.path().join("media/NIT").display()
            ),
        )
        .unwrap();
        assert!(!token.is_present_from(&mountinfo));
    }
}
