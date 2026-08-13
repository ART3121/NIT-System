//! Read-only device discovery and conservative NIT Drive lifecycle.

use std::{path::PathBuf, process::Command};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

mod nit_drive;
mod platform;

pub use nit_drive::{InitializedDrive, NitDrive, NitDriveInitializer};

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

pub trait CommandExecutor {
    fn execute(&self, operation: &PlannedOperation) -> Result<()>;
}

pub struct SystemCommandExecutor;

impl CommandExecutor for SystemCommandExecutor {
    fn execute(&self, operation: &PlannedOperation) -> Result<()> {
        let output = Command::new(&operation.program)
            .args(&operation.arguments)
            .output()
            .map_err(|error| {
                anyhow::anyhow!(
                    "could not execute provisioning program {}: {error}",
                    operation.program
                )
            })?;
        if !output.status.success() {
            bail!(
                "provisioning program {} failed with {}: {}",
                operation.program,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

/// Conservative provisioning planner. `dry_run` never invokes a program.
pub struct Provisioner<S = SystemDeviceSource, E = SystemCommandExecutor> {
    source: S,
    executor: E,
}

impl Default for Provisioner<SystemDeviceSource, SystemCommandExecutor> {
    fn default() -> Self {
        Self {
            source: SystemDeviceSource,
            executor: SystemCommandExecutor,
        }
    }
}

impl<S: DeviceSource> Provisioner<S, SystemCommandExecutor> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            executor: SystemCommandExecutor,
        }
    }
}

impl<S: DeviceSource, E: CommandExecutor> Provisioner<S, E> {
    pub fn with_executor(source: S, executor: E) -> Self {
        Self { source, executor }
    }

    pub fn dry_run(&self, device_id: &str) -> Result<ProvisioningPlan> {
        self.plan_from_fresh_discovery(device_id)
    }

    /// Executes an approved plan after repeating discovery and every P0 safety
    /// check. The caller must type the full confirmation from `dry_run`.
    pub fn execute(&self, device_id: &str, confirmation: &str) -> Result<RemovableDevice> {
        let preview = self.plan_from_fresh_discovery(device_id)?;
        if confirmation != preview.confirmation {
            bail!("destructive confirmation does not match the selected device");
        }
        let final_plan = self.plan_from_fresh_discovery(device_id)?;
        if device_fingerprint(&preview.device) != device_fingerprint(&final_plan.device) {
            bail!("device identity changed after confirmation; aborting");
        }
        for operation in &final_plan.operations {
            self.executor.execute(operation)?;
        }
        let verified = self.find_unique_device(device_id)?;
        if device_fingerprint(&final_plan.device) != device_fingerprint(&verified) {
            bail!("provisioning commands completed but device identity changed");
        }
        validate_provisioning_target(&verified)?;
        Ok(verified)
    }

    fn plan_from_fresh_discovery(&self, device_id: &str) -> Result<ProvisioningPlan> {
        if device_id.trim() != device_id || device_id.is_empty() {
            bail!("invalid device identifier");
        }
        let device = self.find_unique_device(device_id)?;
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

    fn find_unique_device(&self, device_id: &str) -> Result<RemovableDevice> {
        let devices = self.source.discover()?;
        let matches = devices
            .into_iter()
            .filter(|device| device.id == device_id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => bail!("device is absent from fresh discovery; aborting"),
            [device] => Ok(device.clone()),
            _ => bail!("device identifier is ambiguous in fresh discovery; aborting"),
        }
    }
}

fn device_fingerprint(device: &RemovableDevice) -> (&str, &str, u64) {
    (&device.id, &device.model, device.capacity_bytes)
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
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct FakeSource(Vec<RemovableDevice>);

    impl DeviceSource for FakeSource {
        fn discover(&self) -> Result<Vec<RemovableDevice>> {
            Ok(self.0.clone())
        }
    }

    #[derive(Clone, Default)]
    struct FakeExecutor {
        calls: Arc<Mutex<Vec<PlannedOperation>>>,
        fail_at: Option<usize>,
    }

    impl CommandExecutor for FakeExecutor {
        fn execute(&self, operation: &PlannedOperation) -> Result<()> {
            let mut calls = self.calls.lock().unwrap();
            if self.fail_at == Some(calls.len()) {
                bail!("simulated command failure");
            }
            calls.push(operation.clone());
            Ok(())
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

    #[test]
    fn execution_requires_exact_confirmation_and_revalidates_before_commands() {
        let source = FakeSource(vec![device()]);
        let executor = FakeExecutor::default();
        let calls = Arc::clone(&executor.calls);
        let provisioner = Provisioner::with_executor(source, executor);
        let plan = provisioner.dry_run("/dev/sdb").unwrap();
        assert!(provisioner.execute("/dev/sdb", "ERASE").is_err());
        assert!(calls.lock().unwrap().is_empty());

        provisioner.execute("/dev/sdb", &plan.confirmation).unwrap();
        assert_eq!(calls.lock().unwrap().len(), plan.operations.len());
    }

    #[test]
    fn command_failure_aborts_remaining_operations() {
        let executor = FakeExecutor {
            fail_at: Some(1),
            ..FakeExecutor::default()
        };
        let calls = Arc::clone(&executor.calls);
        let provisioner = Provisioner::with_executor(FakeSource(vec![device()]), executor);
        let plan = provisioner.dry_run("/dev/sdb").unwrap();
        assert!(provisioner.execute("/dev/sdb", &plan.confirmation).is_err());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }
}
