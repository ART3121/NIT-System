//! Ephemeral local IPC session for unlocked NIT Vaults.
//!
//! The agent owns the unlocked `Vault`/`Nit` values. Passwords cross IPC only
//! during unlock, are moved immediately into `SecretString`, and are never
//! logged or persisted. Domain operations will be added to this protocol in the
//! CLI integration phase; the lifecycle and transport are established here.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use interprocess::local_socket::prelude::*;
use nit_core::{
    vault::Vault, Entry, EntryId, Horizon, Kind, LocatedEntry, Nit, NitApi, Notes, Roadmap,
    Status as NitStatus, VaultWorkspaceId, View,
};
use nit_drive::{NitDrive, RemovalDetector};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

mod transport;

const PROTOCOL_VERSION: u16 = 1;
const MAX_MESSAGE_BYTES: usize = 128 * 1024 * 1024 + 1024 * 1024;

/// Public session state returned without exposing key material or plaintext.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Locked,
    Unlocked {
        vault_id: String,
        workspace_id: String,
        workspace_name: String,
    },
    Unavailable,
}

#[derive(Serialize, Deserialize)]
enum Request {
    Status {
        protocol: u16,
    },
    Unlock {
        protocol: u16,
        vault_path: PathBuf,
        workspace_id: String,
        password: String,
    },
    UnlockDrive {
        protocol: u16,
        drive_root: PathBuf,
        workspace_id: String,
        password: String,
    },
    Lock {
        protocol: u16,
    },
    Shutdown {
        protocol: u16,
    },
    Load {
        protocol: u16,
        view: View,
    },
    Save {
        protocol: u16,
        view: View,
        expected: Notes,
        notes: Notes,
    },
    All {
        protocol: u16,
    },
    SaveAll {
        protocol: u16,
        expected_active: Notes,
        expected_archived: Notes,
        active: Notes,
        archived: Notes,
    },
    NitStatus {
        protocol: u16,
    },
    FindById {
        protocol: u16,
        id: EntryId,
    },
    Search {
        protocol: u16,
        query: String,
        views: Vec<View>,
        classification: Option<(Kind, Option<Horizon>)>,
    },
    Create {
        protocol: u16,
        kind: Kind,
        horizon: Option<Horizon>,
        text: String,
    },
    Archive {
        protocol: u16,
        query: String,
    },
    Import {
        protocol: u16,
        source: PathBuf,
    },
    RoadmapTarget {
        protocol: u16,
        id: EntryId,
    },
    AttachRoadmap {
        protocol: u16,
        entry: Entry,
        roadmap: Roadmap,
    },
}

