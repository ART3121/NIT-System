use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{bail, Context, Result};

use nit_ai::{generate_roadmap, roadmap_text, GenerateOutcome};
use nit_core::{
    appears_ignored, capture_text, ensure_private, find_index, migrate, parse_capture_code,
    render_notes, text, EntryId, Horizon, Kind, Nit, NitApi, VaultWorkspaceId, View, Workspace,
    ACTIVE_TITLE, ARCHIVE_TITLE,
};
use nit_drive::{
    discover_devices, InitializedDrive, NitDrive, NitDriveInitializer, Provisioner, RemovableDevice,
};
use nit_editor as editor;
use nit_session::{
    default_endpoint, DriveUnlockOutcome, SessionAgent, SessionClient, SessionStatus,
    SessionWorkspace,
};
use nit_tui as tui;
use secrecy::SecretString;
use zeroize::{Zeroize, Zeroizing};

fn write_stdout(arguments: std::fmt::Arguments<'_>) -> Result<()> {
    match io::stdout().lock().write_fmt(arguments) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

macro_rules! print {
    ($($argument:tt)*) => {{
        write_stdout(format_args!($($argument)*))?;
    }};
}

macro_rules! println {
    () => {{
        write_stdout(format_args!("\n"))?;
    }};
    ($($argument:tt)*) => {{
        write_stdout(format_args!("{}\n", format_args!($($argument)*)))?;
    }};
}

#[derive(Debug, PartialEq, Eq)]
enum InitMode {
    Standard,
    Private,
    Tracked,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Tui,
    Capture(Vec<String>),
    Init(InitMode),
    Migrate,
    Root,
    Path,
    Status,
    AssignIds,
    MigrateTimeless,
    DriveCreate(DriveCreateMode),
    DriveMigrate {
        source: Option<PathBuf>,
    },
    Unlock {
        drive: Option<PathBuf>,
        workspace: Option<VaultWorkspaceId>,
    },
    Lock,
    SessionStatus,
    AiRoadmap(EntryId),
    Search {
        query: Vec<String>,
        classification: Option<(Kind, Option<Horizon>)>,
        archived: bool,
        all: bool,
    },
    List {
        classification: Option<(Kind, Option<Horizon>)>,
        archived: bool,
    },
    Show {
        query: Vec<String>,
        archived: bool,
    },
    Edit {
        query: Vec<String>,
        archived: bool,
    },
    Archive(Vec<String>),
    Import(PathBuf),
    Completions(CompletionShell),
    CompletionIds,
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
enum DriveCreateMode {
    Interactive { device_id: Option<String> },
    DryRun { device_id: String },
    Initialize { device_id: String, mount: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl CompletionShell {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }
}

pub fn run() -> Result<()> {
    let arguments = std::env::args().skip(1).collect();
    if arguments == ["--session-agent-internal"] {
        return SessionAgent::serve(&default_endpoint());
    }
    execute(parse_arguments(arguments)?)
}

fn parse_arguments(arguments: Vec<String>) -> Result<Action> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Ok(Action::Tui);
    };
    let remaining = &arguments[1..];

    match command {
        "-init" => parse_init(remaining),
        "-migrate" => no_arguments(remaining, Action::Migrate, "nit -migrate"),
        "-root" => no_arguments(remaining, Action::Root, "nit -root"),
        "-path" => no_arguments(remaining, Action::Path, "nit -path"),
        "-status" => no_arguments(remaining, Action::Status, "nit -status"),
        "-assign-ids" => no_arguments(remaining, Action::AssignIds, "nit -assign-ids"),
        "-migrate-timeless" => {
            no_arguments(remaining, Action::MigrateTimeless, "nit -migrate-timeless")
        }
        "-drive-create" => parse_drive_create(remaining),
        "-drive-migrate" => match remaining {
            [] => Ok(Action::DriveMigrate { source: None }),
            [source] => Ok(Action::DriveMigrate {
                source: Some(PathBuf::from(source)),
            }),
            _ => bail!("usage: nit -drive-migrate [plain-workspace-path]"),
        },
        "-unlock" => parse_unlock(remaining),
        "-lock" => no_arguments(remaining, Action::Lock, "nit -lock"),
        "-session-status" => no_arguments(remaining, Action::SessionStatus, "nit -session-status"),
        "-v" => bail!("`nit -v` was removed in 0.4.0; use `nitcat <NOTE-ID>`"),
        "-ai-roadmap" => match remaining {
            [id] => EntryId::parse(id)
                .map(Action::AiRoadmap)
                .ok_or_else(|| anyhow::anyhow!("usage: nit -ai-roadmap <ID>")),
            _ => bail!("usage: nit -ai-roadmap <ID>"),
        },
        "-search" => parse_search(remaining),
        "-ls" => parse_list(remaining),
        "-show" => {
            let (query, archived) = query_options(remaining)?;
            Ok(Action::Show { query, archived })
        }
        "-edit" => {
            let (query, archived) = query_options(remaining)?;
            Ok(Action::Edit { query, archived })
        }
        "-archive" => Ok(Action::Archive(remaining.to_vec())),
        "-import" => match remaining {
            [path] => Ok(Action::Import(PathBuf::from(path))),
            _ => bail!("usage: nit -import <path>"),
        },
        "-completions" => match remaining {
            [shell] => CompletionShell::parse(shell)
                .map(Action::Completions)
                .ok_or_else(|| anyhow::anyhow!("usage: nit -completions <bash|zsh|fish>")),
            _ => bail!("usage: nit -completions <bash|zsh|fish>"),
        },
        "-completion-ids" => no_arguments(remaining, Action::CompletionIds, "nit -completion-ids"),
        "-tui" => no_arguments(remaining, Action::Tui, "nit -tui"),
        "-help" | "--help" | "-h" => Ok(Action::Help),
        "-version" | "--version" | "-V" => Ok(Action::Version),
        value if value.starts_with('-') => {
            bail!("unknown command '{value}'; run 'nit -help' for usage")
        }
        _ => Ok(Action::Capture(arguments)),
    }
}

fn parse_unlock(arguments: &[String]) -> Result<Action> {
    match arguments {
        [] => Ok(Action::Unlock {
            drive: None,
            workspace: None,
        }),
        [drive] => Ok(Action::Unlock {
            drive: Some(PathBuf::from(drive)),
            workspace: None,
        }),
        [drive, workspace] => Ok(Action::Unlock {
            drive: Some(PathBuf::from(drive)),
            workspace: Some(workspace.parse()?),
        }),
        _ => bail!("usage: nit -unlock [drive-path [workspace-id]]"),
    }
}

fn parse_drive_create(arguments: &[String]) -> Result<Action> {
    let mode = match arguments {
        [] => DriveCreateMode::Interactive { device_id: None },
        [device_id] if !device_id.starts_with('-') => DriveCreateMode::Interactive {
            device_id: Some(device_id.clone()),
        },
        [option, device_id] if option == "--dry-run" && !device_id.starts_with('-') => {
            DriveCreateMode::DryRun {
                device_id: device_id.clone(),
            }
        }
        [option, device_id, mount]
            if option == "--initialize" && !device_id.starts_with('-') =>
        {
            DriveCreateMode::Initialize {
                device_id: device_id.clone(),
                mount: PathBuf::from(mount),
            }
        }
        _ => bail!(
            "usage: nit -drive-create [<device-id> | --dry-run <device-id> | --initialize <device-id> <mount-path>]"
        ),
    };
    Ok(Action::DriveCreate(mode))
}

fn parse_init(arguments: &[String]) -> Result<Action> {
    match arguments {
        [] => Ok(Action::Init(InitMode::Standard)),
        [option] if option == "--private" => Ok(Action::Init(InitMode::Private)),
        [option] if option == "--tracked" => Ok(Action::Init(InitMode::Tracked)),
        _ => bail!("usage: nit -init [--private|--tracked]"),
    }
}

fn no_arguments(arguments: &[String], action: Action, usage: &str) -> Result<Action> {
    if !arguments.is_empty() {
        bail!("usage: {usage}");
    }
    Ok(action)
}

fn parse_list(arguments: &[String]) -> Result<Action> {
    let mut classification = None;
    let mut archived = false;

    for argument in arguments {
        if argument == "--archived" {
            if archived {
                bail!("--archived was provided more than once");
            }
            archived = true;
        } else if let Some(value) = parse_capture_code(argument) {
            if classification.replace(value).is_some() {
                bail!("provide only one classification code to -ls");
            }
        } else {
            bail!("unknown -ls argument '{argument}'; use a code such as -n or --archived");
        }
    }

    Ok(Action::List {
        classification,
        archived,
    })
}

fn query_options(arguments: &[String]) -> Result<(Vec<String>, bool)> {
    let mut query = Vec::new();
    let mut archived = false;
    for argument in arguments {
        if argument == "--archived" {
            if archived {
                bail!("--archived was provided more than once");
            }
            archived = true;
        } else {
            query.push(argument.clone());
        }
    }
    Ok((query, archived))
}

fn parse_search(arguments: &[String]) -> Result<Action> {
    let mut query = Vec::new();
    let mut classification = None;
    let mut archived = false;
    let mut all = false;
    for argument in arguments {
        if argument == "--archived" {
            archived = true;
        } else if argument == "--all" {
            all = true;
        } else if let Some(value) = parse_capture_code(argument) {
            if classification.replace(value).is_some() {
                bail!("provide only one classification code to -search");
            }
        } else {
            query.push(argument.clone());
        }
    }
    if archived && all {
        bail!("--archived and --all cannot be combined");
    }
    if query.is_empty() {
        bail!("usage: nit -search <query> [code] [--archived|--all]");
    }
    Ok(Action::Search {
        query,
        classification,
        archived,
        all,
    })
}

fn apply_edited_text(entry: &mut nit_core::Entry, edited: &str) -> Result<()> {
    if entry.kind != Kind::Note {
        let value = edited.trim().to_owned();
        if value.is_empty() {
            bail!("entry text cannot be empty");
        }
        entry.text = value;
        return Ok(());
    }
    let normalized = edited.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    let title = lines
        .next()
        .and_then(|line| line.strip_prefix("# "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Note must start with '# <title>'"))?;
    entry.text = title.to_owned();
    entry.body = lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_owned();
    Ok(())
}

fn execute(action: Action) -> Result<()> {
    match action {
        Action::Init(mode) => initialize(mode),
        Action::Migrate => {
            let workspace = migrate(&std::env::current_dir()?)?;
            println!(
                "Migrated legacy NIT workspace to {}.",
                workspace.nit_dir().display()
            );
            Ok(())
        }
        Action::Help => {
            print_help()?;
            Ok(())
        }
        Action::Version => {
            println!("nit {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Action::Completions(shell) => {
            print!("{}", completion_script(shell));
            Ok(())
        }
        Action::AssignIds => {
            ensure_plain_maintenance_allowed()?;
            let workspace = Workspace::discover()?;
            let assigned = Nit::assign_missing_ids_in(&workspace)?;
            if assigned == 0 {
                println!("All entries already have IDs.");
            } else {
                println!("Assigned IDs to {assigned} entries.");
                println!("Backups: notes.pre-ids.bak, archive.pre-ids.bak");
            }
            Ok(())
        }
        Action::MigrateTimeless => {
            ensure_plain_maintenance_allowed()?;
            let workspace = Workspace::discover()?;
            let migrated = Nit::migrate_timeless_ids_in(&workspace)?;
            if migrated == 0 {
                println!("All Note and Item IDs are already timeless.");
            } else {
                println!("Migrated {migrated} Note/Item IDs to timeless IDs.");
                println!(
                    "Backups: notes.pre-timeless.bak, archive.pre-timeless.bak, next-ids.pre-timeless.bak"
                );
            }
            Ok(())
        }
        Action::DriveCreate(mode) => create_nit_drive(mode),
        Action::DriveMigrate { source } => migrate_plain_workspace_to_drive(source),
        Action::Unlock { drive, workspace } => unlock_nit_drive(drive, workspace),
        Action::Lock => {
            let client = SessionClient::default();
            match client.lock() {
                Ok(status) => print_session_status(&status),
                Err(_) => {
                    println!("NIT Vault is locked.");
                    Ok(())
                }
            }
        }
        Action::SessionStatus => {
            let client = SessionClient::default();
            match client.status() {
                Ok(status) => print_session_status(&status),
                Err(_) => {
                    println!("NIT Session Agent is not running.");
                    Ok(())
                }
            }
        }
        action => {
            let client = SessionClient::default();
            match storage_preference(client.status()) {
                StoragePreference::Vault(status) => {
                    execute_in_workspace(action, &client, StorageContext::Vault(status))
                }
                StoragePreference::Unavailable => {
                    bail!("NIT Drive is unavailable; reconnect and run `nit -unlock` again")
                }
                StoragePreference::Plain => {
                    let nit = Nit::discover()?;
                    let workspace = nit
                        .workspace()
                        .expect("a discovered NIT instance uses Plain Storage");
                    execute_in_workspace(action, &nit, StorageContext::Plain(workspace))
                }
            }
        }
    }
}

fn create_nit_drive(mode: DriveCreateMode) -> Result<()> {
    match mode {
        DriveCreateMode::DryRun { device_id } => {
            let plan = Provisioner::default().dry_run(&device_id)?;
            print_provisioning_plan(&plan)
        }
        DriveCreateMode::Initialize { device_id, mount } => {
            require_interactive_terminal()?;
            let device = Provisioner::default().dry_run(&device_id)?.device;
            print_discovered_devices(std::slice::from_ref(&device))?;
            println!(
                "\nInitialize mounted removable device\n\nDevice: {device_id}\nMount:  {}",
                mount.display()
            );
            let workspace_name = prompt_workspace_name()?;
            let password = prompt_new_vault_password()?;
            let expected = format!(
                "CREATE {device_id} {} {} {}",
                device.model,
                device.capacity_bytes,
                mount.display()
            );
            let confirmation = prompt_line(&format!(
                "Type exactly to create the Vault:\n{expected}\n> "
            ))?;
            if confirmation != expected {
                bail!("initialization confirmation does not match; nothing was changed");
            }
            let initialized = NitDriveInitializer::default().initialize(
                &device_id,
                &mount,
                &password,
                workspace_name,
            )?;
            print_initialized_drive(&initialized)
        }
        DriveCreateMode::Interactive { device_id } => {
            require_interactive_terminal()?;
            let device_id = match device_id {
                Some(device_id) => device_id,
                None => {
                    print_discovered_devices(&discover_devices()?)?;
                    let selected =
                        prompt_line("\nType the exact device ID to prepare (empty cancels): ")?;
                    if selected.is_empty() {
                        println!("Cancelled; nothing was changed.");
                        return Ok(());
                    }
                    selected
                }
            };
            let provisioner = Provisioner::default();
            let plan = provisioner.dry_run(&device_id)?;
            print_provisioning_plan(&plan)?;
            println!(
                "\nWARNING: every partition and every byte currently stored on this device will be erased."
            );
            let workspace_name = prompt_workspace_name()?;
            let password = prompt_new_vault_password()?;
            let confirmation = prompt_line(&format!(
                "Type the following line exactly to continue:\n{}\n> ",
                plan.confirmation
            ))?;
            if confirmation != plan.confirmation {
                bail!("destructive confirmation does not match; nothing was changed");
            }
            let verified = provisioner
                .execute(&device_id, &confirmation)
                .with_context(|| {
                    format!(
                        "NIT Drive provisioning did not complete; the device may be partially modified. Re-discover it before retrying. If it is already formatted and mounted, use `nit -drive-create --initialize {device_id} <mount-path>`"
                    )
                })?;
            let mount = unique_mount_point(&verified).with_context(|| {
                format!(
                    "device was formatted but no unique mount point is available; mount NIT_DRIVE and run `nit -drive-create --initialize {device_id} <mount-path>`"
                )
            })?;
            let initialized = NitDriveInitializer::default().initialize(
                &device_id,
                &mount,
                &password,
                workspace_name,
            )?;
            print_initialized_drive(&initialized)
        }
    }
}

fn migrate_plain_workspace_to_drive(source: Option<PathBuf>) -> Result<()> {
    let source = match source {
        Some(source) => source,
        None => std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            anyhow::anyhow!("home directory is unavailable; provide the Plain workspace path")
        })?,
    };
    let workspace = Workspace::discover_from(&source).with_context(|| {
        format!(
            "no Plain NIT workspace could be discovered from {}",
            source.display()
        )
    })?;
    let source = Nit::open(&workspace)?;
    let target = SessionClient::default();
    match target.status() {
        Ok(SessionStatus::Unlocked { .. }) => {}
        Ok(SessionStatus::Unavailable) => {
            bail!("NIT Drive is unavailable; reconnect it and run the migration again")
        }
        Ok(SessionStatus::Locked) | Err(_) => unlock_nit_drive(None, None)?,
    }
    let (active, archived) = copy_workspace(&source, &target)?;
    println!(
        "Migration complete.\nSource preserved: {}\nActive entries: {active}\nArchived entries: {archived}",
        workspace.nit_dir().display()
    );
    Ok(())
}

fn copy_workspace(source: &dyn NitApi, target: &dyn NitApi) -> Result<(usize, usize)> {
    let (target_active, target_archived) = target.all()?;
    let (active, archived) = source.all()?;
    if active.entries.is_empty() && archived.entries.is_empty() {
        bail!("source Plain workspace has no entries to migrate");
    }
    if !target_active.entries.is_empty() || !target_archived.entries.is_empty() {
        if target_active == active && target_archived == archived {
            return Ok((active.entries.len(), archived.entries.len()));
        }
        bail!("target Vault workspace is not empty; refusing to merge or overwrite entries");
    }
    target.save_all(&active, &archived)?;
    let (verified_active, verified_archived) = target.all()?;
    if verified_active != active || verified_archived != archived {
        bail!("Vault verification failed after migration");
    }
    Ok((active.entries.len(), archived.entries.len()))
}

#[derive(Debug)]
struct MountedDrive {
    root: PathBuf,
    model: String,
    capacity_bytes: u64,
}

fn unlock_nit_drive(drive: Option<PathBuf>, workspace: Option<VaultWorkspaceId>) -> Result<()> {
    let drive = match drive {
        Some(drive) => drive,
        None => select_mounted_drive(discover_mounted_drives()?)?,
    };
    let client = ensure_session_agent()?;
    let mut password = Zeroizing::new(rpassword::prompt_password("Vault password: ")?);
    if let Some(workspace) = workspace {
        let status = client.unlock_drive(drive, workspace, std::mem::take(&mut *password))?;
        return print_session_status(&status);
    }
    match client.unlock_drive_automatic(&drive, password.to_string())? {
        DriveUnlockOutcome::Unlocked(status) => print_session_status(&status),
        DriveUnlockOutcome::SelectWorkspace(workspaces) => {
            let workspace = select_workspace(&workspaces)?;
            let status =
                client.unlock_drive(drive, workspace.id, std::mem::take(&mut *password))?;
            print_session_status(&status)
        }
    }
}

fn discover_mounted_drives() -> Result<Vec<MountedDrive>> {
    let mut drives = Vec::new();
    for device in discover_devices()? {
        if !device.removable || device.system_disk || device.read_only || device.is_ambiguous() {
            continue;
        }
        for mount in &device.mount_points {
            let Ok(drive) = NitDrive::open(mount) else {
                continue;
            };
            if drives
                .iter()
                .any(|candidate: &MountedDrive| candidate.root == drive.root())
            {
                continue;
            }
            drives.push(MountedDrive {
                root: drive.root().to_path_buf(),
                model: device.model.clone(),
                capacity_bytes: device.capacity_bytes,
            });
        }
    }
    Ok(drives)
}

fn select_mounted_drive(mut drives: Vec<MountedDrive>) -> Result<PathBuf> {
    match drives.len() {
        0 => bail!("no mounted NIT Drive found; connect and mount the device, then retry"),
        1 => Ok(drives.remove(0).root),
        _ => {
            require_interactive_terminal()?;
            println!("Mounted NIT Drives:\n");
            for (index, drive) in drives.iter().enumerate() {
                println!(
                    "  {}. {} · {}",
                    index + 1,
                    drive.model,
                    human_capacity(drive.capacity_bytes)
                );
            }
            let selected = prompt_selection("Select a Drive", drives.len())?;
            Ok(drives.remove(selected).root)
        }
    }
}

fn select_workspace(workspaces: &[SessionWorkspace]) -> Result<&SessionWorkspace> {
    require_interactive_terminal()?;
    println!("This NIT Drive contains multiple workspaces:\n");
    for (index, workspace) in workspaces.iter().enumerate() {
        println!("  {}. {}", index + 1, workspace.name);
    }
    let selected = prompt_selection("Select a workspace", workspaces.len())?;
    Ok(&workspaces[selected])
}

fn prompt_selection(label: &str, count: usize) -> Result<usize> {
    let value = prompt_line(&format!("\n{label} [1-{count}]: "))?;
    let selected = value
        .parse::<usize>()
        .ok()
        .filter(|selected| (1..=count).contains(selected))
        .ok_or_else(|| anyhow::anyhow!("invalid selection"))?;
    Ok(selected - 1)
}

fn print_discovered_devices(devices: &[RemovableDevice]) -> Result<()> {
    println!("Discovered physical devices:\n");
    if devices.is_empty() {
        println!("No physical devices were discovered.");
        return Ok(());
    }
    for device in devices {
        let mounts = if device.mount_points.is_empty() {
            "not mounted".to_owned()
        } else {
            device
                .mount_points
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "{}\n  model: {}\n  size:  {} ({} bytes)\n  mount: {}\n  state: {}",
            device.id,
            device.model,
            human_capacity(device.capacity_bytes),
            device.capacity_bytes,
            mounts,
            device_safety_label(device)
        );
    }
    Ok(())
}

fn print_provisioning_plan(plan: &nit_drive::ProvisioningPlan) -> Result<()> {
    print_discovered_devices(std::slice::from_ref(&plan.device))?;
    println!("\nDry-run provisioning plan:");
    for (index, operation) in plan.operations.iter().enumerate() {
        let kind = if operation.destructive {
            "DESTRUCTIVE"
        } else {
            "support"
        };
        println!(
            "  {}. [{kind}] {} {:?}",
            index + 1,
            operation.program,
            operation.arguments
        );
    }
    println!("\nRequired confirmation:\n{}", plan.confirmation);
    Ok(())
}

fn print_initialized_drive(initialized: &InitializedDrive) -> Result<()> {
    println!(
        "\nNIT Drive ready.\nDrive: {}\nMount: {}\nWorkspace: {} ({})\n\nUnlock it with:\nnit -unlock",
        initialized.drive.id(),
        initialized.drive.root().display(),
        initialized.workspace.name,
        initialized.workspace.id
    );
    Ok(())
}

fn require_interactive_terminal() -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("NIT Drive creation requires an interactive terminal; use --dry-run for read-only inspection");
    }
    Ok(())
}

fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}

fn prompt_workspace_name() -> Result<String> {
    let name = prompt_line("Initial workspace name: ")?;
    if name.is_empty() {
        bail!("workspace name cannot be empty");
    }
    Ok(name)
}

fn prompt_new_vault_password() -> Result<SecretString> {
    let mut password = rpassword::prompt_password("Vault password: ")?;
    let mut repeated = rpassword::prompt_password("Confirm Vault password: ")?;
    if password.is_empty() {
        password.zeroize();
        repeated.zeroize();
        bail!("Vault password cannot be empty");
    }
    if password != repeated {
        password.zeroize();
        repeated.zeroize();
        bail!("Vault passwords do not match");
    }
    repeated.zeroize();
    Ok(SecretString::from(password))
}

fn unique_mount_point(device: &RemovableDevice) -> Result<PathBuf> {
    match device.mount_points.as_slice() {
        [mount] => Ok(mount.clone()),
        [] => bail!("the formatted device is not mounted"),
        _ => bail!("the formatted device has multiple mount points; refusing an ambiguous target"),
    }
}

fn human_capacity(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / GIB)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB)
    }
}

fn device_safety_label(device: &RemovableDevice) -> &'static str {
    if device.system_disk {
        "REJECTED: system/root/boot disk"
    } else if !device.removable {
        "REJECTED: fixed/internal disk"
    } else if device.read_only {
        "REJECTED: read-only"
    } else if device.is_ambiguous() {
        "REJECTED: ambiguous metadata"
    } else if device.capacity_bytes < 64 * 1024 * 1024 {
        "REJECTED: too small"
    } else {
        "eligible removable device"
    }
}

