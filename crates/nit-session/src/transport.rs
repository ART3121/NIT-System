use anyhow::{Context, Result};
use interprocess::local_socket::{Listener, Stream};

#[cfg(unix)]
mod platform {
    use std::{
        fs,
        io::ErrorKind,
        os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
        path::PathBuf,
    };

    use anyhow::{bail, Context, Result};
    use interprocess::local_socket::{
        prelude::*, GenericFilePath, Listener, ListenerOptions, Stream,
    };

    fn effective_uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    fn validate_private_directory(directory: &PathBuf) -> Result<()> {
        let metadata = fs::symlink_metadata(directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!(
                "invalid NIT Session runtime directory {}",
                directory.display()
            );
        }
        if metadata.uid() != effective_uid() {
            bail!(
                "NIT Session runtime directory {} is owned by another user",
                directory.display()
            );
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!(
                "NIT Session runtime directory {} has unsafe permissions",
                directory.display()
            );
        }
        Ok(())
    }

    fn private_runtime_directory() -> Result<PathBuf> {
        let directory = std::env::temp_dir().join(format!("nit-session-{}", effective_uid()));
        match fs::create_dir(&directory) {
            Ok(()) => fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not create NIT Session runtime directory {}",
                        directory.display()
                    )
                })
            }
        }
        validate_private_directory(&directory)?;
        Ok(directory)
    }

    fn path(endpoint: &str) -> Result<PathBuf> {
        Ok(private_runtime_directory()?.join(format!("{endpoint}.sock")))
    }

    pub(super) fn listen(endpoint: &str) -> Result<Listener> {
        let path = path(endpoint)?;
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() {
                bail!(
                    "refusing non-socket NIT Session endpoint {}",
                    path.display()
                );
            }
            let probe_name = path
                .clone()
                .to_fs_name::<GenericFilePath>()
                .context("invalid Unix NIT Session socket path")?;
            match Stream::connect(probe_name) {
                Ok(stream) => {
                    authenticate(&stream)?;
                    bail!("NIT Session Agent is already running");
                }
                Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                    fs::remove_file(&path).with_context(|| {
                        format!(
                            "could not reclaim stale NIT Session socket {}",
                            path.display()
                        )
                    })?;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("could not inspect NIT Session socket {}", path.display())
                    })
                }
            }
        }
        let name = path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .context("invalid Unix NIT Session socket path")?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .with_context(|| format!("could not bind NIT Session socket {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "could not restrict NIT Session socket permissions at {}",
                path.display()
            )
        })?;
        Ok(listener)
    }

    pub(super) fn connect(endpoint: &str) -> Result<Stream> {
        let path = path(endpoint)?;
        let name = path
            .clone()
            .to_fs_name::<GenericFilePath>()
            .context("invalid Unix NIT Session socket path")?;
        let stream = Stream::connect(name)
            .with_context(|| format!("NIT Session Agent is not running at {}", path.display()))?;
        authenticate(&stream)?;
        Ok(stream)
    }

    pub(super) fn authenticate(stream: &Stream) -> Result<()> {
        let credentials = stream
            .peer_creds()
            .context("could not authenticate NIT Session peer")?;
        let peer_uid = credentials
            .euid()
            .context("NIT Session peer did not provide a user identity")?;
        if peer_uid != effective_uid() {
            bail!("rejected NIT Session connection from another user");
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::net::UnixListener;

        use super::*;

        #[test]
        fn stale_socket_is_reclaimed_but_regular_file_is_never_removed() {
            let endpoint = format!("nit-stale-{}", std::process::id());
            let socket_path = path(&endpoint).unwrap();
            let _ = fs::remove_file(&socket_path);
            let stale = UnixListener::bind(&socket_path).unwrap();
            drop(stale);
            let listener = listen(&endpoint).unwrap();
            drop(listener);

            fs::write(&socket_path, b"do not delete").unwrap();
            assert!(listen(&endpoint).is_err());
            assert_eq!(fs::read(&socket_path).unwrap(), b"do not delete");
            fs::remove_file(socket_path).unwrap();
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, io, mem, ptr};

    use anyhow::{bail, Context, Result};
    use interprocess::local_socket::{
        prelude::*, GenericNamespaced, Listener, ListenerOptions, Stream,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: this wrapper owns a successful Win32 handle and closes it once.
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    fn process_token(process: HANDLE) -> Result<OwnedHandle> {
        let mut token = ptr::null_mut();
        // SAFETY: `token` points to valid writable storage and `process` is a
        // live process handle (or the documented current-process pseudo handle).
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error()).context("could not query process token");
        }
        Ok(OwnedHandle(token))
    }

    fn token_user(token: HANDLE) -> Result<Vec<usize>> {
        let mut bytes = 0_u32;
        // SAFETY: this is the documented sizing call with a null output buffer.
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes);
        }
        if bytes < mem::size_of::<TOKEN_USER>() as u32 {
            return Err(io::Error::last_os_error()).context("could not size process identity");
        }
        let words = (bytes as usize).div_ceil(mem::size_of::<usize>());
        let mut buffer = vec![0_usize; words];
        let mut written = bytes;
        // SAFETY: `buffer` is aligned for `TOKEN_USER`, has at least `bytes`
        // writable bytes, and remains alive while its embedded SID is used.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast::<c_void>(),
                bytes,
                &mut written,
            )
        } == 0
        {
            return Err(io::Error::last_os_error()).context("could not read process identity");
        }
        Ok(buffer)
    }

    fn same_user(peer_pid: u32) -> Result<bool> {
        // SAFETY: OpenProcess validates the PID and returns either null or an owned handle.
        let peer_process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, peer_pid) };
        if peer_process.is_null() {
            return Err(io::Error::last_os_error()).context("could not inspect NIT Session peer");
        }
        let peer_process = OwnedHandle(peer_process);
        let peer_token = process_token(peer_process.0)?;
        // SAFETY: GetCurrentProcess returns the documented valid pseudo handle.
        let current_token = process_token(unsafe { GetCurrentProcess() })?;
        let peer_user = token_user(peer_token.0)?;
        let current_user = token_user(current_token.0)?;
        // SAFETY: both aligned buffers contain successful TOKEN_USER results
        // and remain alive for the duration of EqualSid.
        let equal = unsafe {
            let peer = &*(peer_user.as_ptr().cast::<TOKEN_USER>());
            let current = &*(current_user.as_ptr().cast::<TOKEN_USER>());
            EqualSid(peer.User.Sid, current.User.Sid) != 0
        };
        Ok(equal)
    }

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
        let stream = Stream::connect(name)
            .with_context(|| format!("NIT Session Agent is not running at {endpoint}"))?;
        authenticate(&stream)?;
        Ok(stream)
    }

    pub(super) fn authenticate(stream: &Stream) -> Result<()> {
        let credentials = stream
            .peer_creds()
            .context("could not authenticate NIT Session peer")?;
        let peer_pid = credentials
            .pid()
            .context("NIT Session peer did not provide a process identity")?;
        if !same_user(peer_pid)? {
            bail!("rejected NIT Session connection from another user");
        }
        Ok(())
    }
}

pub(crate) fn listen(endpoint: &str) -> Result<Listener> {
    platform::listen(endpoint).context("could not start NIT Session transport")
}

pub(crate) fn connect(endpoint: &str) -> Result<Stream> {
    platform::connect(endpoint).context("could not connect to NIT Session transport")
}

pub(crate) fn authenticate(stream: &Stream) -> Result<()> {
    platform::authenticate(stream).context("NIT Session peer authentication failed")
}