impl Request {
    fn protocol(&self) -> u16 {
        match self {
            Self::Status { protocol }
            | Self::Unlock { protocol, .. }
            | Self::UnlockDrive { protocol, .. }
            | Self::Lock { protocol }
            | Self::Shutdown { protocol }
            | Self::Load { protocol, .. }
            | Self::Save { protocol, .. }
            | Self::All { protocol }
            | Self::SaveAll { protocol, .. }
            | Self::NitStatus { protocol }
            | Self::FindById { protocol, .. }
            | Self::Search { protocol, .. }
            | Self::Create { protocol, .. }
            | Self::Archive { protocol, .. }
            | Self::Import { protocol, .. }
            | Self::RoadmapTarget { protocol, .. }
            | Self::AttachRoadmap { protocol, .. } => *protocol,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Reply {
    Session(SessionStatus),
    Notes(Notes),
    All(Notes, Notes),
    NitStatus(NitStatus),
    LocatedEntry(LocatedEntry),
    Matches(Vec<(View, Entry)>),
    EntryId(EntryId),
    Entry(Entry),
    Count(usize),
    Unit,
}

#[derive(Serialize, Deserialize)]
struct Response {
    protocol: u16,
    status: SessionStatus,
    reply: Option<Reply>,
    error: Option<String>,
}

struct UnlockedSession {
    nit: Arc<Mutex<Option<Nit>>>,
    unavailable: Arc<AtomicBool>,
    stop_monitor: Arc<AtomicBool>,
    monitor: Option<thread::JoinHandle<()>>,
    availability_marker: PathBuf,
    vault_id: String,
    workspace_id: String,
    workspace_name: String,
}

impl UnlockedSession {
    fn new(
        nit: Nit,
        vault_path: PathBuf,
        availability_marker: PathBuf,
        vault_id: String,
        workspace_id: String,
        workspace_name: String,
    ) -> Result<Self> {
        let detector = RemovalDetector::capture(&vault_path)?;
        let nit = Arc::new(Mutex::new(Some(nit)));
        let unavailable = Arc::new(AtomicBool::new(false));
        let stop_monitor = Arc::new(AtomicBool::new(false));
        let monitored_nit = Arc::clone(&nit);
        let monitored_unavailable = Arc::clone(&unavailable);
        let monitored_stop = Arc::clone(&stop_monitor);
        let monitor = thread::Builder::new()
            .name("nit-drive-removal-monitor".into())
            .spawn(move || {
                while !monitored_stop.load(Ordering::Acquire) {
                    if !detector.is_present() {
                        if let Ok(mut nit) = monitored_nit.lock() {
                            *nit = None;
                        }
                        monitored_unavailable.store(true, Ordering::Release);
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            })
            .context("could not start NIT Drive removal monitor")?;
        Ok(Self {
            nit,
            unavailable,
            stop_monitor,
            monitor: Some(monitor),
            availability_marker,
            vault_id,
            workspace_id,
            workspace_name,
        })
    }

    fn with_nit<T>(&self, operation: impl FnOnce(&Nit) -> Result<T>) -> Result<T> {
        let nit = self
            .nit
            .lock()
            .map_err(|_| anyhow!("NIT Session state is unavailable"))?;
        operation(
            nit.as_ref().ok_or_else(|| {
                anyhow!("NIT Drive is unavailable; reconnect and unlock it again")
            })?,
        )
    }
}

impl Drop for UnlockedSession {
    fn drop(&mut self) {
        self.stop_monitor.store(true, Ordering::Release);
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
        if let Ok(mut nit) = self.nit.lock() {
            *nit = None;
        }
    }
}

#[derive(Default)]
struct AgentState {
    unlocked: Option<UnlockedSession>,
    unavailable: bool,
}

impl AgentState {
    fn status(&mut self) -> SessionStatus {
        if self.unlocked.as_ref().is_some_and(|session| {
            session.unavailable.load(Ordering::Acquire) || !session.availability_marker.is_file()
        }) {
            self.unlocked = None;
            self.unavailable = true;
        }
        match &self.unlocked {
            Some(session) => SessionStatus::Unlocked {
                vault_id: session.vault_id.clone(),
                workspace_id: session.workspace_id.clone(),
                workspace_name: session.workspace_name.clone(),
            },
            None if self.unavailable => SessionStatus::Unavailable,
            None => SessionStatus::Locked,
        }
    }

    fn unlock(
        &mut self,
        vault_path: &Path,
        workspace_id: &str,
        password: SecretString,
    ) -> Result<SessionStatus> {
        let vault = Arc::new(Vault::open(vault_path, &password)?);
        drop(password);
        self.activate(vault, vault_path, vault_path.join("header"), workspace_id)
    }

    fn unlock_drive(
        &mut self,
        drive_root: &Path,
        workspace_id: &str,
        password: SecretString,
    ) -> Result<SessionStatus> {
        let drive = NitDrive::open(drive_root)?;
        let vault = drive.unlock(&password)?;
        drop(password);
        self.activate(
            vault,
            drive.root(),
            drive.root().join(".nit-drive/header"),
            workspace_id,
        )
    }

    fn activate(
        &mut self,
        vault: Arc<Vault>,
        presence_path: &Path,
        availability_marker: PathBuf,
        workspace_id: &str,
    ) -> Result<SessionStatus> {
        let workspace_id: VaultWorkspaceId = workspace_id.parse()?;
        let vault_id = hex_encode(&vault.id());
        let nit = Nit::open_vault(vault.clone(), workspace_id)?;
        let info = nit
            .vault_workspace()?
            .ok_or_else(|| anyhow!("Vault workspace is unavailable"))?;
        self.unlocked = Some(UnlockedSession::new(
            nit,
            presence_path.to_path_buf(),
            availability_marker,
            vault_id,
            info.id.to_string(),
            info.name,
        )?);
        self.unavailable = false;
        Ok(self.status())
    }

    fn lock(&mut self) -> SessionStatus {
        self.unlocked = None;
        self.unavailable = false;
        SessionStatus::Locked
    }

    fn with_nit<T>(&mut self, operation: impl FnOnce(&Nit) -> Result<T>) -> Result<T> {
        match self.status() {
            SessionStatus::Unlocked { .. } => self
                .unlocked
                .as_ref()
                .expect("unlocked status requires a session")
                .with_nit(operation),
            SessionStatus::Unavailable => {
                bail!("NIT Drive is unavailable; reconnect and unlock it again")
            }
            SessionStatus::Locked => bail!("NIT Vault is locked; run `nit -unlock` first"),
        }
    }
}

/// Blocking Session Agent. Use one per desktop login/session.
pub struct SessionAgent;

impl SessionAgent {
    /// Serves requests until the agent receives its internal shutdown command.
    /// `endpoint` is mapped to a Unix local socket on Unix and a Named Pipe on
    /// Windows by the transport crate.
    pub fn serve(endpoint: &str) -> Result<()> {
        validate_endpoint(endpoint)?;
        let listener = transport::listen(endpoint)?;
        let mut state = AgentState::default();
        for connection in listener.incoming() {
            let mut connection = connection.context("NIT Session connection failed")?;
            if transport::authenticate(&connection).is_err() {
                continue;
            }
            let request = match read_message::<Request>(&mut connection) {
                Ok(request) => request,
                Err(error) => {
                    let _ = write_response(
                        &mut connection,
                        Response {
                            protocol: PROTOCOL_VERSION,
                            status: state.status(),
                            reply: None,
                            error: Some(error.to_string()),
                        },
                    );
                    continue;
                }
            };
            let shutdown = matches!(request, Request::Shutdown { .. });
            let response = handle_request(&mut state, request);
            let _ = write_response(&mut connection, response);
            if shutdown {
                break;
            }
        }
        state.lock();
        Ok(())
    }
}

/// Synchronous client for the local Session Agent.
#[derive(Clone, Debug)]
pub struct SessionClient {
    endpoint: String,
    snapshots: Arc<Mutex<[Option<Notes>; 2]>>,
}

impl Default for SessionClient {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            snapshots: Arc::new(Mutex::new([None, None])),
        }
    }
}

impl SessionClient {
    pub fn new(endpoint: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        validate_endpoint(&endpoint)?;
        Ok(Self {
            endpoint,
            snapshots: Arc::new(Mutex::new([None, None])),
        })
    }

    pub fn status(&self) -> Result<SessionStatus> {
        match self.call(Request::Status {
            protocol: PROTOCOL_VERSION,
        })? {
            Reply::Session(status) => Ok(status),
            _ => bail!("invalid NIT Session status response"),
        }
    }

    pub fn unlock(
        &self,
        vault_path: impl Into<PathBuf>,
        workspace_id: VaultWorkspaceId,
        mut password: String,
    ) -> Result<SessionStatus> {
        let request = Request::Unlock {
            protocol: PROTOCOL_VERSION,
            vault_path: vault_path.into(),
            workspace_id: workspace_id.to_string(),
            password: std::mem::take(&mut password),
        };
        password.zeroize();
        match self.call(request)? {
            Reply::Session(status) => {
                self.clear_snapshots()?;
                Ok(status)
            }
            _ => bail!("invalid NIT Session unlock response"),
        }
    }

    pub fn unlock_drive(
        &self,
        drive_root: impl Into<PathBuf>,
        workspace_id: VaultWorkspaceId,
        mut password: String,
    ) -> Result<SessionStatus> {
        let request = Request::UnlockDrive {
            protocol: PROTOCOL_VERSION,
            drive_root: drive_root.into(),
            workspace_id: workspace_id.to_string(),
            password: std::mem::take(&mut password),
        };
        password.zeroize();
        match self.call(request)? {
            Reply::Session(status) => {
                self.clear_snapshots()?;
                Ok(status)
            }
            _ => bail!("invalid NIT Session NIT Drive unlock response"),
        }
    }

    pub fn lock(&self) -> Result<SessionStatus> {
        match self.call(Request::Lock {
            protocol: PROTOCOL_VERSION,
        })? {
            Reply::Session(status) => {
                self.clear_snapshots()?;
                Ok(status)
            }
            _ => bail!("invalid NIT Session lock response"),
        }
    }

    #[doc(hidden)]
    pub fn shutdown_agent(&self) -> Result<SessionStatus> {
        match self.call(Request::Shutdown {
            protocol: PROTOCOL_VERSION,
        })? {
            Reply::Session(status) => Ok(status),
            _ => bail!("invalid NIT Session shutdown response"),
        }
    }

    fn call(&self, request: Request) -> Result<Reply> {
        let mut connection = transport::connect(&self.endpoint)?;
        write_message(&mut connection, &request)?;
        let response: Response = read_message(&mut connection)?;
        if response.protocol != PROTOCOL_VERSION {
            bail!("incompatible NIT Session Agent protocol");
        }
        if let Some(error) = response.error {
            bail!(error);
        }
        response
            .reply
            .ok_or_else(|| anyhow!("NIT Session Agent returned no result"))
    }
}

impl NitApi for SessionClient {
    fn allows_external_editor(&self) -> bool {
        false
    }

    fn load(&self, view: View) -> Result<Notes> {
        let notes = match self.call(Request::Load {
            protocol: PROTOCOL_VERSION,
            view,
        })? {
            Reply::Notes(notes) => notes,
            _ => bail!("invalid NIT Session load response"),
        };
        self.remember(view, &notes)?;
        Ok(notes)
    }

    fn save(&self, view: View, notes: &Notes) -> Result<()> {
        let expected = self.expected(view)?;
        expect_unit(self.call(Request::Save {
            protocol: PROTOCOL_VERSION,
            view,
            expected,
            notes: notes.clone(),
        })?)?;
        self.remember(view, notes)
    }

    fn all(&self) -> Result<(Notes, Notes)> {
        let (active, archived) = match self.call(Request::All {
            protocol: PROTOCOL_VERSION,
        })? {
            Reply::All(active, archived) => (active, archived),
            _ => bail!("invalid NIT Session all response"),
        };
        self.remember_all(&active, &archived)?;
        Ok((active, archived))
    }

    fn save_all(&self, active: &Notes, archived: &Notes) -> Result<()> {
        let expected_active = self.expected(View::Active)?;
        let expected_archived = self.expected(View::Archived)?;
        expect_unit(self.call(Request::SaveAll {
            protocol: PROTOCOL_VERSION,
            expected_active,
            expected_archived,
            active: active.clone(),
            archived: archived.clone(),
        })?)?;
        self.remember_all(active, archived)
    }

    fn status(&self) -> Result<NitStatus> {
        match self.call(Request::NitStatus {
            protocol: PROTOCOL_VERSION,
        })? {
            Reply::NitStatus(status) => Ok(status),
            _ => bail!("invalid NIT Session NIT status response"),
        }
    }

    fn find_by_id(&self, id: EntryId) -> Result<LocatedEntry> {
        match self.call(Request::FindById {
            protocol: PROTOCOL_VERSION,
            id,
        })? {
            Reply::LocatedEntry(entry) => Ok(entry),
            _ => bail!("invalid NIT Session find response"),
        }
    }

    fn search(
        &self,
        query: &str,
        views: &[View],
        classification: Option<(Kind, Option<Horizon>)>,
    ) -> Result<Vec<(View, Entry)>> {
        match self.call(Request::Search {
            protocol: PROTOCOL_VERSION,
            query: query.to_owned(),
            views: views.to_vec(),
            classification,
        })? {
            Reply::Matches(matches) => Ok(matches),
            _ => bail!("invalid NIT Session search response"),
        }
    }

    fn create(&self, kind: Kind, horizon: Option<Horizon>, text: String) -> Result<EntryId> {
        let id = match self.call(Request::Create {
            protocol: PROTOCOL_VERSION,
            kind,
            horizon,
            text,
        })? {
            Reply::EntryId(id) => id,
            _ => bail!("invalid NIT Session create response"),
        };
        self.all()?;
        Ok(id)
    }

    fn archive(&self, query: &str) -> Result<()> {
        expect_unit(self.call(Request::Archive {
            protocol: PROTOCOL_VERSION,
            query: query.to_owned(),
        })?)?;
        self.all().map(|_| ())
    }

    fn import(&self, source: &Path) -> Result<usize> {
        let count = match self.call(Request::Import {
            protocol: PROTOCOL_VERSION,
            source: source.to_path_buf(),
        })? {
            Reply::Count(count) => count,
            _ => bail!("invalid NIT Session import response"),
        };
        self.all()?;
        Ok(count)
    }

    fn roadmap_target(&self, id: EntryId) -> Result<Entry> {
        match self.call(Request::RoadmapTarget {
            protocol: PROTOCOL_VERSION,
            id,
        })? {
            Reply::Entry(entry) => Ok(entry),
            _ => bail!("invalid NIT Session Roadmap response"),
        }
    }

    fn attach_roadmap(&self, entry: &Entry, roadmap: Roadmap) -> Result<()> {
        expect_unit(self.call(Request::AttachRoadmap {
            protocol: PROTOCOL_VERSION,
            entry: entry.clone(),
            roadmap,
        })?)?;
        self.all().map(|_| ())
    }
}

impl SessionClient {
    fn clear_snapshots(&self) -> Result<()> {
        *self
            .snapshots
            .lock()
            .map_err(|_| anyhow!("session snapshot state is unavailable"))? = [None, None];
        Ok(())
    }

    fn remember(&self, view: View, notes: &Notes) -> Result<()> {
        self.snapshots
            .lock()
            .map_err(|_| anyhow!("session snapshot state is unavailable"))?[view_index(view)] =
            Some(notes.clone());
        Ok(())
    }

    fn remember_all(&self, active: &Notes, archived: &Notes) -> Result<()> {
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| anyhow!("session snapshot state is unavailable"))?;
        snapshots[0] = Some(active.clone());
        snapshots[1] = Some(archived.clone());
        Ok(())
    }

    fn expected(&self, view: View) -> Result<Notes> {
        self.snapshots
            .lock()
            .map_err(|_| anyhow!("session snapshot state is unavailable"))?[view_index(view)]
        .clone()
        .ok_or_else(|| anyhow!("load this workspace view before saving it"))
    }
}

fn view_index(view: View) -> usize {
    match view {
        View::Active => 0,
        View::Archived => 1,
    }
}

fn expect_unit(reply: Reply) -> Result<()> {
    match reply {
        Reply::Unit => Ok(()),
        _ => bail!("invalid NIT Session operation response"),
    }
}

fn handle_request(state: &mut AgentState, mut request: Request) -> Response {
    if request.protocol() != PROTOCOL_VERSION {
        return Response {
            protocol: PROTOCOL_VERSION,
            status: state.status(),
            reply: None,
            error: Some("incompatible NIT Session Agent protocol".into()),
        };
    }
    let result: Result<Reply> = match &mut request {
        Request::Status { .. } => Ok(Reply::Session(state.status())),
        Request::Unlock {
            vault_path,
            workspace_id,
            password,
            ..
        } => {
            let secret = SecretString::from(std::mem::take(password));
            state
                .unlock(vault_path, workspace_id, secret)
                .map(Reply::Session)
        }
        Request::UnlockDrive {
            drive_root,
            workspace_id,
            password,
            ..
        } => {
            let secret = SecretString::from(std::mem::take(password));
            state
                .unlock_drive(drive_root, workspace_id, secret)
                .map(Reply::Session)
        }
        Request::Lock { .. } | Request::Shutdown { .. } => Ok(Reply::Session(state.lock())),
        Request::Load { view, .. } => state.with_nit(|nit| nit.load(*view)).map(Reply::Notes),
        Request::Save {
            view,
            expected,
            notes,
            ..
        } => state
            .with_nit(|nit| nit.save_if_unchanged(*view, expected, notes))
            .map(|()| Reply::Unit),
        Request::All { .. } => state
            .with_nit(Nit::all)
            .map(|(active, archived)| Reply::All(active, archived)),
        Request::SaveAll {
            expected_active,
            expected_archived,
            active,
            archived,
            ..
        } => state
            .with_nit(|nit| {
                nit.save_all_if_unchanged(expected_active, expected_archived, active, archived)
            })
            .map(|()| Reply::Unit),
        Request::NitStatus { .. } => state.with_nit(Nit::status).map(Reply::NitStatus),
        Request::FindById { id, .. } => state
            .with_nit(|nit| nit.find_by_id(*id))
            .map(Reply::LocatedEntry),
        Request::Search {
            query,
            views,
            classification,
            ..
        } => state
            .with_nit(|nit| nit.search(query, views, *classification))
            .map(Reply::Matches),
        Request::Create {
            kind,
            horizon,
            text,
            ..
        } => state
            .with_nit(|nit| nit.create(*kind, *horizon, std::mem::take(text)))
            .map(Reply::EntryId),
        Request::Archive { query, .. } => state
            .with_nit(|nit| nit.archive(query))
            .map(|()| Reply::Unit),
        Request::Import { source, .. } => {
            state.with_nit(|nit| nit.import(source)).map(Reply::Count)
        }
        Request::RoadmapTarget { id, .. } => state
            .with_nit(|nit| nit.roadmap_target(*id))
            .map(Reply::Entry),
        Request::AttachRoadmap { entry, roadmap, .. } => state
            .with_nit(|nit| nit.attach_roadmap(entry, roadmap.clone()))
            .map(|()| Reply::Unit),
    };
    let (reply, error) = match result {
        Ok(reply) => (Some(reply), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Response {
        protocol: PROTOCOL_VERSION,
        status: state.status(),
        reply,
        error,
    }
}

fn read_message<T: for<'de> Deserialize<'de>>(reader: &mut impl Read) -> Result<T> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(1024));
    let mut byte = [0_u8; 1];
    loop {
        if bytes.len() >= MAX_MESSAGE_BYTES {
            bail!("NIT Session message exceeds size limit");
        }
        reader
            .read_exact(&mut byte)
            .context("incomplete NIT Session message")?;
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }
    serde_json::from_slice(&bytes).context("invalid NIT Session message")
}

fn write_message<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(value).context("could not encode NIT Session message")?;
    if bytes.len() >= MAX_MESSAGE_BYTES {
        bytes.zeroize();
        bail!("NIT Session message exceeds size limit");
    }
    bytes.push(b'\n');
    let result = writer
        .write_all(&bytes)
        .context("could not send NIT Session message");
    bytes.zeroize();
    result
}

fn write_response(writer: &mut impl Write, response: Response) -> Result<()> {
    write_message(writer, &response)
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    if endpoint.is_empty()
        || endpoint.len() > 96
        || !endpoint
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("invalid NIT Session endpoint name");
    }
    Ok(())
}

