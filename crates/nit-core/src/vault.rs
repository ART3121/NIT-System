//! Versioned, authenticated storage primitives for NIT Vault.
//!
//! This module deliberately has no knowledge of NIT domain types. The Vault
//! repository serializes the existing domain state into committed payloads,
//! keeping encryption confined to persistence.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use fs2::FileExt;
use secrecy::{ExposeSecret, SecretBox, SecretString};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

use crate::fsutil::{
    atomic_write, ensure_regular_or_missing, read_bytes_limited, reject_symlink, sync_directory,
};

const VAULT_FORMAT_VERSION: u16 = 1;
const HEADER_MAGIC: &[u8; 8] = b"NITVLT1\0";
const OBJECT_MAGIC: &[u8; 8] = b"NITOBJ1\0";
const ROOT_MAGIC: &[u8; 8] = b"NITROT1\0";
const HEADER_FILE: &str = "header";
const OBJECTS_DIRECTORY: &str = "objects";
const LOCK_FILE: &str = "lock";
const ROOT_FILES: [&str; 2] = ["root.0", "root.1"];

const VAULT_ID_BYTES: usize = 16;
const MASTER_KEY_BYTES: usize = 32;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const OBJECT_ID_BYTES: usize = 32;

const MAX_HEADER_BYTES: u64 = 4 * 1024;
const MAX_ROOT_BYTES: u64 = 4 * 1024;
const MAX_OBJECT_BYTES: u64 = 128 * 1024 * 1024 + 1024;
const MAX_PAYLOAD_BYTES: usize = 128 * 1024 * 1024;

const DEFAULT_MEMORY_COST_KIB: u32 = 64 * 1024;
const DEFAULT_TIME_COST: u32 = 3;
const DEFAULT_PARALLELISM: u32 = 4;
const MIN_MEMORY_COST_KIB: u32 = 8;
const MAX_MEMORY_COST_KIB: u32 = 1024 * 1024;
const MAX_TIME_COST: u32 = 32;
const MAX_PARALLELISM: u32 = 64;

/// Opaque identifier for an immutable encrypted Vault object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultObjectId([u8; OBJECT_ID_BYTES]);

impl VaultObjectId {
    /// Returns the opaque identifier as lowercase hexadecimal.
    pub fn as_hex(self) -> String {
        hex_encode(&self.0)
    }
}

/// An unlocked Vault backed by a normal directory.
///
/// The decrypted Master Key is zeroized when this value is dropped. It is not
/// cloneable or debug-printable. Normal multi-process use keeps this value
/// inside the Session Agent without changing the on-disk format.
pub struct Vault {
    root: PathBuf,
    vault_id: [u8; VAULT_ID_BYTES],
    master_key: SecretBox<[u8; MASTER_KEY_BYTES]>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct KdfParametersV1 {
    algorithm: u8,
    version: u32,
    memory_cost_kib: u32,
    time_cost: u32,
    parallelism: u32,
}

impl KdfParametersV1 {
    const ARGON2ID: u8 = 1;

    fn production() -> Self {
        Self {
            algorithm: Self::ARGON2ID,
            version: 0x13,
            memory_cost_kib: DEFAULT_MEMORY_COST_KIB,
            time_cost: DEFAULT_TIME_COST,
            parallelism: DEFAULT_PARALLELISM,
        }
    }

    fn validate(self) -> Result<()> {
        if self.algorithm != Self::ARGON2ID || self.version != 0x13 {
            bail!("unsupported Vault v1 password derivation algorithm");
        }
        if !(MIN_MEMORY_COST_KIB..=MAX_MEMORY_COST_KIB).contains(&self.memory_cost_kib)
            || !(1..=MAX_TIME_COST).contains(&self.time_cost)
            || !(1..=MAX_PARALLELISM).contains(&self.parallelism)
        {
            bail!("invalid Vault v1 password derivation parameters");
        }
        Ok(())
    }

