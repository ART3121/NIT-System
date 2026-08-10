use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn temporary_directory(name: &str) -> PathBuf {
    let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("nit-cli-{name}-{}-{number}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn nit(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nit"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

fn successful(directory: &Path, arguments: &[&str]) -> String {
    let output = nit(directory, arguments);
    assert!(
        output.status.success(),
        "nit {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn nested_commands_share_the_initialized_workspace() {
    let root = temporary_directory("nested");
    successful(&root, &["-init"]);
    assert!(root.join(".nit/notes").is_file());
    assert!(root.join(".nit/archive").is_file());
    assert!(root.join(".nit/next-ids").is_file());

    successful(&root, &["Fix", "parser", "-st"]);
    let nested = root.join("src/parser");
    fs::create_dir_all(&nested).unwrap();
    let listed = successful(&nested, &["-list"]);
    assert!(listed.contains("- [ST-0001] Fix parser"));
    let shown = successful(&nested, &["-show", "st-1"]);
    assert!(shown.starts_with("ST-0001\nshort/todo"));
    assert_eq!(
        successful(&nested, &["-root"]).trim(),
        root.display().to_string()
    );
    assert_eq!(
        successful(&nested, &["-path"]).trim(),
        root.join(".nit").display().to_string()
    );
    let status = successful(&nested, &["-status"]);
    assert!(status.contains("Active entries: 1"));
    assert!(status.contains("Archived entries: 0"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn id_sequences_are_independent_by_classification() {
    let root = temporary_directory("ids");
    successful(&root, &["-init"]);
    assert!(successful(&root, &["First", "task", "-st"]).contains("ST-0001"));
    assert!(successful(&root, &["Second", "task", "-st"]).contains("ST-0002"));
    assert!(successful(&root, &["A", "note", "-n"]).contains("N-0001"));
    assert!(successful(&root, &["An", "item", "-x"]).contains("X-0001"));
    assert!(successful(&root, &["An", "idea", "-li"]).contains("LI-0001"));
    let notes = fs::read_to_string(root.join(".nit/notes")).unwrap();
    assert!(notes.contains("[ST-0001]"));
    assert!(notes.contains("[ST-0002]"));
    assert!(notes.contains("[N-0001]"));
    assert!(notes.contains("[X-0001]"));
    assert!(notes.contains("[LI-0001]"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn imports_assign_missing_ids_and_reject_conflicts() {
    let root = temporary_directory("id-import");
    successful(&root, &["-init"]);
    successful(&root, &["Existing", "task", "-st"]);
    fs::write(
        root.join("import.md"),
        "# Imported\n\n## Short Term\n\n### To-dos\n- Imported task\n",
    )
    .unwrap();
    assert!(successful(&root, &["-import", "import.md"]).contains("Imported 1 entries"));
    let notes = fs::read_to_string(root.join(".nit/notes")).unwrap();
    assert!(notes.contains("[ST-0002] Imported task"));

    fs::write(
        root.join("conflict.md"),
        "# Imported\n\n## Short Term\n\n### To-dos\n- [ST-0001] Conflict\n",
    )
    .unwrap();
    let conflict = nit(&root, &["-import", "conflict.md"]);
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("duplicate entry ID"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_and_tracked_initialization_preserve_gitignore() {
    let private = temporary_directory("private");
    fs::write(private.join(".gitignore"), "target/\n").unwrap();
    successful(&private, &["-init", "--private"]);
    successful(&private, &["-init", "--private"]);
    assert_eq!(
        fs::read_to_string(private.join(".gitignore")).unwrap(),
        "target/\n.nit/\n"
    );

    let tracked = temporary_directory("tracked");
    fs::write(tracked.join(".gitignore"), ".nit/\n").unwrap();
    let output = successful(&tracked, &["-init", "--tracked"]);
    assert!(output.contains("appears to be ignored"));
    assert_eq!(
        fs::read_to_string(tracked.join(".gitignore")).unwrap(),
        ".nit/\n"
    );
    fs::remove_dir_all(private).unwrap();
    fs::remove_dir_all(tracked).unwrap();
}

#[test]
fn legacy_workspace_requires_explicit_migration() {
    let root = temporary_directory("legacy");
    let legacy = "# NIT System\n\n## Short Term\n\n### Notes\n- legacy entry\n";
    fs::write(root.join(".notes"), legacy).unwrap();

    let discovery = nit(&root, &["-list"]);
    assert!(!discovery.status.success());
    assert!(String::from_utf8_lossy(&discovery.stderr).contains("Legacy NIT workspace detected"));

    successful(&root, &["-migrate"]);
    assert_eq!(fs::read_to_string(root.join(".nit/notes")).unwrap(), legacy);
    assert_eq!(
        fs::read_to_string(root.join(".notes.legacy.bak")).unwrap(),
        legacy
    );
    assert!(root.join(".nit/next-ids").is_file());
    assert!(successful(&root, &["-list"]).contains("- legacy entry"));
    let capture_before_ids = nit(&root, &["New", "entry", "-n"]);
    assert!(!capture_before_ids.status.success());
    assert!(String::from_utf8_lossy(&capture_before_ids.stderr).contains("-assign-ids"));
    let assigned = successful(&root, &["-assign-ids"]);
    assert!(assigned.contains("Assigned IDs to 1 entries"));
    assert!(successful(&root, &["-list"]).contains("- [N-0001] legacy entry"));
    assert!(root.join(".nit/notes.pre-ids.bak").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timed_note_ids_require_and_support_safe_migration() {
    let root = temporary_directory("timeless-migration");
    successful(&root, &["-init"]);
    fs::write(
        root.join(".nit/notes"),
        "# NIT System\n\n## Short Term\n\n### Notes\n- [SN-0001] old note\n",
    )
    .unwrap();

    assert!(successful(&root, &["-list"]).contains("[SN-0001] old note"));
    let blocked = nit(&root, &["New", "note", "-n"]);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("-migrate-timeless"));

    let migrated = successful(&root, &["-migrate-timeless"]);
    assert!(migrated.contains("Migrated 1 Note/Item IDs"));
    let notes = fs::read_to_string(root.join(".nit/notes")).unwrap();
    assert!(notes.contains("## Timeless"));
    assert!(notes.contains("[N-0001] old note"));
    assert!(root.join(".nit/notes.pre-timeless.bak").is_file());
    assert!(root.join(".nit/archive.pre-timeless.bak").is_file());
    assert!(root.join(".nit/next-ids.pre-timeless.bak").is_file());
    assert!(successful(&root, &["New", "note", "-n"]).contains("N-0002"));
    fs::remove_dir_all(root).unwrap();
}