/// Stable per-user endpoint used by CLI, TUI and Desktop.
pub fn default_endpoint() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".into());
    let user = user
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        .take(32)
        .map(char::from)
        .collect::<String>();
    format!(
        "nit-system-session-v1-{}",
        if user.is_empty() { "user" } else { &user }
    )
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
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::Duration,
    };

    use secrecy::SecretString;

    use nit_drive::{DeviceSource, NitDriveInitializer, RemovableDevice};

    use super::*;

    static NEXT_ENDPOINT: AtomicU64 = AtomicU64::new(1);

    struct FakeDriveSource(RemovableDevice);

    impl DeviceSource for FakeDriveSource {
        fn discover(&self) -> Result<Vec<RemovableDevice>> {
            Ok(vec![self.0.clone()])
        }
    }

    fn endpoint() -> String {
        format!(
            "nit-session-test-{}-{}",
            std::process::id(),
            NEXT_ENDPOINT.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn start_agent(endpoint: &str) -> thread::JoinHandle<()> {
        let endpoint = endpoint.to_owned();
        thread::spawn(move || SessionAgent::serve(&endpoint).unwrap())
    }

    fn wait_for_agent(client: &SessionClient) {
        for _ in 0..100 {
            if client.status().is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("agent did not start");
    }

    #[test]
    fn unlock_is_reused_and_explicit_lock_discards_the_session() {
        let temp = tempfile::tempdir().unwrap();
        let password = SecretString::from("password".to_owned());
        let vault = Arc::new(Vault::create(temp.path().join("vault"), &password).unwrap());
        let workspace = Nit::create_vault_workspace(&vault, "Portable").unwrap();
        drop(vault);

        let endpoint = endpoint();
        let handle = start_agent(&endpoint);
        let first = SessionClient::new(&endpoint).unwrap();
        let second = SessionClient::new(&endpoint).unwrap();
        wait_for_agent(&first);
        assert_eq!(first.status().unwrap(), SessionStatus::Locked);
        assert!(matches!(
            first
                .unlock(temp.path().join("vault"), workspace.id, "password".into())
                .unwrap(),
            SessionStatus::Unlocked { .. }
        ));
        assert!(matches!(
            second.status().unwrap(),
            SessionStatus::Unlocked { .. }
        ));
        let stale = NitApi::load(&second, View::Active).unwrap();
        let id = NitApi::create(&first, Kind::Note, None, "Shared through IPC".into()).unwrap();
        assert_eq!(id.to_string(), "N-0001");
        assert_eq!(NitApi::load(&first, View::Active).unwrap().entries.len(), 1);
        assert!(NitApi::save(&second, View::Active, &stale).is_err());
        assert_eq!(NitApi::status(&first).unwrap().active_entries, 1);
        assert_eq!(second.lock().unwrap(), SessionStatus::Locked);
        assert_eq!(first.status().unwrap(), SessionStatus::Locked);
        first.shutdown_agent().unwrap();
        handle.join().unwrap();
        assert!(first.status().is_err());
    }

    #[test]
    fn wrong_password_does_not_unlock_or_kill_the_agent() {
        let temp = tempfile::tempdir().unwrap();
        let vault = Arc::new(
            Vault::create(
                temp.path().join("vault"),
                &SecretString::from("correct".to_owned()),
            )
            .unwrap(),
        );
        let workspace = Nit::create_vault_workspace(&vault, "Portable").unwrap();
        drop(vault);
        let endpoint = endpoint();
        let handle = start_agent(&endpoint);
        let client = SessionClient::new(&endpoint).unwrap();
        wait_for_agent(&client);

        assert!(client
            .unlock(temp.path().join("vault"), workspace.id, "wrong".into())
            .is_err());
        assert_eq!(client.status().unwrap(), SessionStatus::Locked);
        client.shutdown_agent().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn malformed_or_competing_local_connections_do_not_kill_the_agent() {
        let endpoint = endpoint();
        let handle = start_agent(&endpoint);
        let client = SessionClient::new(&endpoint).unwrap();
        wait_for_agent(&client);

        assert!(transport::listen(&endpoint).is_err());
        assert_eq!(client.status().unwrap(), SessionStatus::Locked);
        drop(transport::connect(&endpoint).unwrap());
        assert_eq!(client.status().unwrap(), SessionStatus::Locked);

        client.shutdown_agent().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn disappearance_invalidates_the_unlocked_session() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        let vault = Arc::new(
            Vault::create(&vault_path, &SecretString::from("password".to_owned())).unwrap(),
        );
        let workspace = Nit::create_vault_workspace(&vault, "Portable").unwrap();
        drop(vault);
        let endpoint = endpoint();
        let handle = start_agent(&endpoint);
        let client = SessionClient::new(&endpoint).unwrap();
        wait_for_agent(&client);
        client
            .unlock(&vault_path, workspace.id, "password".into())
            .unwrap();

        let detached = temp.path().join("detached");
        std::fs::rename(&vault_path, &detached).unwrap();
        assert_eq!(client.status().unwrap(), SessionStatus::Unavailable);
        std::fs::rename(&detached, &vault_path).unwrap();
        assert_eq!(client.status().unwrap(), SessionStatus::Unavailable);
        assert!(matches!(
            client
                .unlock(&vault_path, workspace.id, "password".into())
                .unwrap(),
            SessionStatus::Unlocked { .. }
        ));
        client.shutdown_agent().unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn removal_monitor_revokes_the_key_slot_without_a_client_request() {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        let vault = Arc::new(
            Vault::create(&vault_path, &SecretString::from("password".to_owned())).unwrap(),
        );
        let workspace = Nit::create_vault_workspace(&vault, "Portable").unwrap();
        drop(vault);
        let mut state = AgentState::default();
        state
            .unlock(
                &vault_path,
                &workspace.id.to_string(),
                SecretString::from("password".to_owned()),
            )
            .unwrap();
        let key_slot = Arc::clone(&state.unlocked.as_ref().unwrap().nit);

        let detached = temp.path().join("detached");
        std::fs::rename(&vault_path, detached).unwrap();
        for _ in 0..50 {
            if key_slot.lock().unwrap().is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(key_slot.lock().unwrap().is_none());
        assert_eq!(state.status(), SessionStatus::Unavailable);
    }

    #[test]
    fn unlocks_a_versioned_nit_drive_and_serves_domain_operations() {
        let temp = tempfile::tempdir().unwrap();
        let password = SecretString::from("password".to_owned());
        let initialized = NitDriveInitializer::new(FakeDriveSource(RemovableDevice {
            id: "/dev/test".into(),
            model: "Test Drive".into(),
            capacity_bytes: 1024 * 1024 * 1024,
            mount_points: vec![temp.path().to_path_buf()],
            removable: true,
            system_disk: false,
            read_only: false,
        }))
        .initialize("/dev/test", temp.path(), &password, "Portable")
        .unwrap();
        let endpoint = endpoint();
        let handle = start_agent(&endpoint);
        let client = SessionClient::new(&endpoint).unwrap();
        wait_for_agent(&client);

        client
            .unlock_drive(temp.path(), initialized.workspace.id, "password".into())
            .unwrap();
        let id = NitApi::create(&client, Kind::Note, None, "On drive".into()).unwrap();
        assert_eq!(id.to_string(), "N-0001");
        client.shutdown_agent().unwrap();
        handle.join().unwrap();
        assert!(!temp.path().join(".nit").exists());
    }
}