    fn argon2(self) -> Result<Argon2<'static>> {
        self.validate()?;
        let params = Params::new(
            self.memory_cost_kib,
            self.time_cost,
            self.parallelism,
            Some(MASTER_KEY_BYTES),
        )
        .map_err(|_| anyhow!("invalid Vault v1 password derivation parameters"))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

#[derive(Serialize, Deserialize)]
struct VaultHeaderV1 {
    format_version: u16,
    vault_id: [u8; VAULT_ID_BYTES],
    kdf: KdfParametersV1,
    salt: [u8; SALT_BYTES],
    wrap_algorithm: u8,
    wrap_nonce: [u8; NONCE_BYTES],
    wrapped_master_key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct ObjectEnvelopeV1 {
    format_version: u16,
    object_id: VaultObjectId,
    nonce: [u8; NONCE_BYTES],
    ciphertext: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct RootEnvelopeV1 {
    format_version: u16,
    slot: u8,
    nonce: [u8; NONCE_BYTES],
    ciphertext: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct RootPointerV1 {
    generation: u64,
    object_id: VaultObjectId,
}

struct VaultLock {
    file: File,
}

impl VaultLock {
    fn open(root: &Path, exclusive: bool) -> Result<Self> {
        let path = root.join(LOCK_FILE);
        ensure_regular_or_missing(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("could not open Vault lock {}", path.display()))?;
        if exclusive {
            FileExt::lock_exclusive(&file)
        } else {
            FileExt::lock_shared(&file)
        }
        .with_context(|| format!("could not lock Vault {}", root.display()))?;
        Ok(Self { file })
    }
}

impl Drop for VaultLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl Vault {
    /// Creates a new Vault in a missing or empty directory.
    ///
    /// Existing data is never overwritten. The password protects a random
    /// Master Key and is not used to encrypt objects directly.
    pub fn create(path: impl AsRef<Path>, password: &SecretString) -> Result<Self> {
        Self::create_with_kdf(path.as_ref(), password, KdfParametersV1::production())
    }

    fn create_with_kdf(root: &Path, password: &SecretString, kdf: KdfParametersV1) -> Result<Self> {
        prepare_new_directory(root)?;
        kdf.validate()?;

        let objects = root.join(OBJECTS_DIRECTORY);
        fs::create_dir(&objects)
            .with_context(|| format!("could not create {}", objects.display()))?;

        let lock_path = root.join(LOCK_FILE);
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .with_context(|| format!("could not create Vault lock {}", lock_path.display()))?
            .sync_all()?;

        let vault_id = random_array()?;
        let salt = random_array()?;
        let wrap_nonce = random_array()?;
        let master_key = Zeroizing::new(random_array()?);
        let derived_key = derive_key(password, &salt, kdf)?;

        let mut header = VaultHeaderV1 {
            format_version: VAULT_FORMAT_VERSION,
            vault_id,
            kdf,
            salt,
            wrap_algorithm: 1,
            wrap_nonce,
            wrapped_master_key: Vec::new(),
        };
        let aad = header_aad(&header);
        header.wrapped_master_key = encrypt(
            &derived_key,
            &header.wrap_nonce,
            master_key.as_ref(),
            &aad,
            "could not protect Vault Master Key",
        )?;
        let encoded = encode_record(HEADER_MAGIC, &header)?;
        atomic_write(&root.join(HEADER_FILE), encoded)?;
        sync_directory(root)?;

        Ok(Self {
            root: root.to_path_buf(),
            vault_id,
            master_key: SecretBox::new(Box::new(*master_key)),
        })
    }

    /// Opens and authenticates an existing Vault using its password.
    pub fn open(path: impl AsRef<Path>, password: &SecretString) -> Result<Self> {
        let root = path.as_ref();
        validate_vault_directory(root)?;
        let bytes = read_bytes_limited(&root.join(HEADER_FILE), MAX_HEADER_BYTES)
            .context("could not read Vault header")?;
        let header: VaultHeaderV1 = decode_record(HEADER_MAGIC, &bytes, "Vault header")?;
        validate_header(&header)?;

        let derived_key = derive_key(password, &header.salt, header.kdf)?;
        let aad = header_aad(&header);
        let decrypted = decrypt(
            &derived_key,
            &header.wrap_nonce,
            &header.wrapped_master_key,
            &aad,
            "could not unlock Vault: incorrect password or authenticated header damage",
        )?;
        if decrypted.len() != MASTER_KEY_BYTES {
            bail!("invalid decrypted Vault Master Key length");
        }
        let mut master_key = Zeroizing::new([0_u8; MASTER_KEY_BYTES]);
        master_key.copy_from_slice(&decrypted);

        Ok(Self {
            root: root.to_path_buf(),
            vault_id: header.vault_id,
            master_key: SecretBox::new(Box::new(*master_key)),
        })
    }

    /// Returns the stable random identity stored in the authenticated header.
    pub fn id(&self) -> [u8; VAULT_ID_BYTES] {
        self.vault_id
    }

    /// Returns the directory containing this Vault's encrypted format.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Commits a non-empty state as a new immutable encrypted object.
    ///
    /// The object is persisted before an alternating authenticated root is
    /// atomically replaced, so an interrupted commit leaves the last valid root
    /// readable. Orphaned immutable objects are harmless and can be collected by
    /// a later maintenance phase.
    pub fn commit(&self, payload: &[u8]) -> Result<VaultObjectId> {
        validate_payload(payload)?;
        validate_vault_directory(&self.root)?;
        let _lock = VaultLock::open(&self.root, true)?;
        self.commit_unlocked(payload)
    }

    /// Atomically transforms the latest committed payload while holding the
    /// Vault's exclusive lock for the entire read/modify/write operation.
    pub(crate) fn transaction<T>(
        &self,
        operation: impl FnOnce(Option<&[u8]>) -> Result<(Vec<u8>, T)>,
    ) -> Result<T> {
        validate_vault_directory(&self.root)?;
        let _lock = VaultLock::open(&self.root, true)?;
        let current = self
            .read_current_root()?
            .map(|root| self.read_object(root.object_id))
            .transpose()?;
        let (next, value) = operation(current.as_deref().map(Vec::as_slice))?;
        let next = Zeroizing::new(next);
        validate_payload(&next)?;
        self.commit_unlocked(&next)?;
        Ok(value)
    }

    fn commit_unlocked(&self, payload: &[u8]) -> Result<VaultObjectId> {
        let current = self.read_current_root()?;
        let generation = current.as_ref().map_or(Ok(1), |root| {
            root.generation
                .checked_add(1)
                .ok_or_else(|| anyhow!("Vault root generation overflow"))
        })?;
        let object_id = self.write_object(payload)?;
        let slot = (generation % 2) as u8;
        self.write_root(
            slot,
            &RootPointerV1 {
                generation,
                object_id,
            },
        )?;
        Ok(object_id)
    }

    /// Loads the latest committed state, or `None` if the Vault is new.
    pub fn load_latest(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        validate_vault_directory(&self.root)?;
        let _lock = VaultLock::open(&self.root, false)?;
        let Some(root) = self.read_current_root()? else {
            return Ok(None);
        };
        self.read_object(root.object_id).map(Some)
    }

    fn write_object(&self, payload: &[u8]) -> Result<VaultObjectId> {
        for _ in 0..8 {
            let object_id = VaultObjectId(random_array()?);
            let path = self.object_path(object_id);
            if path.exists() {
                continue;
            }
            let nonce = random_array()?;
            let aad = object_aad(self.vault_id, object_id);
            let ciphertext = encrypt(
                self.master_key.expose_secret(),
                &nonce,
                payload,
                &aad,
                "could not encrypt Vault object",
            )?;
            let envelope = ObjectEnvelopeV1 {
                format_version: VAULT_FORMAT_VERSION,
                object_id,
                nonce,
                ciphertext,
            };
            let bytes = encode_record(OBJECT_MAGIC, &envelope)?;
            write_immutable(&path, &bytes)?;
            return Ok(object_id);
        }
        bail!("could not allocate a unique Vault object identifier")
    }

    fn read_object(&self, expected_id: VaultObjectId) -> Result<Zeroizing<Vec<u8>>> {
        let path = self.object_path(expected_id);
        let bytes = read_bytes_limited(&path, MAX_OBJECT_BYTES)
            .with_context(|| format!("could not read Vault object {}", expected_id.as_hex()))?;
        let envelope: ObjectEnvelopeV1 = decode_record(OBJECT_MAGIC, &bytes, "Vault object")?;
        if envelope.format_version != VAULT_FORMAT_VERSION {
            bail!(
                "unsupported Vault object version {}",
                envelope.format_version
            );
        }
        if envelope.object_id != expected_id {
            bail!("Vault object identifier does not match its authenticated root");
        }
        if envelope.ciphertext.len() <= TAG_BYTES
            || envelope.ciphertext.len() > MAX_PAYLOAD_BYTES + TAG_BYTES
        {
            bail!("invalid Vault object ciphertext length");
        }
        let aad = object_aad(self.vault_id, expected_id);
        decrypt(
            self.master_key.expose_secret(),
            &envelope.nonce,
            &envelope.ciphertext,
            &aad,
            "Vault object authentication failed",
        )
    }

    fn object_path(&self, object_id: VaultObjectId) -> PathBuf {
        self.root.join(OBJECTS_DIRECTORY).join(object_id.as_hex())
    }

    fn write_root(&self, slot: u8, pointer: &RootPointerV1) -> Result<()> {
        let nonce = random_array()?;
        let plaintext = Zeroizing::new(postcard::to_stdvec(pointer)?);
        let aad = root_aad(self.vault_id, slot);
        let ciphertext = encrypt(
            self.master_key.expose_secret(),
            &nonce,
            &plaintext,
            &aad,
            "could not encrypt Vault root",
        )?;
        let envelope = RootEnvelopeV1 {
            format_version: VAULT_FORMAT_VERSION,
            slot,
            nonce,
            ciphertext,
        };
        atomic_write(
            &self.root.join(ROOT_FILES[usize::from(slot)]),
            encode_record(ROOT_MAGIC, &envelope)?,
        )
    }

    fn read_current_root(&self) -> Result<Option<RootPointerV1>> {
        let mut valid = Vec::new();
        let mut invalid = Vec::new();
        for slot in 0_u8..=1 {
            match self.read_root(slot) {
                Ok(Some(root)) => valid.push(root),
                Ok(None) => {}
                Err(error) => invalid.push(error),
            }
        }

        if let Some(root) = valid.into_iter().max_by_key(|root| root.generation) {
            return Ok(Some(root));
        }
        if invalid.is_empty() {
            Ok(None)
        } else {
            Err(invalid.remove(0).context("no valid Vault root remains"))
        }
    }

    fn read_root(&self, slot: u8) -> Result<Option<RootPointerV1>> {
        let path = self.root.join(ROOT_FILES[usize::from(slot)]);
        let bytes = match read_bytes_limited(&path, MAX_ROOT_BYTES) {
            Ok(bytes) => bytes,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let envelope: RootEnvelopeV1 = decode_record(ROOT_MAGIC, &bytes, "Vault root")?;
        if envelope.format_version != VAULT_FORMAT_VERSION || envelope.slot != slot {
            bail!("invalid Vault root metadata in slot {slot}");
        }
        if envelope.ciphertext.len() <= TAG_BYTES || envelope.ciphertext.len() > 256 {
            bail!("invalid Vault root ciphertext length in slot {slot}");
        }
        let aad = root_aad(self.vault_id, slot);
        let plaintext = decrypt(
            self.master_key.expose_secret(),
            &envelope.nonce,
            &envelope.ciphertext,
            &aad,
            "Vault root authentication failed",
        )?;
        let pointer: RootPointerV1 = postcard::from_bytes(&plaintext)
            .map_err(|_| anyhow!("invalid authenticated Vault root payload"))?;
        if pointer.generation == 0 || (pointer.generation % 2) as u8 != slot {
            bail!("invalid authenticated Vault root generation");
        }
        Ok(Some(pointer))
    }
}

fn prepare_new_directory(root: &Path) -> Result<()> {
    reject_symlink(root)?;
    match fs::metadata(root) {
        Ok(metadata) if !metadata.is_dir() => {
            bail!("Vault path is not a directory: {}", root.display())
        }
        Ok(_) => {
            let mut entries = fs::read_dir(root)
                .with_context(|| format!("could not inspect {}", root.display()))?;
            if entries.next().transpose()?.is_some() {
                bail!("refusing to create a Vault in a non-empty directory");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(root)
                .with_context(|| format!("could not create Vault directory {}", root.display()))?;
        }
        Err(error) => return Err(error.into()),
    }
    sync_directory(
        root.parent()
            .ok_or_else(|| anyhow!("Vault directory has no parent: {}", root.display()))?,
    )
}

fn validate_vault_directory(root: &Path) -> Result<()> {
    reject_symlink(root)?;
    let metadata =
        fs::metadata(root).with_context(|| format!("Vault is unavailable: {}", root.display()))?;
    if !metadata.is_dir() {
        bail!("Vault path is not a directory: {}", root.display());
    }
    let objects = root.join(OBJECTS_DIRECTORY);
    reject_symlink(&objects)?;
    if !objects.is_dir() {
        bail!("Vault objects directory is missing: {}", objects.display());
    }
    ensure_regular_or_missing(&root.join(HEADER_FILE))?;
    ensure_regular_or_missing(&root.join(LOCK_FILE))?;
    for name in ROOT_FILES {
        ensure_regular_or_missing(&root.join(name))?;
    }
    Ok(())
}

fn validate_header(header: &VaultHeaderV1) -> Result<()> {
    if header.format_version != VAULT_FORMAT_VERSION {
        bail!("unsupported Vault format version {}", header.format_version);
    }
    header.kdf.validate()?;
    if header.wrap_algorithm != 1 {
        bail!("unsupported Vault v1 Master Key wrapping algorithm");
    }
    if header.wrapped_master_key.len() != MASTER_KEY_BYTES + TAG_BYTES {
        bail!("invalid wrapped Vault Master Key length");
    }
    Ok(())
}

fn validate_payload(payload: &[u8]) -> Result<()> {
    if payload.is_empty() {
        bail!("refusing to commit an empty Vault payload");
    }
    if payload.len() > MAX_PAYLOAD_BYTES {
        bail!(
            "Vault payload is too large ({} bytes; limit is {} bytes)",
            payload.len(),
            MAX_PAYLOAD_BYTES
        );
    }
    Ok(())
}

fn derive_key(
    password: &SecretString,
    salt: &[u8; SALT_BYTES],
    parameters: KdfParametersV1,
) -> Result<Zeroizing<[u8; MASTER_KEY_BYTES]>> {
    let mut key = Zeroizing::new([0_u8; MASTER_KEY_BYTES]);
    parameters
        .argon2()?
        .hash_password_into(password.expose_secret().as_bytes(), salt, key.as_mut())
        .map_err(|_| anyhow!("could not derive Vault password key"))?;
    Ok(key)
}

fn encrypt(
    key: &[u8; MASTER_KEY_BYTES],
    nonce: &[u8; NONCE_BYTES],
    plaintext: &[u8],
    aad: &[u8],
    message: &'static str,
) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!(message))?;
    let nonce = XNonce::from(*nonce);
    cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| anyhow!(message))
}

fn decrypt(
    key: &[u8; MASTER_KEY_BYTES],
    nonce: &[u8; NONCE_BYTES],
    ciphertext: &[u8],
    aad: &[u8],
    message: &'static str,
) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!(message))?;
    let nonce = XNonce::from(*nonce);
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| anyhow!(message))
}

fn random_array<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).context("operating system random generator is unavailable")?;
    Ok(bytes)
}

