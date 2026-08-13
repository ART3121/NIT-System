use anyhow::{Context, Result};
use interprocess::local_socket::{Listener, Stream};

#[cfg(unix)]
mod platform {
    use std::{fs, path::PathBuf};

    use anyhow::{bail, Context, Result};
    use interprocess::local_socket::{
        prelude::*, GenericFilePath, Listener, ListenerOptions, Stream,
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn private_runtime_directory() -> Result<PathBuf> {
        let directory = std::env::temp_dir().join(format!("nit-session-{}", user_identity()));
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "could not create NIT Session runtime directory {}",
                directory.display()
            )
        })?;
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "invalid NIT Session runtime directory {}",
                directory.display()
            );
        }
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        Ok(directory)
    }

    fn user_identity() -> String {
        std::env::var("UID")
            .ok()
            .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
            .unwrap_or_else(|| "current-user".into())
    }

    fn path(endpoint: &str) -> Result<PathBuf> {
        Ok(private_runtime_directory()?.join(format!("{endpoint}.sock")))
    }

    pub(super) fn listen(endpoint: &str) -> Result<Listener> {
        let path = path(endpoint)?;
        let name = path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .context("invalid Unix NIT Session socket path")?;
        ListenerOptions::new()
            .name(name)
            .create_sync()
            .with_context(|| format!("could not bind NIT Session socket {}", path.display()))
    }

    pub(super) fn connect(endpoint: &str) -> Result<Stream> {
        let path = path(endpoint)?;
        let name = path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .context("invalid Unix NIT Session socket path")?;
        Stream::connect(name)
            .with_context(|| format!("NIT Session Agent is not running at {}", path.display()))
    }
}

#[cfg(windows)]
mod platform {
    use anyhow::{Context, Result};
    use interprocess::local_socket::{
        prelude::*, GenericNamespaced, Listener, ListenerOptions, Stream,
    };

    pub(super) fn listen(endpoint: &str) -> Result<Listener> {
        let name = endpoint
            .to_ns_name::<GenericNamespaced>()
            .context("invalid Windows NIT Session Named Pipe")?;
        ListenerOptions::new()
            .name(name)
            .create_sync()
            .with_context(|| format!("could not bind NIT Session Named Pipe {endpoint}"))
    }

    pub(super) fn connect(endpoint: &str) -> Result<Stream> {
        let name = endpoint
            .to_ns_name::<GenericNamespaced>()
            .context("invalid Windows NIT Session Named Pipe")?;
        Stream::connect(name)
            .with_context(|| format!("NIT Session Agent is not running at {endpoint}"))
    }
}

pub(crate) fn listen(endpoint: &str) -> Result<Listener> {
    platform::listen(endpoint).context("could not start NIT Session transport")
}

pub(crate) fn connect(endpoint: &str) -> Result<Stream> {
    platform::connect(endpoint).context("could not connect to NIT Session transport")
}
