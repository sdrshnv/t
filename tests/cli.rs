use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

struct App {
    data: TempDir,
    binary: PathBuf,
}

impl App {
    fn new() -> Self {
        Self {
            data: tempfile::tempdir().unwrap(),
            binary: assert_cmd::cargo::cargo_bin!("t").to_path_buf(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env("XDG_DATA_HOME", self.data.path())
            .env_remove("VISUAL")
            .env_remove("EDITOR");
        command
    }

    fn run(&self, arguments: &[&str]) {
        self.command().args(arguments).assert().success();
    }

    fn editor(&self, name: &str, body: &str) -> PathBuf {
        let path = self.data.path().join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }
}

#[test]
fn canonical_tax_workflow_persists_and_schedules_dependencies() {
    let app = App::new();
    app.command()
        .args(["add", "1", "File", "taxes"])
        .assert()
        .success()
        .stdout("added #1\n")
        .stderr("");
    app.command()
        .args(["add", "2", "Gather documents"])
        .assert()
        .success()
        .stdout("added #2\n");
    app.command()
        .args(["add", "3", "Download W-2"])
        .assert()
        .success()
        .stdout("added #3\n");
    app.run(&["dep", "1", "2"]);
    app.run(&["dep", "2", "3"]);

    app.command()
        .assert()
        .success()
        .stdout("#1 [1] File taxes\n  #2 [1←2] Gather documents\n    #3 [1←3] Download W-2\n");
    app.command()
        .arg("done")
        .assert()
        .success()
        .stdout("done #3\n");
    app.command()
        .assert()
        .success()
        .stdout("#1 [1] File taxes\n  #2 [1←2] Gather documents\n");
    app.command()
        .args(["show", "3"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "✓ #3 [3] Download W-2\nstatus: done\n",
        ));
}

#[test]
fn empty_states_and_error_exit_codes_follow_the_contract() {
    let app = App::new();
    for arguments in [vec![], vec!["ls"], vec!["tree"], vec!["find", "missing"]] {
        app.command()
            .args(arguments)
            .assert()
            .success()
            .stdout("")
            .stderr("");
    }
    app.command()
        .args(["done", "99"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("t: task 99 not found\n");
    app.command()
        .args(["add", "4", "invalid"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("invalid value '4'"));
}

#[test]
fn list_modes_search_tree_and_numeric_shortcuts_are_deterministic() {
    let app = App::new();
    app.run(&["add", "2", "Parent TAX task"]);
    app.run(&["add", "3", "shared receipt"]);
    app.run(&["add", "1", "Second tax parent"]);
    app.run(&["dep", "1", "2"]);
    app.run(&["dep", "3", "2"]);

    app.command()
        .arg("ls")
        .assert()
        .success()
        .stdout("#2 [1←3] shared receipt\n");
    app.command()
        .args(["ls", "--blocked"])
        .assert()
        .success()
        .stdout("#3 [1] Second tax parent\n#1 [2] Parent TAX task\n");
    app.command()
        .arg("1")
        .assert()
        .success()
        .stdout("#3 [1] Second tax parent\n  #2 [1←3] shared receipt\n");
    app.command().arg("2").assert().success().stdout("");

    app.command()
        .args(["find", "TaX"])
        .assert()
        .success()
        .stdout("#1 [2] Parent TAX task\n#3 [1] Second tax parent\n");
    app.command().arg("tree").assert().success().stdout(
        "#1 [2] Parent TAX task\n└─ #2 [1←3] shared receipt\n\n#3 [1] Second tax parent\n└─ #2 [1←3] shared receipt [see above]\n",
    );

    app.run(&["done", "2"]);
    app.command()
        .args(["ls", "--done"])
        .assert()
        .success()
        .stdout("✓ #2 [3] shared receipt\n");
    app.command()
        .args(["ls", "--all"])
        .assert()
        .success()
        .stdout("#3 [1] Second tax parent\n#1 [2] Parent TAX task\n✓ #2 [3] shared receipt\n");
}

#[test]
fn soft_deletion_force_and_multi_level_undo_restore_the_graph_and_ids() {
    let app = App::new();
    app.run(&["add", "1", "parent"]);
    app.run(&["add", "2", "child"]);
    app.run(&["dep", "1", "2"]);
    app.command()
        .args(["rm", "2"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("use --force"));
    app.command()
        .args(["rm", "2", "--force"])
        .assert()
        .success()
        .stdout("removed #2\n");
    app.command()
        .args(["show", "2"])
        .assert()
        .code(1)
        .stderr("t: task 2 not found\n");
    app.command()
        .arg("undo")
        .assert()
        .success()
        .stdout("undid rm\n");
    app.command()
        .arg("undo")
        .assert()
        .success()
        .stdout("undid dep\n");
    app.command()
        .arg("undo")
        .assert()
        .success()
        .stdout("undid add\n");
    app.command()
        .args(["add", "3", "replacement"])
        .assert()
        .success()
        .stdout("added #2\n");
}

#[test]
fn editor_success_failure_and_conflict_do_not_hold_or_corrupt_history() {
    let app = App::new();
    app.run(&["add", "1", "original"]);

    let success = app.editor("edit-success", "printf '%s\\n' 'edited text' > \"$1\"");
    app.command()
        .args(["edit", "1"])
        .env("EDITOR", &success)
        .assert()
        .success()
        .stdout("edited #1\n");
    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#1 [1] edited text\n"));

    let failure = app.editor("edit-failure", "exit 7");
    app.command()
        .args(["edit", "1"])
        .env("EDITOR", &failure)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("editor exited with status"));
    app.command()
        .arg("undo")
        .assert()
        .success()
        .stdout("undid edit\n");

    let inner = app.editor("edit-inner", "printf '%s\\n' 'concurrent text' > \"$1\"");
    let outer = app.editor(
        "edit-outer",
        "EDITOR=\"$INNER_EDITOR\" \"$T_BINARY\" edit 1 >/dev/null",
    );
    app.command()
        .args(["edit", "1"])
        .env("EDITOR", &outer)
        .env("INNER_EDITOR", &inner)
        .env("T_BINARY", &app.binary)
        .assert()
        .code(1)
        .stderr("t: task 1 changed while the editor was open\n");
    app.command()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("#1 [1] concurrent text\n"));
}

#[test]
fn completion_scripts_are_generated_without_creating_storage() {
    let app = App::new();
    for (shell, marker) in [
        ("bash", "_t()"),
        ("zsh", "#compdef t"),
        ("fish", "complete -c t"),
    ] {
        app.command()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains(marker))
            .stderr("");
    }
    assert!(!Path::new(app.data.path()).join("t/tasks.db").exists());
}

#[test]
fn storage_prefers_xdg_falls_back_to_home_and_errors_without_either() {
    let app = App::new();
    app.run(&["add", "1", "xdg task"]);
    assert!(app.data.path().join("t/tasks.db").exists());

    let home = tempfile::tempdir().unwrap();
    Command::new(&app.binary)
        .env_clear()
        .env("HOME", home.path())
        .args(["add", "2", "home task"])
        .assert()
        .success()
        .stdout("added #1\n");
    assert!(home.path().join(".local/share/t/tasks.db").exists());

    Command::new(&app.binary)
        .env_clear()
        .assert()
        .code(1)
        .stdout("")
        .stderr("t: cannot determine data directory: set XDG_DATA_HOME or HOME\n");
    Command::new(&app.binary)
        .env_clear()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c t"));
}
