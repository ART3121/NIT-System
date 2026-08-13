use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, bail, Context, Result};
use nit_core::{vault::Vault, Nit, VaultWorkspaceInfo};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::{validate_provisioning_target, DeviceSource, SystemDeviceSource};

const DRIVE_DIRECTORY: &str = ".nit-drive";
const DRIVE_HEADER: &str = "header";
const VAULT_DIRECTORY: &str = "vault";
const DRIVE_FORMAT_VERSION: u16 = 1;
const DRIVE_ID_BYTES: usize = 16;
const MAX_HEADER_BYTES: u64 = 16 * 1024;

#[derive(Serialize, Deserialize)]
struct DriveHeaderV1 {
    format_version: u16,
    drive_id: String,
    vault_directory: String,
    vault_id: String,
}

/// A discovered NIT Drive using a normal mounted filesystem.
#[derive(Clone, Debug)]
pub struct NitDrive {
    root: PathBuf,
    id: String,
    vault_id: String,
}

pub struct InitializedDrive {
    pub drive: NitDrive,
    pub workspace: VaultWorkspaceInfo,
}

/// Initializes NIT Drive metadata only on a freshly revalidated removable
/// device mount. Tests can provide a fake `DeviceSource`; production uses OS
/// discovery.
pub struct NitDriveInitializer<S = SystemDeviceSource> {
    source: S,
}

impl Default for NitDriveInitializer<SystemDeviceSource> {
    fn default() -> Self {
        Self {
            source: SystemDeviceSource,
        }
    }
}

impl<S: DeviceSource> NitDriveInitializer<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }

    pub fn initialize(
        &self,
        device_id: &str,
        mount_root: &Path,
        password: &SecretString,
        workspace_name: impl Into<String>,
    ) -> Result<InitializedDrive> {
        let mount_root = mount_root
            .canonicalize()
            .with_context(|| format!("NIT Drive mount is unavailable: {}", mount_root.display()))?;
        let device = unique_device(self.source.discover()?, device_id)?;
        validate_provisioning_target(&device)?;
        if !device
            .mount_points
            .iter()
            .filter_map(|path| path.canonicalize().ok())
            .any(|path| path == mount_root)
        {
            bail!("selected directory is not a mount point of the selected removable device");
        }

        let destination = mount_root.join(DRIVE_DIRECTORY);
        reject_symlink(&destination)?;
        if destination.exists() {
            bail!("NIT Drive metadata already exists; refusing to overwrite it");
        }
        let staging = tempfile::Builder::new()
            .prefix(".nit-drive.prepare.")
            .tempdir_in(&mount_root)
            .with_context(|| format!("could not stage NIT Drive in {}", mount_root.display()))?;
        let vault_path = staging.path().join(VAULT_DIRECTORY);
        let vault = Arc::new(Vault::create(&vault_path, password)?);
        let workspace = Nit::create_vault_workspace(&vault, workspace_name)?;
        let drive_id = random_hex_id()?;
        Nit::bind_vault(&vault, &format!("nit-drive-v1:{drive_id}"))?;
        let header = DriveHeaderV1 {
            format_version: DRIVE_FORMAT_VERSION,
            drive_id: drive_id.clone(),
            vault_directory: VAULT_DIRECTORY.into(),
            vault_id: hex_encode(&vault.id()),
        };
        atomic_write(
            &staging.path().join(DRIVE_HEADER),
            serde_json::to_vec_pretty(&header)?,
        )?;
        drop(vault);
        sync_directory(&vault_path)?;
        sync_directory(staging.path())?;
        let staging_path = staging.keep();
        if let Err(error) = fs::rename(&staging_path, &destination) {
            let _ = fs::remove_dir_all(&staging_path);
            return Err(error).context("could not install NIT Drive metadata");
        }
        sync_directory(&mount_root)?;
        let drive = NitDrive::open(&mount_root)?;
        Ok(InitializedDrive { drive, workspace })
    }
}