#[derive(Debug, PartialEq, Eq)]
enum StoragePreference {
    Plain,
    Vault(SessionStatus),
    Unavailable,
}

fn storage_preference(status: Result<SessionStatus>) -> StoragePreference {
    match status {
        Ok(status @ SessionStatus::Unlocked { .. }) => StoragePreference::Vault(status),
        Ok(SessionStatus::Unavailable) => StoragePreference::Unavailable,
        Ok(SessionStatus::Locked) | Err(_) => StoragePreference::Plain,
    }
}

fn ensure_plain_maintenance_allowed() -> Result<()> {
    match storage_preference(SessionClient::default().status()) {
        StoragePreference::Plain => Ok(()),
        StoragePreference::Vault(_) => {
            bail!("this maintenance command is only available for Plain Storage; run `nit -lock` first")
        }
        StoragePreference::Unavailable => {
            bail!("NIT Drive is unavailable; reconnect and run `nit -unlock` again")
        }
    }
}

fn ensure_session_agent() -> Result<SessionClient> {
    let client = SessionClient::default();
    if client.status().is_ok() {
        return Ok(client);
    }
    let executable = std::env::current_exe()?;
    Command::new(executable)
        .arg("--session-agent-internal")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start NIT Session Agent")?;
    for _ in 0..100 {
        if client.status().is_ok() {
            return Ok(client);
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("NIT Session Agent did not start")
}

fn print_session_status(status: &SessionStatus) -> Result<()> {
    match status {
        SessionStatus::Locked => println!("NIT Vault is locked."),
        SessionStatus::Unavailable => {
            println!("NIT Drive is unavailable and must be unlocked again.")
        }
        SessionStatus::Unlocked {
            vault_id,
            workspace_id,
            workspace_name,
        } => println!(
            "NIT Vault unlocked.\nVault: {vault_id}\nWorkspace: {workspace_name} ({workspace_id})"
        ),
    }
    Ok(())
}

enum StorageContext<'a> {
    Plain(&'a Workspace),
    Vault(SessionStatus),
}

fn initialize(mode: InitMode) -> Result<()> {
    let result = Workspace::init(&std::env::current_dir()?)?;
    let tracked_warning = mode == InitMode::Tracked && appears_ignored(result.workspace.root())?;
    match mode {
        InitMode::Private => ensure_private(result.workspace.root())?,
        InitMode::Tracked => {}
        InitMode::Standard => {}
    }
    if result.already_existed {
        println!(
            "NIT workspace already exists at {}",
            result.workspace.nit_dir().display()
        );
    } else {
        println!(
            "Initialized NIT workspace at {}",
            result.workspace.nit_dir().display()
        );
    }
    if tracked_warning {
        println!("Workspace initialized, but `.nit/` appears to be ignored by .gitignore.");
    }
    Ok(())
}

fn execute_in_workspace(
    action: Action,
    nit: &dyn NitApi,
    storage: StorageContext<'_>,
) -> Result<()> {
    match action {
        Action::Tui => tui::run(nit)?,
        Action::Capture(message) => {
            let (kind, horizon, value) = capture_text(message)?;
            let id = nit.create(kind, horizon, value)?;
            println!("Added {id} ({}).", classification_label(kind, horizon));
        }
        Action::Root => match &storage {
            StorageContext::Plain(workspace) => println!("{}", workspace.root().display()),
            StorageContext::Vault(_) => bail!("Vault workspaces have no host project root"),
        },
        Action::Path => match &storage {
            StorageContext::Plain(workspace) => println!("{}", workspace.nit_dir().display()),
            StorageContext::Vault(_) => bail!("Vault storage paths are intentionally opaque"),
        },
        Action::Status => {
            let status = nit.status()?;
            match &storage {
                StorageContext::Plain(workspace) => println!(
                    "NIT Workspace\n\nRoot: {}\nStorage: {}\nActive entries: {}\nArchived entries: {}",
                    workspace.root().display(),
                    workspace.nit_dir().display(),
                    status.active_entries,
                    status.archived_entries
                ),
                StorageContext::Vault(SessionStatus::Unlocked {
                    vault_id,
                    workspace_id,
                    workspace_name,
                }) => println!(
                    "NIT Vault Workspace\n\nVault: {vault_id}\nWorkspace: {workspace_name} ({workspace_id})\nActive entries: {}\nArchived entries: {}",
                    status.active_entries,
                    status.archived_entries
                ),
                StorageContext::Vault(_) => unreachable!("only unlocked sessions execute commands"),
            }
        }
        Action::AiRoadmap(id) => execute_ai_roadmap(nit, id)?,
        Action::Search {
            query,
            classification,
            archived,
            all,
        } => {
            let query = text(query)?;
            let views = if all {
                vec![View::Active, View::Archived]
            } else if archived {
                vec![View::Archived]
            } else {
                vec![View::Active]
            };
            for (view, entry) in nit.search(&query, &views, classification)? {
                println!(
                    "{} · {} · {}{}",
                    entry
                        .id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "—".into()),
                    entry.classification(),
                    entry.text,
                    if view == View::Archived {
                        " · archived"
                    } else {
                        ""
                    }
                );
            }
        }
        Action::List {
            classification,
            archived,
        } => {
            let mut notes = nit.load(if archived {
                View::Archived
            } else {
                View::Active
            })?;
            for entry in &mut notes.entries {
                if entry.kind == Kind::Note {
                    entry.body.clear();
                    entry.roadmap = None;
                }
            }
            let (kind, horizon) = classification
                .map(|(kind, horizon)| (Some(kind), horizon))
                .unwrap_or((None, None));
            print!(
                "{}",
                render_notes(
                    &notes,
                    kind,
                    horizon,
                    if archived {
                        ARCHIVE_TITLE
                    } else {
                        ACTIVE_TITLE
                    }
                )
            );
        }
        Action::Show { query, archived } => {
            let notes = nit.load(if archived {
                View::Archived
            } else {
                View::Active
            })?;
            let query = text(query)?;
            let entry = &notes.entries[find_index(&notes, &query)?];
            println!(
                "{}\n{}\n\n{}",
                entry
                    .id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "No ID".into()),
                entry.classification(),
                entry.display_text()
            );
            if let Some(roadmap) = &entry.roadmap {
                println!("\nRoadmap\n\n{}", roadmap_text(roadmap));
            }
        }
        Action::Edit { query, archived } => {
            if matches!(storage, StorageContext::Vault(_)) {
                bail!("external editing is disabled for Vault Storage to avoid plaintext temporary files");
            }
            let view = if archived {
                View::Archived
            } else {
                View::Active
            };
            let mut notes = nit.load(view)?;
            let query = text(query)?;
            let index = find_index(&notes, &query)?;
            let edited = if notes.entries[index].kind == Kind::Note {
                editor::open(&format!(
                    "# {}\n\n{}",
                    notes.entries[index].text, notes.entries[index].body
                ))?
            } else {
                editor::open(&notes.entries[index].text)?
            };
            apply_edited_text(&mut notes.entries[index], &edited)?;
            nit.save(view, &notes)?;
            println!("Updated.");
        }
        Action::Archive(query) => {
            nit.archive(&text(query)?)?;
            println!("Archived.");
        }
        Action::Import(source) => {
            let source = if source.is_absolute() {
                source
            } else {
                std::env::current_dir()?.join(source)
            };
            let imported = nit.import(&source)?;
            println!("Imported {imported} entries.");
        }
        Action::CompletionIds => {
            let (active, archived) = nit.all()?;
            for id in active
                .entries
                .iter()
                .chain(&archived.entries)
                .filter_map(|entry| entry.id)
            {
                println!("{id}");
            }
        }
        Action::Init(_)
        | Action::Migrate
        | Action::AssignIds
        | Action::MigrateTimeless
        | Action::DriveCreate(_)
        | Action::DriveMigrate { .. }
        | Action::Unlock { .. }
        | Action::Lock
        | Action::SessionStatus
        | Action::Completions(_)
        | Action::Help
        | Action::Version => {
            unreachable!("non-workspace action reached workspace dispatcher")
        }
    }
    Ok(())
}

