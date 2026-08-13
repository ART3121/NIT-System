#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::discover_devices;
#[cfg(target_os = "linux")]
pub(crate) use linux::PresenceToken;
#[cfg(target_os = "windows")]
pub(crate) use windows::discover_devices;
#[cfg(target_os = "windows")]
pub(crate) use windows::PresenceToken;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) fn discover_devices() -> anyhow::Result<Vec<crate::RemovableDevice>> {
    anyhow::bail!("NIT Drive discovery currently supports Windows and Linux")
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) struct PresenceToken;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl PresenceToken {
    pub(crate) fn capture(_path: &std::path::Path) -> anyhow::Result<Self> {
        anyhow::bail!("NIT Drive removal detection currently supports Windows and Linux")
    }

    pub(crate) fn is_present(&self) -> bool {
        false
    }
}
