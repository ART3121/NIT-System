use std::{fs, path::Path, process::Command};

use anyhow::{bail, Context, Result};

const SUPPORTED_EDITORS: [&str; 4] = ["nvim", "vim", "vi", "nano"];

pub(crate) fn open(initial: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("nit-edit-{}.md", std::process::id()));
    fs::write(&path, initial)?;

    let edit_result = (|| -> Result<String> {
        launch(&path, &SUPPORTED_EDITORS)?;
        let edited = fs::read_to_string(&path)?;
        let edited = edited.trim().to_owned();
        if edited.is_empty() {
            bail!("refusing to save an empty entry");
        }
        Ok(edited)
    })();

    let _ = fs::remove_file(&path);
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
