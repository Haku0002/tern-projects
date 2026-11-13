use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");

fn command(db: &Path, home: &Path, args: &[&str], request: Option<&Value>) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.env("TERN_PROJECTS_HOME", home)
        .arg("--db")
        .arg(db)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if request.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().unwrap();
    if let Some(request) = request {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn run(db: &Path, home: &Path, args: &[&str]) -> Value {
    let out = command(db, home, args, None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn library(db: &Path, home: &Path, request: Value) -> Value {
    let out = command(db, home, &["library"], Some(&request));
    assert!(
        out.status.success(),
        "request {request}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn fails(db: &Path, home: &Path, request: Value) -> String {
    let out = command(db, home, &["library"], Some(&request));
    assert!(
        !out.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stderr).unwrap()
}
fn seed(home: &Path, id: &str, cwd: &Path, updated: i64) -> PathBuf {
    let dir = home.join(".omp/agent/sessions/fixture");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{id}.jsonl"));
    let records = [
        json!({"type":"session","id":id,"cwd":cwd,"timestamp":updated-1}),
        json!({"type":"message","timestamp":updated,"message":{"role":"user","content":[{"type":"text","text":"shared searchable needle"}]}}),
    ];
    std::fs::write(
        &file,
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    file
}
fn snapshot(db: &Path, home: &Path) -> Value {
    library(
        db,
        home,
        json!({"op":"snapshot","view":"mine","per":25,"recent":15}),
    )
}
fn project<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap()
}
fn create(db: &Path, home: &Path, name: &str, folders: Value) -> String {
    library(
        db,
        home,
        json!({"op":"project-save","name":name,"icon":"folder","folders":folders}),
    )["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn uids(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["uid"].as_str().unwrap())
        .collect()
}
/// Folders as the backend reports them: canonical, without Windows' verbatim prefix.
fn resolved(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap();
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(legacy) => PathBuf::from(legacy),
        None => path,
    }
}
/// The verbatim spelling earlier releases stored for Windows folders.
fn verbatim(path: &Path) -> String {
    let path = resolved(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        format!(r"\\?\{path}")
    } else {
        path
    }
}
/// An OMP stand-in that only answers `--help`; the returned PATH hides real installs.
fn fake_omp(home: &Path) -> std::ffi::OsString {
    let bin = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let help = ["omp v0.0.0", "  --cwd DIR", "  --config FILE", "  --add-dir DIR"];
    #[cfg(windows)]
    {
        let script: String = std::iter::once("@echo off".to_owned())
            .chain(help.map(|line| format!("echo {line}")))
            .map(|line| line + "\r\n")
            .collect();
        std::fs::write(bin.join("omp.cmd"), script).unwrap();
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        std::env::join_paths([system.join(r"WindowsPowerShell\v1.0"), system]).unwrap()
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let omp = bin.join("omp");
        let lines = help.map(|line| format!("'{line}'")).join(" ");
        std::fs::write(&omp, format!("#!/bin/sh\nprintf '%s\\n' {lines}\n")).unwrap();
        std::fs::set_permissions(&omp, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::ffi::OsString::new()
    }
}
fn launch_plan(db: &Path, home: &Path, path: &std::ffi::OsStr, request: Value) -> Value {
    let mut child = Command::new(BIN)
        .env("TERN_PROJECTS_HOME", home)
        .env("PATH", path)
        .arg("--db")
        .arg(db)
        .arg("library")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "request {request}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn membership_overrides_pins_archive_and_rebuild_are_consistent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("cache/index.sqlite");
    let app = home.join("app");
    let sub = app.join("sub");
    let other = home.join("app-old");
    let backend = home.join("backend");
    let empty = home.join("empty");
    for path in [&sub, &other, &backend, &empty] {
        std::fs::create_dir_all(path).unwrap();
    }
    let source = seed(&home, "app", &app, 100);
    seed(&home, "sub", &sub, 200);
    seed(&home, "other", &other, 300);
    seed(&home, "back", &backend, 400);
    let original = std::fs::read(&source).unwrap();
    run(&db, &home, &["index"]);
    let a = create(&db, &home, "A", json!([app, app.join("."), backend]));
    let b = create(&db, &home, "B", json!([sub]));
    let c = create(&db, &home, "C", json!([empty]));
    let snap = snapshot(&db, &home);
    assert_eq!(
        project(&snap, &a)["folders"],
        json!([resolved(&app), resolved(&backend)])
    );
    assert_eq!(
        uids(&project(&snap, &a)["sessions"]),
        ["omp:back", "omp:sub", "omp:app"]
    );
    assert_eq!(uids(&project(&snap, &b)["sessions"]), ["omp:sub"]);
    assert_eq!(project(&snap, &c)["count"], 0);
    let before = run(&db, &home, &["query", "--q", "needle"])["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["uid"] == "omp:sub")
        .unwrap()
        .clone();
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"project","project_id":c}}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(project(&snap, &a)["count"], 2);
    assert_eq!(project(&snap, &b)["count"], 0);
    let moved = &project(&snap, &c)["sessions"][0];
    assert_eq!(moved["cwd"], before["cwd"]);
    assert_eq!(moved["resume"], before["resume"]);
    let filtered = run(&db, &home, &["query", "--project-id", &c, "--q", "needle"]);
    assert_eq!(filtered["total"], 1);
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"none"}}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(project(&snap, &c)["count"], 0);
    assert!(uids(&snap["recent"]).contains(&"omp:sub"));
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"automatic"}}),
    );
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:app","pinned":true}),
    );
    library(
        &db,
        &home,
        json!({"op":"settings-update","default_harness":"omp","icon_style":"monochrome"}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(uids(&snap["pinned"]), ["omp:app"]);
    assert!(uids(&snap["recent"]).contains(&"omp:app"));
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:app","archived":true}),
    );
    let snap = snapshot(&db, &home);
    assert!(uids(&snap["pinned"]).is_empty());
    assert!(!uids(&snap["recent"]).contains(&"omp:app"));
    assert_eq!(project(&snap, &a)["count"], 2);
    assert_eq!(run(&db, &home, &["query", "--q", "needle"])["total"], 3);
    assert_eq!(
        uids(&library(&db, &home, json!({"op":"archived","limit":50,"offset":0}))["sessions"]),
        ["omp:app"]
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    // Destroy only the cache, then rebuild. Durable annotations must remain.
    std::fs::remove_file(&db).unwrap();
    run(&db, &home, &["index"]);
    let snap = snapshot(&db, &home);
    assert_eq!(snap["settings"]["default_harness"], "omp");
    assert_eq!(snap["settings"]["icon_style"], "monochrome");
    assert_eq!(project(&snap, &a)["count"], 2);
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:app","archived":false}),
    );
    assert_eq!(uids(&snapshot(&db, &home)["pinned"]), ["omp:app"]);
    let error = fails(
        &db,
        &home,
        json!({"op":"project-save","id":a,"name":" ","icon":"folder","folders":[app]}),
    );
    assert!(error.contains("Name"));
    assert_eq!(project(&snapshot(&db, &home), &a)["name"], "A");
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"project","project_id":c}}),
    );
    library(&db, &home, json!({"op":"project-remove","id":c}));
    assert!(source.exists());
    let after_removal = snapshot(&db, &home);
    assert_eq!(project(&after_removal, &a)["count"], 3);
    assert_eq!(project(&after_removal, &b)["count"], 1);
    library(
        &db,
        &home,
        json!({"op":"settings-update","default_harness":null}),
    );
    assert!(snapshot(&db, &home)["settings"]["default_harness"].is_null());
}

