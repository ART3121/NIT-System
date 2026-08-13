//! Read-only device discovery and conservative NIT Drive lifecycle.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

mod platform;

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
