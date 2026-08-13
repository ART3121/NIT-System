use std::{collections::HashSet, fmt, str::FromStr, sync::Arc};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    ids::IdSequences,
    model::{Entry, EntryId, Horizon, Kind, Notes, Roadmap, RoadmapStep},
    repository::RepositoryState,
    vault::Vault,
};

const CATALOG_FORMAT_VERSION: u16 = 1;
const WORKSPACE_ID_BYTES: usize = 16;
const MAX_WORKSPACES: usize = 1024;
const MAX_WORKSPACE_NAME_BYTES: usize = 128;

/// Stable path-independent identity of a workspace stored inside a Vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VaultWorkspaceId([u8; WORKSPACE_ID_BYTES]);

impl VaultWorkspaceId {
    pub fn as_hex(self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Display for VaultWorkspaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_hex())
    }
}

impl FromStr for VaultWorkspaceId {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.len() != WORKSPACE_ID_BYTES * 2 {
            bail!("invalid Vault workspace ID");
        }
        let mut bytes = [0_u8; WORKSPACE_ID_BYTES];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultWorkspaceInfo {
    pub id: VaultWorkspaceId,
    pub name: String,
}

#[derive(Clone)]
pub(crate) struct VaultRepository {
    vault: Arc<Vault>,
    workspace_id: VaultWorkspaceId,
}

#[derive(Serialize, Deserialize)]
struct CatalogV1 {
    format_version: u16,
    binding: Option<String>,
    workspaces: Vec<WorkspaceV1>,
}

#[derive(Serialize, Deserialize)]
struct WorkspaceV1 {
    id: VaultWorkspaceId,
    name: String,
    state: StateV1,
}

#[derive(Serialize, Deserialize)]
struct StateV1 {
    active: Vec<EntryV1>,
    archived: Vec<EntryV1>,
    next_ids: [u64; 8],
}

#[derive(Serialize, Deserialize)]
struct EntryV1 {
    id: Option<String>,
    kind: u8,
    horizon: Option<u8>,
    text: String,
    body: String,
    roadmap: Option<RoadmapV1>,
}

#[derive(Serialize, Deserialize)]
struct RoadmapV1 {
    steps: Vec<RoadmapStepV1>,
}

#[derive(Serialize, Deserialize)]
struct RoadmapStepV1 {
    title: String,
    description: String,
}

impl CatalogV1 {
    fn empty() -> Self {
        Self {
            format_version: CATALOG_FORMAT_VERSION,
            binding: None,
            workspaces: Vec::new(),
        }
    }

    fn decode(payload: Option<&[u8]>) -> Result<Self> {
        let Some(payload) = payload else {
            return Ok(Self::empty());
        };
        let catalog: Self = postcard::from_bytes(payload)
            .map_err(|_| anyhow!("invalid authenticated Vault catalog"))?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        postcard::to_stdvec(self).context("could not encode Vault catalog")
    }

    fn validate(&self) -> Result<()> {
        if self.format_version != CATALOG_FORMAT_VERSION {
            bail!(
                "unsupported authenticated Vault catalog version {}",
                self.format_version
            );
        }
        if self.workspaces.len() > MAX_WORKSPACES {
            bail!("Vault contains too many workspaces");
        }
        if self.binding.as_ref().is_some_and(|binding| {
            binding.is_empty() || binding.len() > 128 || binding.chars().any(char::is_control)
        }) {
            bail!("invalid authenticated Vault binding");
        }
        let mut ids = HashSet::new();
        for workspace in &self.workspaces {
            validate_name(&workspace.name)?;
            if !ids.insert(workspace.id) {
                bail!("duplicate workspace identity in Vault catalog");
            }
        }
        Ok(())
    }
}

impl VaultRepository {
    pub(crate) fn bind(vault: &Vault, binding: &str) -> Result<()> {
        if binding.is_empty() || binding.len() > 128 || binding.chars().any(char::is_control) {
            bail!("invalid Vault binding");
        }
        vault.transaction(|payload| {
            let mut catalog = CatalogV1::decode(payload)?;
            if catalog
                .binding
                .as_deref()
                .is_some_and(|value| value != binding)
            {
                bail!("Vault is already bound to a different context");
            }
            catalog.binding = Some(binding.to_owned());
            Ok((catalog.encode()?, ()))
        })
    }

    pub(crate) fn binding(vault: &Vault) -> Result<Option<String>> {
        let payload = vault.load_latest()?;
        Ok(CatalogV1::decode(payload.as_deref().map(Vec::as_slice))?.binding)
    }

    pub(crate) fn create_workspace(
        vault: &Arc<Vault>,
        name: impl Into<String>,
    ) -> Result<VaultWorkspaceInfo> {
        let name = name.into();
        validate_name(&name)?;
        vault.transaction(|payload| {
            let mut catalog = CatalogV1::decode(payload)?;
            if catalog.workspaces.len() >= MAX_WORKSPACES {
                bail!("Vault contains too many workspaces");
            }
            let id = unique_workspace_id(&catalog)?;
            catalog.workspaces.push(WorkspaceV1 {
                id,
                name: name.clone(),
                state: StateV1::from_domain(&RepositoryState::default()),
            });
            let info = VaultWorkspaceInfo { id, name };
            Ok((catalog.encode()?, info))
        })
    }