fn execute_ai_roadmap(nit: &dyn NitApi, id: EntryId) -> Result<()> {
    let entry = nit.roadmap_target(id)?;
    println!("Preparing local Ollama and generating a Roadmap for {id}…");
    io::stdout().flush()?;
    let roadmap = match generate_roadmap(&entry, false)? {
        GenerateOutcome::Ready(roadmap) => roadmap,
        GenerateOutcome::NeedsPull(model) => {
            let size = if model == "qwen3:1.7b" {
                " (approximately 1.4 GB)"
            } else {
                ""
            };
            if !confirm(&format!(
                "Model {model} is not installed{size}. Download it?"
            ))? {
                println!("Model was not downloaded. Run `ollama pull {model}` when ready.");
                return Ok(());
            }
            println!("Downloading {model} and generating the Roadmap…");
            io::stdout().flush()?;
            match generate_roadmap(&entry, true)? {
                GenerateOutcome::Ready(roadmap) => roadmap,
                GenerateOutcome::NeedsPull(_) => unreachable!("pull was explicitly allowed"),
            }
        }
    };
    println!("\nRoadmap for {id}\n\n{}\n", roadmap_text(&roadmap));
    if confirm(&format!("Attach this Roadmap to {id}?"))? {
        nit.attach_roadmap(&entry, roadmap)?;
        println!("Roadmap attached to {id}.");
    } else {
        println!("Roadmap discarded; no files were changed.");
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

fn classification_label(kind: Kind, horizon: Option<Horizon>) -> String {
    horizon
        .map(|value| format!("{value}/{kind}"))
        .unwrap_or_else(|| kind.to_string())
}

fn print_help() -> Result<()> {
    println!(
        r#"NIT System terminal notes

Usage:
  nit                                      Open the TUI
  nit <text> -<code>                       Capture an entry
  nit -init [--private|--tracked]          Create a workspace
  nit -migrate                             Migrate legacy files
  nit -assign-ids                          Assign IDs to existing entries
  nit -migrate-timeless                    Convert timed Note/Item IDs safely
  nit -drive-create                        Interactively create a NIT Drive
  nit -drive-create --dry-run <device-id>  Preview without changing the device
  nit -drive-create <device-id>            Prepare one explicitly named device
  nit -drive-create --initialize <device-id> <mount-path>
                                           Finish a formatted, mounted device
  nit -drive-migrate [plain-path]          Copy Plain storage into an empty Drive
  nit -unlock                              Find and unlock a mounted NIT Drive
  nit -unlock <drive-path> [workspace-id]  Advanced explicit unlock
  nit -lock                                Lock the active Vault session
  nit -session-status                      Show Vault session state
  nit -ai-roadmap <ID>                     Generate a local AI Roadmap
  nit -root                                Print the workspace root
  nit -path                                Print the .nit directory
  nit -status                              Show workspace statistics
  nit -search <text> [code] [--archived|--all]
                                           Search titles, bodies, IDs, and Roadmaps
  nit -ls [code] [--archived]              List entries (Notes: ID and title only)
  nit -show <text> [--archived]            Show one entry
  nit -edit <text> [--archived]            Edit one entry
  nit -archive <text>                      Archive one entry
  nit -import <path>                       Import entries
  nit -completions <bash|zsh|fish>         Generate shell completions
  nit -tui                                 Open the TUI explicitly
  nit -help                                Show this help
  nit -version                             Show the version

Capture codes:
  Ideas:     -si short  -mi medium  -li long
  To-dos:   -st short  -mt medium  -lt long
  Timeless: -n note    -x item"#
    );
    Ok(())
}

fn completion_script(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => include_str!("../../../completions/bash/nit"),
        CompletionShell::Zsh => include_str!("../../../completions/zsh/_nit"),
        CompletionShell::Fish => include_str!("../../../completions/fish/nit.fish"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_words_are_always_capture_text() {
        assert_eq!(
            parse_arguments(words(&["list", "of", "project", "ideas", "-n"])).unwrap(),
            Action::Capture(words(&["list", "of", "project", "ideas", "-n"]))
        );
    }

    #[test]
    fn commands_require_the_dash_prefix() {
        assert_eq!(
            parse_arguments(words(&["-show", "project", "ideas"])).unwrap(),
            Action::Show {
                query: words(&["project", "ideas"]),
                archived: false,
            }
        );
        assert_eq!(
            parse_arguments(words(&["-init", "--private"])).unwrap(),
            Action::Init(InitMode::Private)
        );
    }

    #[test]
    fn init_modes_are_mutually_exclusive() {
        assert_eq!(
            parse_arguments(words(&["-init", "--tracked"])).unwrap(),
            Action::Init(InitMode::Tracked)
        );
        assert!(parse_arguments(words(&["-init", "--private", "--tracked"])).is_err());
    }

    #[test]
    fn assign_ids_is_an_explicit_command() {
        assert_eq!(
            parse_arguments(words(&["-assign-ids"])).unwrap(),
            Action::AssignIds
        );
        assert!(parse_arguments(words(&["-assign-ids", "extra"])).is_err());
    }

    #[test]
    fn ai_roadmap_requires_one_entry_id() {
        assert_eq!(
            parse_arguments(words(&["-ai-roadmap", "li-1"])).unwrap(),
            Action::AiRoadmap(EntryId::parse("LI-0001").unwrap())
        );
        assert!(parse_arguments(words(&["-ai-roadmap"])).is_err());
        assert!(parse_arguments(words(&["-ai-roadmap", "not-an-id"])).is_err());
    }

    #[test]
    fn vault_session_commands_follow_the_existing_single_dash_parser() {
        let workspace = "00112233445566778899aabbccddeeff"
            .parse::<VaultWorkspaceId>()
            .unwrap();
        assert_eq!(
            parse_arguments(words(&[
                "-unlock",
                "/media/user/NIT/vault",
                "00112233445566778899aabbccddeeff",
            ]))
            .unwrap(),
            Action::Unlock {
                drive: Some(PathBuf::from("/media/user/NIT/vault")),
                workspace: Some(workspace),
            }
        );
        assert_eq!(
            parse_arguments(words(&["-unlock"])).unwrap(),
            Action::Unlock {
                drive: None,
                workspace: None,
            }
        );
        assert_eq!(
            parse_arguments(words(&["-unlock", "/media/user/NIT"])).unwrap(),
            Action::Unlock {
                drive: Some(PathBuf::from("/media/user/NIT")),
                workspace: None,
            }
        );
        assert_eq!(parse_arguments(words(&["-lock"])).unwrap(), Action::Lock);
        assert_eq!(
            parse_arguments(words(&["-session-status"])).unwrap(),
            Action::SessionStatus
        );
    }

    #[test]
    fn drive_create_parser_separates_preview_format_and_recovery() {
        assert_eq!(
            parse_arguments(words(&["-drive-create"])).unwrap(),
            Action::DriveCreate(DriveCreateMode::Interactive { device_id: None })
        );
        assert_eq!(
            parse_arguments(words(&["-drive-create", "/dev/sdb"])).unwrap(),
            Action::DriveCreate(DriveCreateMode::Interactive {
                device_id: Some("/dev/sdb".into()),
            })
        );
        assert_eq!(
            parse_arguments(words(&["-drive-create", "--dry-run", "/dev/sdb",])).unwrap(),
            Action::DriveCreate(DriveCreateMode::DryRun {
                device_id: "/dev/sdb".into(),
            })
        );
        assert_eq!(
            parse_arguments(words(&[
                "-drive-create",
                "--initialize",
                "/dev/sdb",
                "/media/NIT_DRIVE",
            ]))
            .unwrap(),
            Action::DriveCreate(DriveCreateMode::Initialize {
                device_id: "/dev/sdb".into(),
                mount: PathBuf::from("/media/NIT_DRIVE"),
            })
        );
        assert!(parse_arguments(words(&["-drive-create", "--dry-run"])).is_err());
        assert!(parse_arguments(words(&["-drive-create", "--initialize", "/dev/sdb",])).is_err());
    }

    #[test]
    fn drive_migration_defaults_to_home_and_accepts_an_explicit_source() {
        assert_eq!(
            parse_arguments(words(&["-drive-migrate"])).unwrap(),
            Action::DriveMigrate { source: None }
        );
        assert_eq!(
            parse_arguments(words(&["-drive-migrate", "/home/user"])).unwrap(),
            Action::DriveMigrate {
                source: Some(PathBuf::from("/home/user")),
            }
        );
        assert!(parse_arguments(words(&["-drive-migrate", "/home/user", "extra",])).is_err());
    }

    #[test]
    fn workspace_migration_preserves_active_archive_and_refuses_nonempty_target() {
        let temp = tempfile::tempdir().unwrap();
        let source_root = temp.path().join("source");
        std::fs::create_dir_all(&source_root).unwrap();
        let source_workspace = Workspace::init(&source_root).unwrap().workspace;
        let source = Nit::open(&source_workspace).unwrap();
        let vault = std::sync::Arc::new(
            nit_core::vault::Vault::create(
                temp.path().join("vault"),
                &SecretString::from("password".to_owned()),
            )
            .unwrap(),
        );
        let target_workspace = Nit::create_vault_workspace(&vault, "Target").unwrap();
        let target = Nit::open_vault(vault, target_workspace.id).unwrap();
        let archived = source
            .create(Kind::Note, None, "Archived note".into())
            .unwrap();
        source.archive(&archived.to_string()).unwrap();
        source
            .create(Kind::Todo, Some(Horizon::Short), "Active task".into())
            .unwrap();

        assert_eq!(copy_workspace(&source, &target).unwrap(), (1, 1));
        assert_eq!(target.all().unwrap(), source.all().unwrap());
        assert_eq!(copy_workspace(&source, &target).unwrap(), (1, 1));
        target
            .create(Kind::Item, None, "Different target entry".into())
            .unwrap();
        assert!(copy_workspace(&source, &target).is_err());
    }

    #[test]
    fn drive_creation_refuses_ambiguous_mount_selection() {
        let mut device = removable_device();
        assert!(unique_mount_point(&device).is_err());
        device.mount_points.push(PathBuf::from("/media/NIT"));
        assert_eq!(
            unique_mount_point(&device).unwrap(),
            PathBuf::from("/media/NIT")
        );
        device.mount_points.push(PathBuf::from("/mnt/NIT"));
        assert!(unique_mount_point(&device).is_err());
    }

    #[test]
    fn automatic_unlock_accepts_one_drive_without_exposing_its_path() {
        assert!(select_mounted_drive(Vec::new()).is_err());
        let root = select_mounted_drive(vec![MountedDrive {
            root: PathBuf::from("/run/media/user/NIT_DRIVE"),
            model: "Test USB".into(),
            capacity_bytes: 8 * 1024 * 1024 * 1024,
        }])
        .unwrap();
        assert_eq!(root, PathBuf::from("/run/media/user/NIT_DRIVE"));
    }

    #[test]
    fn drive_listing_marks_unsafe_targets() {
        let mut device = removable_device();
        assert_eq!(device_safety_label(&device), "eligible removable device");
        device.system_disk = true;
        assert_eq!(
            device_safety_label(&device),
            "REJECTED: system/root/boot disk"
        );
    }

    fn removable_device() -> RemovableDevice {
        RemovableDevice {
            id: "/dev/sdb".into(),
            model: "Test USB".into(),
            capacity_bytes: 16 * 1024 * 1024 * 1024,
            mount_points: Vec::new(),
            removable: true,
            system_disk: false,
            read_only: false,
        }
    }

    #[test]
    fn active_or_unavailable_drive_never_falls_back_to_plain_storage() {
        let unlocked = SessionStatus::Unlocked {
            vault_id: "vault".into(),
            workspace_id: "workspace".into(),
            workspace_name: "NIT".into(),
        };
        assert_eq!(
            storage_preference(Ok(unlocked.clone())),
            StoragePreference::Vault(unlocked)
        );
        assert_eq!(
            storage_preference(Ok(SessionStatus::Unavailable)),
            StoragePreference::Unavailable
        );
        assert_eq!(
            storage_preference(Ok(SessionStatus::Locked)),
            StoragePreference::Plain
        );
        assert_eq!(
            storage_preference(Err(anyhow::anyhow!("agent absent"))),
            StoragePreference::Plain
        );
    }

    #[test]
    fn removed_viewer_command_points_to_nitcat() {
        let error = parse_arguments(words(&["-v", "N-0001"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("nitcat"));
    }

    #[test]
    fn list_filters_use_combined_codes() {
        assert_eq!(
            parse_arguments(words(&["-ls", "-n", "--archived"])).unwrap(),
            Action::List {
                classification: Some((Kind::Note, None)),
                archived: true,
            }
        );
        assert!(parse_arguments(words(&["-ls", "note"])).is_err());
        assert!(parse_arguments(words(&["-list"])).is_err());
    }

    #[test]
    fn completion_generation_does_not_require_a_workspace() {
        assert_eq!(
            parse_arguments(words(&["-completions", "fish"])).unwrap(),
            Action::Completions(CompletionShell::Fish)
        );
        assert!(completion_script(CompletionShell::Bash).contains("complete -F _nit nit"));
        assert!(parse_arguments(words(&["-completions", "powershell"])).is_err());
    }

    #[test]
    fn literal_type_subcommands_no_longer_exist() {
        assert!(matches!(
            parse_arguments(words(&["note", "something", "-n"])).unwrap(),
            Action::Capture(_)
        ));
    }

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
}