#[test]
fn curated_filter_precedes_fts_count_and_paging() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let app = home.join("app");
    let other = home.join("other");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    for i in 0..115 {
        seed(
            &home,
            &format!("s{i:03}"),
            if i < 60 { &other } else { &app },
            1000 - i,
        );
    }
    run(&db, &home, &["index"]);
    let project = create(&db, &home, "Paged", json!([app]));
    for text in ["", "needle"] {
        let first = run(
            &db,
            &home,
            &[
                "query",
                "--project-id",
                &project,
                "--q",
                text,
                "--limit",
                "50",
            ],
        );
        let second = run(
            &db,
            &home,
            &[
                "query",
                "--project-id",
                &project,
                "--q",
                text,
                "--limit",
                "50",
                "--offset",
                "50",
            ],
        );
        assert_eq!(first["total"], 55);
        assert_eq!(second["total"], 55);
        let first = uids(&first["sessions"]);
        let second = uids(&second["sessions"]);
        assert_eq!(first.first(), Some(&"omp:s060"));
        assert_eq!(first.last(), Some(&"omp:s109"));
        assert_eq!(
            second,
            ["omp:s110", "omp:s111", "omp:s112", "omp:s113", "omp:s114"]
        );
    }
}

#[test]
fn library_schema_and_folder_validation_fail_without_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = temp.path().join("index.sqlite");
    run(&db, &home, &["index"]);
    let error = fails(
        &db,
        &home,
        json!({"op":"project-save","name":"Invalid","icon":"folder","folders":[home.join("missing")]}),
    );
    assert!(error.contains("directory"));
    assert_eq!(snapshot(&db, &home)["projects"], json!([]));
    let lib = temp.path().join("library.sqlite");
    let conn = rusqlite::Connection::open(&lib).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert!(fails(
        &db,
        &home,
        json!({"op":"snapshot","view":"mine","per":25,"recent":15})
    )
    .contains("version 99"));
    let conn = rusqlite::Connection::open(lib).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
}