    pub(crate) fn list_workspaces(vault: &Vault) -> Result<Vec<VaultWorkspaceInfo>> {
        let payload = vault.load_latest()?;
        let catalog = CatalogV1::decode(payload.as_deref().map(Vec::as_slice))?;
        Ok(catalog
            .workspaces
            .into_iter()
            .map(|workspace| VaultWorkspaceInfo {
                id: workspace.id,
                name: workspace.name,
            })
            .collect())
    }

    pub(crate) fn open(vault: Arc<Vault>, workspace_id: VaultWorkspaceId) -> Result<Self> {
        let workspaces = Self::list_workspaces(&vault)?;
        if !workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id)
        {
            bail!("workspace {workspace_id} does not exist in this Vault");
        }
        Ok(Self {
            vault,
            workspace_id,
        })
    }

    pub(crate) fn info(&self) -> Result<VaultWorkspaceInfo> {
        Self::list_workspaces(&self.vault)?
            .into_iter()
            .find(|workspace| workspace.id == self.workspace_id)
            .ok_or_else(|| anyhow!("workspace {} is unavailable", self.workspace_id))
    }

    pub(crate) fn read(&self) -> Result<RepositoryState> {
        let payload = self.vault.load_latest()?;
        let catalog = CatalogV1::decode(payload.as_deref().map(Vec::as_slice))?;
        let workspace = catalog
            .workspaces
            .into_iter()
            .find(|workspace| workspace.id == self.workspace_id)
            .ok_or_else(|| anyhow!("workspace {} is unavailable", self.workspace_id))?;
        workspace.state.into_domain()
    }

    pub(crate) fn update<T>(
        &self,
        operation: impl FnOnce(&mut RepositoryState) -> Result<T>,
    ) -> Result<T> {
        self.vault.transaction(|payload| {
            let mut catalog = CatalogV1::decode(payload)?;
            let workspace = catalog
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == self.workspace_id)
                .ok_or_else(|| anyhow!("workspace {} is unavailable", self.workspace_id))?;
            let mut state = workspace.state.to_domain()?;
            let value = operation(&mut state)?;
            workspace.state = StateV1::from_domain(&state);
            Ok((catalog.encode()?, value))
        })
    }
}

impl StateV1 {
    fn from_domain(state: &RepositoryState) -> Self {
        Self {
            active: state.active.entries.iter().map(EntryV1::from).collect(),
            archived: state.archived.entries.iter().map(EntryV1::from).collect(),
            next_ids: state.sequences.values(),
        }
    }

    fn into_domain(self) -> Result<RepositoryState> {
        Ok(RepositoryState {
            active: Notes {
                entries: self
                    .active
                    .into_iter()
                    .map(EntryV1::into_domain)
                    .collect::<Result<_>>()?,
            },
            archived: Notes {
                entries: self
                    .archived
                    .into_iter()
                    .map(EntryV1::into_domain)
                    .collect::<Result<_>>()?,
            },
            sequences: IdSequences::from_values(self.next_ids)?,
        })
    }

    fn to_domain(&self) -> Result<RepositoryState> {
        Ok(RepositoryState {
            active: Notes {
                entries: self
                    .active
                    .iter()
                    .map(EntryV1::to_domain)
                    .collect::<Result<_>>()?,
            },
            archived: Notes {
                entries: self
                    .archived
                    .iter()
                    .map(EntryV1::to_domain)
                    .collect::<Result<_>>()?,
            },
            sequences: IdSequences::from_values(self.next_ids)?,
        })
    }
}

impl From<&Entry> for EntryV1 {
    fn from(entry: &Entry) -> Self {
        Self {
            id: entry.id.map(|id| id.to_string()),
            kind: encode_kind(entry.kind),
            horizon: entry.horizon.map(encode_horizon),
            text: entry.text.clone(),
            body: entry.body.clone(),
            roadmap: entry.roadmap.as_ref().map(|roadmap| RoadmapV1 {
                steps: roadmap
                    .steps
                    .iter()
                    .map(|step| RoadmapStepV1 {
                        title: step.title.clone(),
                        description: step.description.clone(),
                    })
                    .collect(),
            }),
        }
    }
}

impl EntryV1 {
    fn into_domain(self) -> Result<Entry> {
        Ok(Entry {
            id: self.id.as_deref().map(parse_entry_id).transpose()?,
            kind: decode_kind(self.kind)?,
            horizon: self.horizon.map(decode_horizon).transpose()?,
            text: self.text,
            body: self.body,
            roadmap: self.roadmap.map(RoadmapV1::into_domain),
        })
    }