impl NitDrive {
    pub fn open(mount_root: &Path) -> Result<Self> {
        let root = mount_root
            .canonicalize()
            .with_context(|| format!("NIT Drive is unavailable: {}", mount_root.display()))?;
        let directory = root.join(DRIVE_DIRECTORY);
        reject_symlink(&directory)?;
        if !directory.is_dir() {
            bail!("no NIT Drive metadata found at {}", root.display());
        }
        let header_path = directory.join(DRIVE_HEADER);
        reject_symlink(&header_path)?;
        let metadata = fs::metadata(&header_path)?;
        if !metadata.is_file() || metadata.len() > MAX_HEADER_BYTES {
            bail!("invalid NIT Drive header");
        }
        let header: DriveHeaderV1 = serde_json::from_slice(&fs::read(&header_path)?)
            .context("invalid NIT Drive header encoding")?;
        validate_header(&header)?;
        let vault_path = directory.join(&header.vault_directory);
        reject_symlink(&vault_path)?;
        if !vault_path.is_dir() {
            bail!("NIT Drive Vault is missing");
        }
        Ok(Self {
            root,
            id: header.drive_id,
            vault_id: header.vault_id,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn vault_path(&self) -> PathBuf {
        self.root.join(DRIVE_DIRECTORY).join(VAULT_DIRECTORY)
    }

    pub fn unlock(&self, password: &SecretString) -> Result<Arc<Vault>> {
        let vault = Arc::new(Vault::open(self.vault_path(), password)?);
        if hex_encode(&vault.id()) != self.vault_id {
            bail!("NIT Drive metadata does not match its authenticated Vault");
        }
        let expected_binding = format!("nit-drive-v1:{}", self.id);
        if Nit::vault_binding(&vault)?.as_deref() != Some(expected_binding.as_str()) {
            bail!("NIT Drive identity does not match its authenticated Vault");
        }
        Ok(vault)
    }
}

fn unique_device(
    devices: Vec<crate::RemovableDevice>,
    device_id: &str,
) -> Result<crate::RemovableDevice> {
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

fn validate_header(header: &DriveHeaderV1) -> Result<()> {
    if header.format_version != DRIVE_FORMAT_VERSION {
        bail!(
            "unsupported NIT Drive format version {}",
            header.format_version
        );
    }
    if !valid_hex_id(&header.drive_id, DRIVE_ID_BYTES)
        || !valid_hex_id(&header.vault_id, 16)
        || header.vault_directory != VAULT_DIRECTORY
    {
        bail!("invalid NIT Drive metadata");
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing symbolic link in NIT Drive: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn random_hex_id() -> Result<String> {
    let mut id = [0_u8; DRIVE_ID_BYTES];
    getrandom::fill(&mut id).context("operating system random generator is unavailable")?;
    Ok(hex_encode(&id))
}

fn valid_hex_id(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn atomic_write(path: &Path, contents: Vec<u8>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("NIT Drive header has no parent"))?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(&contents)?;
    temporary.as_file().sync_all()?;
    let file = temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    file.sync_all()?;
    drop(file);
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceSource, RemovableDevice};

    struct FakeSource(RemovableDevice);

    impl DeviceSource for FakeSource {
        fn discover(&self) -> Result<Vec<RemovableDevice>> {
            Ok(vec![self.0.clone()])
        }
    }

    fn source(root: &Path) -> FakeSource {
        FakeSource(RemovableDevice {
            id: "/dev/testusb".into(),
            model: "Test USB".into(),
            capacity_bytes: 16 * 1024 * 1024 * 1024,
            mount_points: vec![root.to_path_buf()],
            removable: true,
            system_disk: false,
            read_only: false,
        })
    }

    #[test]
    fn initializes_reopens_and_unlocks_drive_with_a_random_workspace_identity() {
        let temp = tempfile::tempdir().unwrap();
        let password = SecretString::from("password".to_owned());
        let initialized = NitDriveInitializer::new(source(temp.path()))
            .initialize("/dev/testusb", temp.path(), &password, "Portable")
            .unwrap();
        assert_eq!(initialized.workspace.name, "Portable");
        assert!(!temp.path().join(".nit").exists());
        assert!(temp.path().join(".nit-drive/vault/header").is_file());

        let reopened = NitDrive::open(temp.path()).unwrap();
        assert_eq!(reopened.id(), initialized.drive.id());
        let vault = reopened.unlock(&password).unwrap();
        assert_eq!(
            Nit::vault_workspaces(&vault).unwrap(),
            vec![initialized.workspace]
        );
    }

    #[test]
    fn refuses_overwrite_wrong_password_and_tampered_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let password = SecretString::from("password".to_owned());
        let initializer = NitDriveInitializer::new(source(temp.path()));
        initializer
            .initialize("/dev/testusb", temp.path(), &password, "Portable")
            .unwrap();
        assert!(initializer
            .initialize("/dev/testusb", temp.path(), &password, "Again")
            .is_err());
        let drive = NitDrive::open(temp.path()).unwrap();
        assert!(drive
            .unlock(&SecretString::from("wrong".to_owned()))
            .is_err());

        let path = temp.path().join(".nit-drive/header");
        let mut header: DriveHeaderV1 = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        header.drive_id = "00".repeat(DRIVE_ID_BYTES);
        fs::write(&path, serde_json::to_vec(&header).unwrap()).unwrap();
        let drive = NitDrive::open(temp.path()).unwrap();
        assert!(drive.unlock(&password).is_err());

        header.vault_directory = "../outside".into();
        fs::write(path, serde_json::to_vec(&header).unwrap()).unwrap();
        assert!(NitDrive::open(temp.path()).is_err());
    }
}
