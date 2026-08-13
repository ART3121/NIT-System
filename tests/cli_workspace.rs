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
    path.canonicalize().unwrap()
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

fn nitcat(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nitcat"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn suite_binaries_share_the_workspace_version() {
    let root = temporary_directory("versions");
    assert_eq!(successful(&root, &["-version"]).trim(), "nit 0.6.0");
    let output = Command::new(env!("CARGO_BIN_EXE_nitcat"))
        .current_dir(&root)
        .arg("-version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "nitcat 0.6.0"
    );

    let removed = nit(&root, &["-v", "N-0001"]);
    assert!(!removed.status.success());
    assert!(String::from_utf8_lossy(&removed.stderr).contains("nitcat"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn both_binaries_generate_completions_and_dynamic_ids() {
    let root = temporary_directory("completions");
    assert!(successful(&root, &["-completions", "bash"]).contains("complete -F _nit nit"));
    let cat_completion = nitcat(&root, &["-completions", "fish"]);
    assert!(cat_completion.status.success());
    assert!(String::from_utf8(cat_completion.stdout)
        .unwrap()
        .contains("complete -c nitcat"));

    successful(&root, &["-init"]);
    successful(&root, &["Reference", "-n"]);
    successful(&root, &["Task", "-st"]);
    assert_eq!(successful(&root, &["-completion-ids"]), "N-0001\nST-0001\n");

    let note_ids = nitcat(&root, &["-completion-ids"]);
    assert!(note_ids.status.success());
    assert_eq!(String::from_utf8(note_ids.stdout).unwrap(), "N-0001\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nested_commands_share_the_initialized_workspace() {
    let root = temporary_directory("nested");
    successful(&root, &["-init"]);
    assert!(root.join(".nit/notes").is_dir());
    assert!(root.join(".nit/archive").is_dir());
    assert!(root.join(".nit/ideas").is_file());
    assert!(root.join(".nit/items").is_file());
    assert!(root.join(".nit/todos").is_file());
    assert!(root.join(".nit/next-ids").is_file());

    successful(&root, &["Fix", "parser", "-st"]);
    let nested = root.join("src/parser");
    fs::create_dir_all(&nested).unwrap();
    let listed = successful(&nested, &["-ls"]);
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
    let listed = successful(&root, &["-ls"]);
    assert!(listed.contains("[ST-0001]"));
    assert!(listed.contains("[ST-0002]"));
    assert!(listed.contains("[N-0001]"));
    assert!(listed.contains("[X-0001]"));
    assert!(listed.contains("[LI-0001]"));
    assert_eq!(
        fs::read_to_string(root.join(".nit/notes/N-0001.md")).unwrap(),
        "# A note\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_captures_keep_every_entry_and_allocate_unique_ids() {
    let root = temporary_directory("concurrent-capture");
    successful(&root, &["-init"]);

    let mut children = Vec::new();
    for index in 0..12 {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_nit"))
                .current_dir(&root)
                .args([format!("parallel entry {index}"), "-n".into()])
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    let listed = successful(&root, &["-ls", "-n"]);
    assert_eq!(
        listed
            .lines()
            .filter(|line| line.contains("parallel entry"))
            .count(),
        12
    );
    let mut ids = listed
        .lines()
        .filter_map(|line| line.split_once(']').map(|(id, _)| id.to_owned()))
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 12);
}

#[test]
fn ls_keeps_all_entry_classes_but_hides_note_bodies() {
    let root = temporary_directory("ls-note-titles");
    successful(&root, &["-init"]);
    successful(&root, &["Reference", "note", "-n"]);
    successful(&root, &["Fix", "parser", "-st"]);
    fs::write(
        root.join(".nit/notes/N-0001.md"),
        "# Reference note\n\nPrivate body shown only by nitcat.\n",
    )
    .unwrap();

    let listed = successful(&root, &["-ls"]);
    assert!(listed.contains("- [N-0001] Reference note"));
    assert!(!listed.contains("Private body shown only by nitcat"));
    assert!(listed.contains("- [ST-0001] Fix parser"));

    let removed = nit(&root, &["-list"]);
    assert!(!removed.status.success());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn search_finds_note_bodies_and_can_include_the_archive() {
    let root = temporary_directory("search");
    successful(&root, &["-init"]);
    successful(&root, &["Architecture", "notes", "-n"]);
    fs::write(
        root.join(".nit/notes/N-0001.md"),
        "# Architecture notes\n\nThe scheduler delegates retry policy to the worker.\n",
    )
    .unwrap();

    let body_match = successful(&root, &["-search", "retry policy"]);
    assert!(body_match.contains("N-0001 · note · Architecture notes"));
    successful(&root, &["-archive", "N-0001"]);
    assert!(successful(&root, &["-search", "retry policy"]).is_empty());
    let archived = successful(&root, &["-search", "retry policy", "--all"]);
    assert!(archived.contains("N-0001 · note · Architecture notes · archived"));
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
    let todos = fs::read_to_string(root.join(".nit/todos")).unwrap();
    assert!(todos.contains("[ST-0002] Imported task"));

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

    let discovery = nit(&root, &["-ls"]);
    assert!(!discovery.status.success());
    assert!(String::from_utf8_lossy(&discovery.stderr).contains("Legacy NIT workspace detected"));

    successful(&root, &["-migrate"]);
    assert_eq!(fs::read_to_string(root.join(".nit/notes")).unwrap(), legacy);
    assert_eq!(
        fs::read_to_string(root.join(".notes.legacy.bak")).unwrap(),
        legacy
    );
    assert!(root.join(".nit/next-ids").is_file());
    let before_ids = nit(&root, &["-ls"]);
    assert!(!before_ids.status.success());
    assert!(String::from_utf8_lossy(&before_ids.stderr).contains("-assign-ids"));
    let capture_before_ids = nit(&root, &["New", "entry", "-n"]);
    assert!(!capture_before_ids.status.success());
    assert!(String::from_utf8_lossy(&capture_before_ids.stderr).contains("-assign-ids"));
    let assigned = successful(&root, &["-assign-ids"]);
    assert!(assigned.contains("Assigned IDs to 1 entries"));
    assert!(successful(&root, &["-ls"]).contains("- [N-0001] legacy entry"));
    assert!(root.join(".nit/backups/layout-v0.2/notes").is_file());
    assert!(root.join(".nit/notes/N-0001.md").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn timed_note_ids_require_and_support_safe_migration() {
    let root = temporary_directory("timeless-migration");
    fs::create_dir(root.join(".nit")).unwrap();
    fs::write(
        root.join(".nit/notes"),
        "# NIT System\n\n## Short Term\n\n### Notes\n- [SN-0001] old note\n",
    )
    .unwrap();
    fs::write(root.join(".nit/archive"), "# NIT System — Archived\n").unwrap();
    fs::write(
        root.join(".nit/next-ids"),
        "# NIT ID sequences\nSN 2\nN 1\n",
    )
    .unwrap();

    let blocked_list = nit(&root, &["-ls"]);
    assert!(!blocked_list.status.success());
    assert!(String::from_utf8_lossy(&blocked_list.stderr).contains("-migrate-timeless"));
    let blocked = nit(&root, &["New", "note", "-n"]);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("-migrate-timeless"));

    let migrated = successful(&root, &["-migrate-timeless"]);
    assert!(migrated.contains("Migrated 1 Note/Item IDs"));
    assert!(successful(&root, &["-ls"]).contains("[N-0002] old note"));
    assert_eq!(
        fs::read_to_string(root.join(".nit/notes/N-0002.md")).unwrap(),
        "# old note\n"
    );
    assert!(root.join(".nit/backups/layout-v0.2/notes").is_file());
    assert!(successful(&root, &["New", "note", "-n"]).contains("N-0003"));
    fs::remove_dir_all(root).unwrap();
}