    fn to_domain(&self) -> Result<Entry> {
        Ok(Entry {
            id: self.id.as_deref().map(parse_entry_id).transpose()?,
            kind: decode_kind(self.kind)?,
            horizon: self.horizon.map(decode_horizon).transpose()?,
            text: self.text.clone(),
            body: self.body.clone(),
            roadmap: self.roadmap.as_ref().map(RoadmapV1::to_domain),
        })
    }
}

impl RoadmapV1 {
    fn into_domain(self) -> Roadmap {
        Roadmap {
            steps: self
                .steps
                .into_iter()
                .map(|step| RoadmapStep {
                    title: step.title,
                    description: step.description,
                })
                .collect(),
        }
    }

    fn to_domain(&self) -> Roadmap {
        Roadmap {
            steps: self
                .steps
                .iter()
                .map(|step| RoadmapStep {
                    title: step.title.clone(),
                    description: step.description.clone(),
                })
                .collect(),
        }
    }
}

fn parse_entry_id(value: &str) -> Result<EntryId> {
    EntryId::parse(value).ok_or_else(|| anyhow!("invalid Entry ID in authenticated Vault state"))
}

fn encode_kind(kind: Kind) -> u8 {
    match kind {
        Kind::Idea => 1,
        Kind::Note => 2,
        Kind::Item => 3,
        Kind::Todo => 4,
    }
}

fn decode_kind(value: u8) -> Result<Kind> {
    match value {
        1 => Ok(Kind::Idea),
        2 => Ok(Kind::Note),
        3 => Ok(Kind::Item),
        4 => Ok(Kind::Todo),
        _ => bail!("invalid Entry kind in authenticated Vault state"),
    }
}

fn encode_horizon(horizon: Horizon) -> u8 {
    match horizon {
        Horizon::Short => 1,
        Horizon::Medium => 2,
        Horizon::Long => 3,
    }
}

fn decode_horizon(value: u8) -> Result<Horizon> {
    match value {
        1 => Ok(Horizon::Short),
        2 => Ok(Horizon::Medium),
        3 => Ok(Horizon::Long),
        _ => bail!("invalid Entry horizon in authenticated Vault state"),
    }
}

fn validate_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name {
        bail!("Vault workspace name must be non-empty and have no surrounding whitespace");
    }
    if name.len() > MAX_WORKSPACE_NAME_BYTES || name.chars().any(char::is_control) {
        bail!("invalid Vault workspace name");
    }
    Ok(())
}

fn unique_workspace_id(catalog: &CatalogV1) -> Result<VaultWorkspaceId> {
    for _ in 0..8 {
        let mut bytes = [0_u8; WORKSPACE_ID_BYTES];
        getrandom::fill(&mut bytes).context("operating system random generator is unavailable")?;
        let candidate = VaultWorkspaceId(bytes);
        if !catalog
            .workspaces
            .iter()
            .any(|workspace| workspace.id == candidate)
        {
            return Ok(candidate);
        }
    }
    bail!("could not allocate a unique Vault workspace identity")
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

fn hex_digit(value: u8) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => bail!("invalid Vault workspace ID"),
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;

    fn unlocked_vault(temp: &tempfile::TempDir) -> Arc<Vault> {
        Arc::new(
            Vault::create(
                temp.path().join("vault"),
                &SecretString::from("password".to_owned()),
            )
            .unwrap(),
        )
    }

    #[test]
    fn workspace_ids_round_trip_without_paths() {
        let temp = tempfile::tempdir().unwrap();
        let vault = unlocked_vault(&temp);
        let info = VaultRepository::create_workspace(&vault, "Portable project").unwrap();

        assert_eq!(
            info.id.to_string().parse::<VaultWorkspaceId>().unwrap(),
            info.id
        );
        assert_eq!(
            VaultRepository::list_workspaces(&vault).unwrap(),
            vec![info]
        );
    }

    #[test]
    fn supports_multiple_independent_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let vault = unlocked_vault(&temp);
        let first = VaultRepository::create_workspace(&vault, "First").unwrap();
        let second = VaultRepository::create_workspace(&vault, "Second").unwrap();
        let first_repository = VaultRepository::open(vault.clone(), first.id).unwrap();
        let second_repository = VaultRepository::open(vault, second.id).unwrap();

        first_repository
            .update(|state| {
                state.active.entries.push(Entry {
                    id: EntryId::new(None, Kind::Note, 1),
                    kind: Kind::Note,
                    horizon: None,
                    text: "Only first".into(),
                    body: String::new(),
                    roadmap: None,
                });
                Ok(())
            })
            .unwrap();

        assert_eq!(first_repository.read().unwrap().active.entries.len(), 1);
        assert!(second_repository.read().unwrap().active.entries.is_empty());
    }

    #[test]
    fn rejects_invalid_or_missing_workspace_identity() {
        assert!("short".parse::<VaultWorkspaceId>().is_err());
        let temp = tempfile::tempdir().unwrap();
        let vault = unlocked_vault(&temp);
        let missing = VaultWorkspaceId([7; WORKSPACE_ID_BYTES]);
        assert!(VaultRepository::open(vault, missing).is_err());
    }
}
