use serde::Serialize;

/// A source record identifying one indexable unit (file or DB row).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub source: String,
    pub version: String,
}

/// A fully-parsed session ready for insertion.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub messages: i64,
    pub first_user: String,
    pub last_assistant: String,
    pub body: String,
    pub resume: Resume,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resume {
    pub kind: &'static str, // "tab" | "spawn"
    pub cwd: String,
    pub argv: Vec<String>,
    pub command: String,
    /// Command that forks this session into a new session ID (`omp --fork`, `claude --fork-session`, `codex fork`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fork_argv: Vec<String>,
    /// Every session id the chat is stored under when it spans several (an OMP chain); empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<String>,
}

impl Resume {
    pub fn tab(cwd: impl Into<String>, argv: Vec<String>) -> Self {
        let command = crate::text::shell_join(&argv);
        Resume {
            kind: "tab",
            cwd: cwd.into(),
            argv,
            command,
            fork: None,
            fork_argv: Vec::new(),
            sessions: Vec::new(),
        }
    }

    pub fn spawn(cwd: impl Into<String>, argv: Vec<String>) -> Self {
        let command = crate::text::shell_join(&argv);
        Resume {
            kind: "spawn",
            cwd: cwd.into(),
            argv,
            command,
            fork: None,
            fork_argv: Vec::new(),
            sessions: Vec::new(),
        }
    }

    pub fn with_fork(mut self, fork_argv: Vec<String>) -> Self {
        self.fork = Some(crate::text::shell_join(&fork_argv));
        self.fork_argv = fork_argv;
        self
    }
}

/// Populates `resume.fork` / `resume.fork_argv` for rows indexed before fork commands were stored.
pub fn fill_missing_fork(agent: &str, resume: &mut serde_json::Value) {
    let Some(obj) = resume.as_object_mut() else { return };
    if obj.get("fork").and_then(serde_json::Value::as_str).is_some_and(|s| !s.is_empty()) {
        return;
    }
    let Some(argv) = obj.get("argv").and_then(serde_json::Value::as_array) else { return };
    let args: Vec<&str> = argv.iter().filter_map(serde_json::Value::as_str).collect();
    let fork_argv: Option<Vec<String>> = match (agent, args.as_slice()) {
        ("omp", ["omp", "--resume", path]) if !path.is_empty() => {
            Some(vec!["omp".into(), "--fork".into(), (*path).into()])
        }
        ("claude", ["claude", "--resume", id]) if !id.is_empty() => {
            Some(vec!["claude".into(), "--resume".into(), (*id).into(), "--fork-session".into()])
        }
        ("codex", ["codex", "resume", id]) if !id.is_empty() => {
            Some(vec!["codex".into(), "fork".into(), (*id).into()])
        }
        _ => None,
    };
    if let Some(fa) = fork_argv {
        let cmd = crate::text::shell_join(&fa);
        obj.insert("fork".into(), serde_json::Value::String(cmd));
        obj.insert(
            "fork_argv".into(),
            serde_json::Value::Array(fa.into_iter().map(serde_json::Value::String).collect()),
        );
    }
}

/// Session as returned in JSON responses.
#[derive(Debug, Serialize)]
pub struct SessionJson {
    pub uid: String,
    pub agent: String,
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub custom_title: bool,
    pub cwd: String,
    pub project_root: String,
    pub project_label: String,
    pub worktree_root: String,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub messages: i64,
    pub last_assistant: String,
    pub snippet: Option<String>,
    pub resume: serde_json::Value,
    pub pinned: bool,
    pub archived: bool,
    pub assignment: Assignment,
}

#[derive(Debug, Clone, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// Compute a human-readable project label from a project_root path and a home dir.
pub fn project_label(root: &str, home: &str) -> String {
    if root.is_empty() {
        return "No project".to_string();
    }
    if root == home {
        return "~".to_string();
    }
    // basename
    root.rsplit('/').next().unwrap_or(root).to_string()
}
