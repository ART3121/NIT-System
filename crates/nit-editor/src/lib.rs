use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    process::Command,
};

use anyhow::{bail, Context, Result};

const SUPPORTED_EDITORS: [&str; 4] = ["nvim", "vim", "vi", "nano"];
const MAX_EDIT_BYTES: u64 = 16 * 1024 * 1024;

pub fn open(initial: &str) -> Result<String> {
    let mut temporary = tempfile::Builder::new()
        .prefix("nit-edit-")
        .suffix(".md")
        .tempfile()
        .context("could not create a private editor file")?;
    temporary.write_all(initial.as_bytes())?;
    temporary.flush()?;
    let path = temporary.path();

    let edit_result = (|| -> Result<String> {
        launch(path, &SUPPORTED_EDITORS)?;
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_EDIT_BYTES {
            bail!("edited document exceeds the {} byte limit", MAX_EDIT_BYTES);
        }
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        File::open(path)?
            .take(MAX_EDIT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_EDIT_BYTES {
            bail!("edited document exceeds the {} byte limit", MAX_EDIT_BYTES);
        }
        let edited = String::from_utf8(bytes).context("edited document is not valid UTF-8")?;
        let edited = edited.trim().to_owned();
        if edited.is_empty() {
            bail!("refusing to save an empty entry");
        }
        Ok(edited)
    })();

    edit_result
}

fn launch(path: &Path, editors: &[&str]) -> Result<()> {
    for editor in editors {
        match Command::new(editor).arg(path).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(_) => bail!("{editor} exited unsuccessfully"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("could not start {editor}")),
        }
    }

    bail!(
        "no supported text editor found; install one of: {}",
        editors.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_when_a_command_is_missing() {
        assert!(launch(
            Path::new("/tmp/nit-editor-fallback-test"),
            &["nit-editor-that-does-not-exist", "true"]
        )
        .is_ok());
    }

    #[test]
    fn priority_is_stable() {
        assert_eq!(SUPPORTED_EDITORS, ["nvim", "vim", "vi", "nano"]);
    }
}