#[test]
fn migration_includes_live_wal_and_refuses_existing_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("legacy");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&old).unwrap();
    let source = old.join("index.sqlite");
    let conn = rusqlite::Connection::open(&source).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;CREATE TABLE proof(value TEXT);INSERT INTO proof VALUES('in WAL');").unwrap();
    let db = temp.path().join("new/index.sqlite");
    run(
        &db,
        &home,
        &["migrate-data", "--from", old.to_str().unwrap()],
    );
    let dest = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        dest.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
    let out = command(
        &db,
        &home,
        &["migrate-data", "--from", old.to_str().unwrap()],
        None,
    );
    assert!(!out.status.success());
    assert_eq!(
        dest.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
    assert_eq!(
        conn.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
}

#[test]
fn verbatim_folders_launch_browse_and_match_in_legacy_spelling() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let primary = home.join("primary");
    let sub = primary.join("sub");
    let extra = home.join("extra");
    for path in [&sub, &extra] {
        std::fs::create_dir_all(path).unwrap();
    }
    let path_env = fake_omp(&home);
    let (cwd, sub_cwd, extra_cwd) = (resolved(&primary), resolved(&sub), resolved(&extra));
    // Harnesses record the verbatim cwd that earlier launch plans passed.
    seed(&home, "verbatim", Path::new(&verbatim(&sub)), 100);
    run(&db, &home, &["index"]);
    // Both spellings name one folder.
    let id = create(
        &db,
        &home,
        "Verbatim",
        json!([verbatim(&primary), primary, verbatim(&extra)]),
    );
    let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
    for (position, folder) in [&primary, &extra].into_iter().enumerate() {
        conn.execute(
            "UPDATE project_folders SET path=?1 WHERE project_id=?2 AND position=?3",
            rusqlite::params![verbatim(folder), id.parse::<i64>().unwrap(), position as i64],
        )
        .unwrap();
    }

    let snap = snapshot(&db, &home);
    let saved = project(&snap, &id);
    assert_eq!(saved["folders"], json!([cwd, extra_cwd]));
    assert_eq!(uids(&saved["sessions"]), ["omp:verbatim"]);
    assert_eq!(saved["sessions"][0]["cwd"], json!(sub_cwd));
    assert_eq!(saved["sessions"][0]["resume"]["cwd"], json!(sub_cwd));

    let overlay = resolved(temp.path()).join("omp-new.yml");
    let plan = launch_plan(
        &db,
        &home,
        &path_env,
        json!({"op":"launch-plan","harness":"omp","project_id":id}),
    );
    assert_eq!(plan["cwd"], json!(cwd));
    assert_eq!(
        plan["argv"].as_array().unwrap()[1..],
        json!(["--cwd", cwd, "--config", overlay, "--add-dir", extra_cwd])
            .as_array()
            .unwrap()[..]
    );
    assert_eq!(plan["warnings"], json!([]));
    let plan = launch_plan(
        &db,
        &home,
        &path_env,
        json!({"op":"launch-plan","harness":"omp","cwd":verbatim(&primary)}),
    );
    assert_eq!(plan["cwd"], json!(cwd));

    let browsed = library(
        &db,
        &home,
        json!({"op":"folders","path":verbatim(&home),"cwd":"","offset":0,"limit":100}),
    );
    assert_eq!(browsed["path"], json!(resolved(&home)));
    assert_eq!(browsed["parent"], json!(resolved(temp.path())));
    let entry = |name: &str| {
        browsed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap()["path"]
            .clone()
    };
    assert_eq!(entry("primary"), json!(cwd));

    // A missing verbatim folder is still the configured folder; saving rewrites it.
    std::fs::remove_dir(&extra).unwrap();
    let edited = library(
        &db,
        &home,
        json!({"op":"project-save","id":id,"name":"Verbatim","icon":"folder","folders":saved["folders"]}),
    );
    assert_eq!(edited["project"]["folders"], json!([cwd, extra_cwd]));
    let stored: Vec<String> = conn
        .prepare("SELECT path FROM project_folders ORDER BY position")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        stored,
        [cwd.to_str().unwrap(), extra_cwd.to_str().unwrap()]
    );
}

