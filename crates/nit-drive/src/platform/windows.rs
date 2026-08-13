use std::{path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::RemovableDevice;

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

fn parse_devices(source: &[u8]) -> Result<Vec<RemovableDevice>> {
    let parsed: Vec<WindowsDevice> =
        serde_json::from_slice(source).context("invalid Windows device discovery response")?;
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