fn encode_record<T: Serialize>(magic: &[u8; 8], value: &T) -> Result<Vec<u8>> {
    let payload = postcard::to_stdvec(value).context("could not encode Vault record")?;
    let mut bytes = Vec::with_capacity(magic.len() + payload.len());
    bytes.extend_from_slice(magic);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode_record<T: DeserializeOwned>(magic: &[u8; 8], bytes: &[u8], name: &str) -> Result<T> {
    if bytes.len() <= magic.len() || &bytes[..magic.len()] != magic {
        bail!("invalid {name} magic");
    }
    postcard::from_bytes(&bytes[magic.len()..]).map_err(|_| anyhow!("invalid {name} encoding"))
}

fn header_aad(header: &VaultHeaderV1) -> Vec<u8> {
    let mut aad = Vec::with_capacity(128);
    aad.extend_from_slice(b"NIT-Vault-v1/header");
    aad.extend_from_slice(&header.format_version.to_le_bytes());
    aad.extend_from_slice(&header.vault_id);
    aad.push(header.kdf.algorithm);
    aad.extend_from_slice(&header.kdf.version.to_le_bytes());
    aad.extend_from_slice(&header.kdf.memory_cost_kib.to_le_bytes());
    aad.extend_from_slice(&header.kdf.time_cost.to_le_bytes());
    aad.extend_from_slice(&header.kdf.parallelism.to_le_bytes());
    aad.extend_from_slice(&header.salt);
    aad.push(header.wrap_algorithm);
    aad.extend_from_slice(&header.wrap_nonce);
    aad
}

fn object_aad(vault_id: [u8; VAULT_ID_BYTES], object_id: VaultObjectId) -> Vec<u8> {
    let mut aad = Vec::with_capacity(96);
    aad.extend_from_slice(b"NIT-Vault-v1/object");
    aad.extend_from_slice(&VAULT_FORMAT_VERSION.to_le_bytes());
    aad.extend_from_slice(&vault_id);
    aad.extend_from_slice(&object_id.0);
    aad
}

fn root_aad(vault_id: [u8; VAULT_ID_BYTES], slot: u8) -> Vec<u8> {
    let mut aad = Vec::with_capacity(64);
    aad.extend_from_slice(b"NIT-Vault-v1/root");
    aad.extend_from_slice(&VAULT_FORMAT_VERSION.to_le_bytes());
    aad.extend_from_slice(&vault_id);
    aad.push(slot);
    aad
}

fn write_immutable(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Vault object has no parent: {}", path.display()))?;
    reject_symlink(path)?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("could not stage Vault object in {}", parent.display()))?;
    temporary
        .write_all(contents)
        .with_context(|| format!("could not stage Vault object {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("could not sync staged Vault object {}", path.display()))?;
    let file = temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)
        .with_context(|| format!("could not persist Vault object {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("could not sync Vault object {}", path.display()))?;
    drop(file);
    sync_directory(parent)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn password(value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }

    fn test_kdf() -> KdfParametersV1 {
        KdfParametersV1 {
            algorithm: KdfParametersV1::ARGON2ID,
            version: 0x13,
            memory_cost_kib: 8 * 1024,
            time_cost: 1,
            parallelism: 1,
        }
    }

    fn create(temp: &tempfile::TempDir) -> Vault {
        Vault::create_with_kdf(
            &temp.path().join("vault"),
            &password("correct horse"),
            test_kdf(),
        )
        .unwrap()
    }

    fn flip_last_byte(path: &Path) {
        let mut bytes = fs::read(path).unwrap();
        let last = bytes.last_mut().unwrap();
        *last ^= 0x80;
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn creates_commits_and_reopens_a_vault() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        assert!(vault.load_latest().unwrap().is_none());
        vault.commit(b"first state").unwrap();
        let id = vault.id();
        drop(vault);

        let reopened = Vault::open(temp.path().join("vault"), &password("correct horse")).unwrap();
        assert_eq!(reopened.id(), id);
        assert_eq!(
            reopened.load_latest().unwrap().unwrap().as_slice(),
            b"first state"
        );
    }

    #[test]
    fn rejects_an_incorrect_password_without_exposing_key_material() {
        let temp = tempfile::tempdir().unwrap();
        drop(create(&temp));

        let error = Vault::open(temp.path().join("vault"), &password("wrong password"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("could not unlock Vault"));
    }

    #[test]
    fn refuses_to_overwrite_a_non_empty_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("important"), b"keep").unwrap();

        let error = Vault::create_with_kdf(&root, &password("password"), test_kdf())
            .err()
            .unwrap();
        assert!(error.to_string().contains("non-empty"));
        assert_eq!(fs::read(root.join("important")).unwrap(), b"keep");
    }

    #[test]
    fn independent_vaults_use_random_salts_keys_nonces_and_object_ids() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = create(&first_temp);
        let second = create(&second_temp);
        let first_object = first.commit(b"same state").unwrap();
        let second_object = second.commit(b"same state").unwrap();

        assert_ne!(first.id(), second.id());
        assert_ne!(first_object.0, second_object.0);
        assert_ne!(
            fs::read(first_temp.path().join("vault/header")).unwrap(),
            fs::read(second_temp.path().join("vault/header")).unwrap()
        );
        assert_ne!(
            fs::read(first.object_path(first_object)).unwrap(),
            fs::read(second.object_path(second_object)).unwrap()
        );
    }

    #[test]
    fn multiple_commits_load_only_the_latest_state() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        let first = vault.commit(b"one").unwrap();
        let second = vault.commit(b"two").unwrap();
        let third = vault.commit(b"three").unwrap();

        assert_ne!(first.0, second.0);
        assert_ne!(second.0, third.0);
        assert_eq!(vault.load_latest().unwrap().unwrap().as_slice(), b"three");
    }

    #[test]
    fn detects_tampered_object_ciphertext() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        let object = vault.commit(b"authenticated state").unwrap();
        flip_last_byte(&vault.object_path(object));

        let error = vault.load_latest().unwrap_err();
        assert!(error.to_string().contains("authentication failed"));
    }

    #[test]
    fn detects_truncated_objects() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        let object = vault.commit(b"authenticated state").unwrap();
        let path = vault.object_path(object);
        let mut bytes = fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 7);
        fs::write(path, bytes).unwrap();

        let error = vault.load_latest().unwrap_err();
        assert!(error.to_string().contains("invalid Vault object encoding"));
    }

    #[test]
    fn detects_a_missing_object() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        let object = vault.commit(b"state").unwrap();
        fs::remove_file(vault.object_path(object)).unwrap();

        let error = vault.load_latest().unwrap_err();
        assert!(error.to_string().contains("could not read Vault object"));
    }

    #[test]
    fn header_damage_never_becomes_a_valid_master_key() {
        let temp = tempfile::tempdir().unwrap();
        drop(create(&temp));
        flip_last_byte(&temp.path().join("vault/header"));

        let error = Vault::open(temp.path().join("vault"), &password("correct horse"))
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("could not unlock Vault")
                || error.to_string().contains("invalid Vault header encoding")
        );
    }

    #[test]
    fn rejects_an_unsupported_header_version_before_unlock() {
        let temp = tempfile::tempdir().unwrap();
        drop(create(&temp));
        let path = temp.path().join("vault/header");
        let bytes = fs::read(&path).unwrap();
        let mut header: VaultHeaderV1 =
            decode_record(HEADER_MAGIC, &bytes, "Vault header").unwrap();
        header.format_version = 2;
        fs::write(path, encode_record(HEADER_MAGIC, &header).unwrap()).unwrap();

        let error = Vault::open(temp.path().join("vault"), &password("correct horse"))
            .err()
            .unwrap();
        assert!(error
            .to_string()
            .contains("unsupported Vault format version 2"));
    }

    #[test]
    fn rejects_resource_exhausting_kdf_metadata() {
        let temp = tempfile::tempdir().unwrap();
        drop(create(&temp));
        let path = temp.path().join("vault/header");
        let bytes = fs::read(&path).unwrap();
        let mut header: VaultHeaderV1 =
            decode_record(HEADER_MAGIC, &bytes, "Vault header").unwrap();
        header.kdf.memory_cost_kib = MAX_MEMORY_COST_KIB + 1;
        fs::write(path, encode_record(HEADER_MAGIC, &header).unwrap()).unwrap();

        let error = Vault::open(temp.path().join("vault"), &password("correct horse"))
            .err()
            .unwrap();
        assert!(error
            .to_string()
            .contains("invalid Vault v1 password derivation parameters"));
    }

    #[test]
    fn rejects_empty_and_oversized_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        assert!(vault.commit(b"").unwrap_err().to_string().contains("empty"));
        let oversized = vec![0_u8; MAX_PAYLOAD_BYTES + 1];
        assert!(vault
            .commit(&oversized)
            .unwrap_err()
            .to_string()
            .contains("too large"));
    }

    #[test]
    fn object_id_is_bound_to_the_authenticated_ciphertext() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        let object = vault.commit(b"state").unwrap();
        let original = vault.object_path(object);
        let replacement = VaultObjectId([0x42; OBJECT_ID_BYTES]);
        fs::rename(&original, vault.object_path(replacement)).unwrap();

        let error = vault.read_object(replacement).unwrap_err();
        assert!(error.to_string().contains("identifier does not match"));
    }

    #[test]
    fn falls_back_to_previous_authenticated_root_after_partial_root_write() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        vault.commit(b"stable").unwrap();
        vault.commit(b"newer").unwrap();
        let newest_slot = temp.path().join("vault/root.0");
        fs::write(newest_slot, b"partial").unwrap();

        assert_eq!(vault.load_latest().unwrap().unwrap().as_slice(), b"stable");
    }

    #[test]
    fn fails_when_no_authenticated_root_remains() {
        let temp = tempfile::tempdir().unwrap();
        let vault = create(&temp);
        vault.commit(b"one").unwrap();
        vault.commit(b"two").unwrap();
        fs::write(temp.path().join("vault/root.0"), b"bad").unwrap();
        fs::write(temp.path().join("vault/root.1"), b"bad").unwrap();

        let error = vault.load_latest().unwrap_err();
        assert!(error.to_string().contains("no valid Vault root remains"));
    }

    #[test]
    fn rejects_symbolic_links_in_vault_structure() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let temp = tempfile::tempdir().unwrap();
            let vault = create(&temp);
            let header = temp.path().join("vault/header");
            let outside = temp.path().join("outside");
            fs::rename(&header, &outside).unwrap();
            symlink(&outside, &header).unwrap();

            let error = Vault::open(temp.path().join("vault"), &password("correct horse"))
                .err()
                .unwrap();
            assert!(error.to_string().contains("symbolic link"));
            drop(vault);
        }
    }
}
