//! Ephemeral local IPC session for unlocked NIT Vaults.
//!
//! The agent owns the unlocked `Vault`/`Nit` values. Passwords cross IPC only
//! during unlock, are moved immediately into `SecretString`, and are never
//! logged or persisted. Domain operations will be added to this protocol in the
//! CLI integration phase; the lifecycle and transport are established here.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, bail, Context, Result};
use interprocess::local_socket::prelude::*;
use nit_core::{vault::Vault, Nit, VaultWorkspaceId};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

mod transport;

const PROTOCOL_VERSION: u16 = 1;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

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
    Lock {
        protocol: u16,
    },
    Shutdown {
        protocol: u16,
    },
}

impl Request {
    fn protocol(&self) -> u16 {
        match self {
            Self::Status { protocol }
            | Self::Unlock { protocol, .. }
            | Self::Lock { protocol }
            | Self::Shutdown { protocol } => *protocol,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Response {
    protocol: u16,
    status: SessionStatus,
    error: Option<String>,
}

struct UnlockedSession {
    _nit: Nit,
    vault_path: PathBuf,
    vault_id: String,
    workspace_id: String,
    workspace_name: String,
}

#[derive(Default)]
struct AgentState {
    unlocked: Option<UnlockedSession>,
    unavailable: bool,
}

impl AgentState {
    fn status(&mut self) -> SessionStatus {
        if self
            .unlocked
            .as_ref()
            .is_some_and(|session| !vault_still_available(&session.vault_path))
        {
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
        let workspace_id: VaultWorkspaceId = workspace_id.parse()?;
        let vault = Arc::new(Vault::open(vault_path, &password)?);
        drop(password);
        let vault_id = hex_encode(&vault.id());
        let nit = Nit::open_vault(vault.clone(), workspace_id)?;
        let info = nit
            .vault_workspace()?
            .ok_or_else(|| anyhow!("Vault workspace is unavailable"))?;
        self.unlocked = Some(UnlockedSession {
            _nit: nit,
            vault_path: vault.path().to_path_buf(),
            vault_id,
            workspace_id: info.id.to_string(),
            workspace_name: info.name,
        });
        self.unavailable = false;
        Ok(self.status())
    }

    fn lock(&mut self) -> SessionStatus {
        self.unlocked = None;
        self.unavailable = false;
        SessionStatus::Locked
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
            let request = match read_message::<Request>(&mut connection) {
                Ok(request) => request,
                Err(error) => {
                    write_response(
                        &mut connection,
                        Response {
                            protocol: PROTOCOL_VERSION,
                            status: state.status(),
                            error: Some(error.to_string()),
                        },
                    )?;
                    continue;
                }
            };
            let shutdown = matches!(request, Request::Shutdown { .. });
            let response = handle_request(&mut state, request);
            write_response(&mut connection, response)?;
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
}

impl Default for SessionClient {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
        }
    }
}

impl SessionClient {
    pub fn new(endpoint: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        validate_endpoint(&endpoint)?;
        Ok(Self { endpoint })
    }

    pub fn status(&self) -> Result<SessionStatus> {
        self.call(Request::Status {
            protocol: PROTOCOL_VERSION,
        })
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
        self.call(request)
    }

    pub fn lock(&self) -> Result<SessionStatus> {
        self.call(Request::Lock {
            protocol: PROTOCOL_VERSION,
        })
    }

    #[doc(hidden)]
    pub fn shutdown_agent(&self) -> Result<SessionStatus> {
        self.call(Request::Shutdown {
            protocol: PROTOCOL_VERSION,
        })
    }

    fn call(&self, request: Request) -> Result<SessionStatus> {
        let mut connection = transport::connect(&self.endpoint)?;
        write_message(&mut connection, &request)?;
        let response: Response = read_message(&mut connection)?;
        if response.protocol != PROTOCOL_VERSION {
            bail!("incompatible NIT Session Agent protocol");
        }
        if let Some(error) = response.error {
            bail!(error);
        }
        Ok(response.status)
    }
}

fn handle_request(state: &mut AgentState, mut request: Request) -> Response {
    if request.protocol() != PROTOCOL_VERSION {
        return Response {
            protocol: PROTOCOL_VERSION,
            status: state.status(),
            error: Some("incompatible NIT Session Agent protocol".into()),
        };
    }
    let result = match &mut request {
        Request::Status { .. } => Ok(state.status()),
        Request::Unlock {
            vault_path,
            workspace_id,
            password,
            ..
        } => {
            let secret = SecretString::from(std::mem::take(password));
            state.unlock(vault_path, workspace_id, secret)
        }
        Request::Lock { .. } | Request::Shutdown { .. } => Ok(state.lock()),
    };
    Response {
        protocol: PROTOCOL_VERSION,
        status: state.status(),
        error: result.err().map(|error| error.to_string()),
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

fn vault_still_available(path: &Path) -> bool {
    path.is_dir() && path.join("header").is_file() && path.join("objects").is_dir()
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

    use super::*;

    static NEXT_ENDPOINT: AtomicU64 = AtomicU64::new(1);

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
}
