//! Read-only device discovery and conservative NIT Drive lifecycle.

use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

mod platform;

/// Captures the current physical/mount presence of a Vault path.
///
/// A detector is tied to one connection. Once it reports false it must be
/// discarded; reinsertion requires a new detector after password unlock.
pub struct RemovalDetector {
    token: platform::PresenceToken,
}

impl RemovalDetector {
    pub fn capture(path: &std::path::Path) -> Result<Self> {
        Ok(Self {
            token: platform::PresenceToken::capture(path)?,
        })
    }

    pub fn is_present(&self) -> bool {
        self.token.is_present()
    }
}

/// Physical disk information used for explicit provisioning decisions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovableDevice {
    /// OS-stable identifier for the current boot, such as `/dev/sdb` or
    /// `\\.\PHYSICALDRIVE2`. It is never accepted without rediscovery.
    pub id: String,
    pub model: String,
    pub capacity_bytes: u64,
    pub mount_points: Vec<PathBuf>,
    pub removable: bool,
    pub system_disk: bool,
    pub read_only: bool,
}

impl RemovableDevice {
    /// Whether the discovery data is sufficient for provisioning validation.
    pub fn is_ambiguous(&self) -> bool {
        self.id.trim().is_empty() || self.model.trim().is_empty() || self.capacity_bytes == 0
    }
}

/// Enumerates physical disks without mutating, mounting or formatting them.
pub fn discover_devices() -> Result<Vec<RemovableDevice>> {
    platform::discover_devices()
}

/// Read-only source used to force fresh discovery immediately before planning
/// and, later, immediately before execution.
pub trait DeviceSource {
    fn discover(&self) -> Result<Vec<RemovableDevice>>;
}

pub struct SystemDeviceSource;

impl DeviceSource for SystemDeviceSource {
    fn discover(&self) -> Result<Vec<RemovableDevice>> {
        discover_devices()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisioningPlan {
    pub device: RemovableDevice,
    pub operations: Vec<PlannedOperation>,
    pub confirmation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedOperation {
    pub program: String,
    pub arguments: Vec<String>,
    pub destructive: bool,
}

/// Conservative provisioning planner. `dry_run` never invokes a program.
pub struct Provisioner<S = SystemDeviceSource> {
    source: S,
}

impl Default for Provisioner<SystemDeviceSource> {
    fn default() -> Self {
        Self {
            source: SystemDeviceSource,
        }
    }
}

impl<S: DeviceSource> Provisioner<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }

    pub fn dry_run(&self, device_id: &str) -> Result<ProvisioningPlan> {
        if device_id.trim() != device_id || device_id.is_empty() {
            bail!("invalid device identifier");
        }
        let devices = self.source.discover()?;
        let matches = devices
            .into_iter()
            .filter(|device| device.id == device_id)
            .collect::<Vec<_>>();
        let device = match matches.as_slice() {
            [] => bail!("device is absent from fresh discovery; aborting"),
            [device] => device.clone(),
            _ => bail!("device identifier is ambiguous in fresh discovery; aborting"),
        };
        validate_provisioning_target(&device)?;
        Ok(ProvisioningPlan {
            operations: platform::provisioning_operations(&device)?,
            confirmation: format!(
                "ERASE {} {} {}",
                device.id, device.model, device.capacity_bytes
            ),
            device,
        })
    }
}

fn validate_provisioning_target(device: &RemovableDevice) -> Result<()> {
    if device.is_ambiguous() {
        bail!("device metadata is ambiguous; aborting");
    }
    if !device.removable {
        bail!("refusing to provision a fixed/internal disk");
    }
    if device.system_disk {
        bail!("refusing to provision a system, root or boot disk");
    }
    if device.read_only {
        bail!("refusing to provision a read-only disk");
    }
    if device.capacity_bytes < 64 * 1024 * 1024 {
        bail!("device is too small to be a NIT Drive");
    }
    Ok(())
}

#[cfg(test)]
mod provisioning_tests {
    use super::*;

    #[derive(Clone)]
    struct FakeSource(Vec<RemovableDevice>);

    impl DeviceSource for FakeSource {
        fn discover(&self) -> Result<Vec<RemovableDevice>> {
            Ok(self.0.clone())
        }
    }

    fn device() -> RemovableDevice {
        RemovableDevice {
            id: "/dev/sdb".into(),
            model: "Test USB".into(),
            capacity_bytes: 16 * 1024 * 1024 * 1024,
            mount_points: vec![PathBuf::from("/media/NIT")],
            removable: true,
            system_disk: false,
            read_only: false,
        }
    }

    #[test]
    fn dry_run_contains_explicit_identity_and_no_shell_command() {
        let plan = Provisioner::new(FakeSource(vec![device()]))
            .dry_run("/dev/sdb")
            .unwrap();
        assert!(plan.confirmation.contains("/dev/sdb"));
        assert!(plan.confirmation.contains("Test USB"));
        assert!(plan
            .operations
            .iter()
            .all(|operation| operation.program != "sh"));
        assert!(plan
            .operations
            .iter()
            .any(|operation| operation.destructive));
    }

    #[test]
    fn rejects_internal_system_read_only_ambiguous_absent_and_duplicate_devices() {
        let mut candidate = device();
        candidate.removable = false;
        assert!(Provisioner::new(FakeSource(vec![candidate]))
            .dry_run("/dev/sdb")
            .is_err());

        let mut candidate = device();
        candidate.system_disk = true;
        assert!(Provisioner::new(FakeSource(vec![candidate]))
            .dry_run("/dev/sdb")
            .is_err());

        let mut candidate = device();
        candidate.read_only = true;
        assert!(Provisioner::new(FakeSource(vec![candidate]))
            .dry_run("/dev/sdb")
            .is_err());

        let mut candidate = device();
        candidate.model.clear();
        assert!(Provisioner::new(FakeSource(vec![candidate]))
            .dry_run("/dev/sdb")
            .is_err());

        assert!(Provisioner::new(FakeSource(vec![]))
            .dry_run("/dev/sdb")
            .is_err());
        assert!(Provisioner::new(FakeSource(vec![device(), device()]))
            .dry_run("/dev/sdb")
            .is_err());
    }
}