#[test]
fn rename_persists_across_rebuild_syncs_fts_and_migrates_v1_library() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("cache/index.sqlite");
    let app = home.join("app");
    std::fs::create_dir_all(&app).unwrap();
    seed(&home, "r1", &app, 100);

    // Seed a legacy v1 library.sqlite before opening with v2 code.
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let lib_path = db.parent().unwrap().join("library.sqlite");
    let conn = rusqlite::Connection::open(&lib_path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE projects(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,icon TEXT NOT NULL,created_ms INTEGER NOT NULL);
        CREATE TABLE project_folders(project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,position INTEGER NOT NULL,path TEXT NOT NULL,PRIMARY KEY(project_id,position),UNIQUE(project_id,path));
        CREATE TABLE chat_state(uid TEXT PRIMARY KEY,assignment TEXT NOT NULL DEFAULT 'automatic',project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,pinned_ms INTEGER,archived_ms INTEGER);
        CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        INSERT INTO chat_state(uid,pinned_ms) VALUES('omp:r1', 123);
        PRAGMA user_version=1;
    ",
    )
    .unwrap();
    drop(conn);

    run(&db, &home, &["index"]);
    let snap = snapshot(&db, &home);
    assert_eq!(uids(&snap["pinned"]), ["omp:r1"]);
    assert_eq!(snap["recent"][0]["title"], "shared searchable needle");

    // Rename chat -> reflects in snapshot and FTS query, and survives index rebuild.
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:r1","title":"Custom Renamed Title"}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(snap["recent"][0]["title"], "Custom Renamed Title");
    assert_eq!(snap["recent"][0]["custom_title"], true);
    assert_eq!(
        uids(&run(&db, &home, &["query", "--q", "Renamed"])["sessions"]),
        ["omp:r1"]
    );

    std::fs::remove_file(&db).unwrap();
    run(&db, &home, &["index"]);
    let snap = snapshot(&db, &home);
    assert_eq!(snap["recent"][0]["title"], "Custom Renamed Title");
    assert_eq!(
        uids(&run(&db, &home, &["query", "--q", "Renamed"])["sessions"]),
        ["omp:r1"]
    );

    // Clearing title with null reverts to the original disk title in both snapshot and FTS.
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:r1","title":null}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(snap["recent"][0]["title"], "shared searchable needle");
    assert!(snap["recent"][0].get("custom_title").is_none());
    assert!(run(&db, &home, &["query", "--q", "Renamed"])["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn project_folder_changes_retain_chats_and_source_ignores_hide_everywhere() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let repo = home.join("repo");
    let sub = repo.join("sub");
    let other = home.join("other");
    for folder in [&sub, &other] {
        std::fs::create_dir_all(folder).unwrap();
    }
    seed(&home, "root", &repo, 100);
    seed(&home, "sub", &sub, 200);
    seed(&home, "other", &other, 300);
    run(&db, &home, &["index"]);

    let pid = create(&db, &home, "Repo", json!([repo]));
    assert_eq!(uids(&project(&snapshot(&db, &home), &pid)["sessions"]), ["omp:sub", "omp:root"]);

    // Moving project folders retains previously matched chats in `project_chats`.
    library(&db, &home, json!({"op":"project-save","id":pid,"name":"Repo","icon":"folder","folders":[other]}));
    assert_eq!(
        uids(&project(&snapshot(&db, &home), &pid)["sessions"]),
        ["omp:other", "omp:sub", "omp:root"]
    );

    // Ignore source folder hides matching chats and restores cleanly on unignore.
    library(&db, &home, json!({"op":"source-ignore","path":repo}));
    let snap = snapshot(&db, &home);
    assert_eq!(uids(&snap["recent"]), ["omp:other"]);
    assert_eq!(uids(&project(&snap, &pid)["sessions"]), ["omp:other"]);
    assert_eq!(run(&db, &home, &["query", "--q", "needle"])["total"], 1);

    library(&db, &home, json!({"op":"source-unignore","path":resolved(&repo)}));
    let after = snapshot(&db, &home);
    assert_eq!(after["ignored_sources"], json!([]));
    assert_eq!(run(&db, &home, &["query", "--q", "needle"])["total"], 3);

    // Also verify upstream v3 (with project_chats + ignored_sources, no custom_title) migrates cleanly to v4.
    let v3_dir = temp.path().join("v3_mig");
    std::fs::create_dir_all(&v3_dir).unwrap();
    let v3_db = v3_dir.join("index.sqlite");
    let v3_lib = v3_dir.join("library.sqlite");
    let c = rusqlite::Connection::open(&v3_lib).unwrap();
    c.execute_batch(
        "CREATE TABLE projects(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,icon TEXT NOT NULL,created_ms INTEGER NOT NULL);
         CREATE TABLE project_folders(project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,position INTEGER NOT NULL,path TEXT NOT NULL,PRIMARY KEY(project_id,position),UNIQUE(project_id,path));
         CREATE TABLE chat_state(uid TEXT PRIMARY KEY,assignment TEXT NOT NULL DEFAULT 'automatic',project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,pinned_ms INTEGER,archived_ms INTEGER);
         CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
         CREATE TABLE project_chats(project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,uid TEXT NOT NULL,PRIMARY KEY(project_id,uid));
         CREATE TABLE ignored_sources(path TEXT PRIMARY KEY,resolved TEXT NOT NULL);
         INSERT INTO projects VALUES(1,'V3Proj','folder',100);
         INSERT INTO project_folders VALUES(1,0,'/tmp/v3');
         INSERT INTO project_chats VALUES(1,'omp:root');
         PRAGMA user_version=3;"
    ).unwrap();
    drop(c);
    let snap_v3 = snapshot(&v3_db, &home);
    assert_eq!(snap_v3["projects"][0]["name"], "V3Proj");
    let c2 = rusqlite::Connection::open(&v3_lib).unwrap();
    assert_eq!(c2.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 4);
}
