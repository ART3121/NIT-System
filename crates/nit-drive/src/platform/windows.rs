use std::{iter, os::windows::ffi::OsStrExt, path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use windows_sys::Win32::Storage::FileSystem::{
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
};

use crate::{PlannedOperation, RemovableDevice};

pub(crate) struct PresenceToken {
    canonical_path: PathBuf,
    volume_name: String,
}

impl PresenceToken {
    pub(crate) fn capture(path: &std::path::Path) -> Result<Self> {
        let canonical_path = path
            .canonicalize()
            .with_context(|| format!("Vault path is unavailable: {}", path.display()))?;
        Ok(Self {
            volume_name: volume_name(&canonical_path)?,
            canonical_path,
        })
    }

    pub(crate) fn is_present(&self) -> bool {
        self.canonical_path.canonicalize().ok().as_ref() == Some(&self.canonical_path)
            && volume_name(&self.canonical_path).ok().as_ref() == Some(&self.volume_name)
    }
}

fn volume_name(path: &std::path::Path) -> Result<String> {
    const BUFFER_CHARS: usize = 32_768;
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let mut mount = vec![0_u16; BUFFER_CHARS];
    // SAFETY: both UTF-16 buffers are NUL-terminated/writable for their stated lengths.
    if unsafe { GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return Err(std::io::Error::last_os_error()).context("could not identify Vault volume");
    }
    let mount_end = mount
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(mount.len());
    if mount_end == mount.len() {
        bail!("invalid Windows Vault volume path");
    }
    mount.truncate(mount_end + 1);

    let mut name = vec![0_u16; BUFFER_CHARS];
    // SAFETY: `mount` is NUL-terminated and `name` is writable for its stated length.
    if unsafe {
        GetVolumeNameForVolumeMountPointW(mount.as_ptr(), name.as_mut_ptr(), name.len() as u32)
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("could not read Vault volume identity");
    }
    let name_end = name
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(name.len());
    if name_end == 0 || name_end == name.len() {
        bail!("invalid Windows Vault volume identity");
    }
    String::from_utf16(&name[..name_end]).context("Vault volume identity is not valid UTF-16")
}

const DISCOVERY_SCRIPT: &str = r#"
$systemDrive = $env:SystemDrive
$result = @(Get-CimInstance Win32_DiskDrive | ForEach-Object {
  $disk = $_
  $storageDisk = Get-Disk -Number $disk.Index -ErrorAction SilentlyContinue
  $mounts = @()
  Get-CimAssociatedInstance -InputObject $disk -Association Win32_DiskDriveToDiskPartition | ForEach-Object {
    Get-CimAssociatedInstance -InputObject $_ -Association Win32_LogicalDiskToPartition | ForEach-Object {
      if ($_.DeviceID) { $mounts += ($_.DeviceID + '\') }
    }
  }
  [PSCustomObject]@{
    Id = [string]$disk.DeviceID
    Model = [string]$disk.Model
    CapacityBytes = [UInt64]$disk.Size
    MountPoints = @($mounts)
    Removable = ($disk.MediaType -match 'Removable' -or $disk.InterfaceType -eq 'USB')
    SystemDisk = ([bool]$storageDisk.IsSystem -or [bool]$storageDisk.IsBoot -or @($mounts | Where-Object { $_ -like ($systemDrive + '*') }).Count -gt 0)
    ReadOnly = [bool]$storageDisk.IsReadOnly
  }
})
$result | ConvertTo-Json -Compress -Depth 3
"#;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsDevice {
    id: String,
    model: String,
    capacity_bytes: u64,
    mount_points: Vec<String>,
    removable: bool,
    system_disk: bool,
    read_only: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WindowsDevices {
    Many(Vec<WindowsDevice>),
    One(WindowsDevice),
}

pub(crate) fn discover_devices() -> Result<Vec<RemovableDevice>> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            DISCOVERY_SCRIPT,
        ])
        .output()
        .context("could not execute Windows device discovery")?;
    if !output.status.success() {
        bail!(
            "Windows device discovery failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_devices(&output.stdout)
}

pub(crate) fn provisioning_operations(device: &RemovableDevice) -> Result<Vec<PlannedOperation>> {
    let disk_number = physical_drive_number(&device.id)?;
    Ok(vec![PlannedOperation {
        program: "powershell.exe".into(),
        arguments: vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "& { param([int]$DiskNumber, [UInt64]$ExpectedSize) $disk = Get-Disk -Number $DiskNumber -ErrorAction Stop; if ($disk.IsSystem -or $disk.IsBoot -or $disk.IsReadOnly -or $disk.Size -ne $ExpectedSize -or $disk.BusType -notin @('USB','SD','MMC')) { throw 'Device failed NIT Drive safety validation' }; Clear-Disk -Number $DiskNumber -RemoveData -RemoveOEM -Confirm:$false; Initialize-Disk -Number $DiskNumber -PartitionStyle GPT; New-Partition -DiskNumber $DiskNumber -UseMaximumSize -AssignDriveLetter | Format-Volume -FileSystem exFAT -NewFileSystemLabel NIT_DRIVE -Confirm:$false }".into(),
            disk_number.to_string(),
            device.capacity_bytes.to_string(),
        ],
        destructive: true,
    }])
}

fn physical_drive_number(id: &str) -> Result<u32> {
    let upper = id.to_ascii_uppercase();
    let value = upper
        .strip_prefix(r"\\.\PHYSICALDRIVE")
        .ok_or_else(|| anyhow::anyhow!("invalid Windows physical drive identifier"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid Windows physical drive identifier");
    }
    value
        .parse()
        .context("invalid Windows physical drive number")
}

fn parse_devices(source: &[u8]) -> Result<Vec<RemovableDevice>> {
    let parsed: WindowsDevices =
        serde_json::from_slice(source).context("invalid Windows device discovery response")?;
    let parsed = match parsed {
        WindowsDevices::Many(devices) => devices,
        WindowsDevices::One(device) => vec![device],
    };
    let mut devices = parsed
        .into_iter()
        .map(|device| RemovableDevice {
            id: device.id,
            model: device.model,
            capacity_bytes: device.capacity_bytes,
            mount_points: device.mount_points.into_iter().map(PathBuf::from).collect(),
            removable: device.removable,
            system_disk: device.system_disk,
            read_only: device.read_only,
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_cim_output() {
        let devices = parse_devices(
            br#"[{"Id":"\\\\.\\PHYSICALDRIVE0","Model":"Internal","CapacityBytes":1000,"MountPoints":["C:\\"],"Removable":false,"SystemDisk":true,"ReadOnly":false},{"Id":"\\\\.\\PHYSICALDRIVE2","Model":"USB","CapacityBytes":2000,"MountPoints":["E:\\"],"Removable":true,"SystemDisk":false,"ReadOnly":false}]"#,
        )
        .unwrap();
        assert_eq!(devices.len(), 2);
        assert!(devices[0].system_disk);
        assert!(devices[1].removable);
    }
}
