use std::path::Path;
use anyhow::Context;
use rusqlite::{params, OpenFlags};
use crate::model::{Candidate, Parsed, Resume};
use crate::text::{clean_user, one_line, cap_body, prefix_fork_title};
use super::Adapter;

pub struct CodexAdapter;

impl Adapter for CodexAdapter {
    fn agent(&self) -> &'static str { "codex" }

    fn candidates(&mut self, home: &Path) -> anyhow::Result<Vec<Candidate>> {
        let state_db = find_versioned(home, ".codex", "state_", ".sqlite");
        let Some(state_path) = state_db else {
            // Fall back to rollout files.
            return rollout_candidates(home);
        };

        let conn = rusqlite::Connection::open_with_flags(
            &state_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening codex state db {}", state_path.display()))?;
        conn.execute_batch("PRAGMA busy_timeout=2000;")?;

        // Check for thread_source column (older schemas may not have it).
        let has_thread_source = has_column(&conn, "threads", "thread_source");

        let sql = if has_thread_source {
            "SELECT id, updated_at_ms, COALESCE(name,'') FROM threads \
             WHERE archived = 0 AND preview <> '' \
             AND source NOT LIKE '{%' \
             AND COALESCE(thread_source,'') <> 'subagent'"
        } else {
            "SELECT id, updated_at_ms, COALESCE(name,'') FROM threads \
             WHERE archived = 0 AND preview <> '' \
             AND source NOT LIKE '{%'"
        };

        let mut stmt = conn.prepare(sql)?;
        let out = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .flatten()
            .map(|(id, updated_ms, name)| Candidate {
                source: format!("codex:{id}"),
                version: if name.is_empty() { updated_ms.to_string() } else { format!("{updated_ms}:{name}") },
            })
            .collect();
        Ok(out)
    }

    fn parse(&mut self, home: &Path, c: &Candidate) -> anyhow::Result<Option<Parsed>> {
        let id = c.source.strip_prefix("codex:").unwrap_or(&c.source);

        // Check for rollout-only source (from rollout_candidates).
        if id.starts_with('/') || id.starts_with('~') {
            return parse_rollout_file(id, id);
        }

        let state_path = match find_versioned(home, ".codex", "state_", ".sqlite") {
            Some(p) => p,
            None => return rollout_parse_by_id(home, id),
        };
        let history_path = find_versioned(home, ".codex", "thread_history_", ".sqlite");

        let conn = rusqlite::Connection::open_with_flags(
            &state_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA busy_timeout=2000;")?;

        // Fetch thread row.
        let thread: Option<(String, String, Option<String>, Option<String>, Option<String>, i64, i64, Option<String>)> =
            conn.query_row(
                "SELECT COALESCE(name,''), COALESCE(title,''), cwd, git_branch, model, \
                 COALESCE(created_at_ms,0), COALESCE(updated_at_ms,0), rollout_path \
                 FROM threads WHERE id = ?1",
                params![id],
                |r| Ok((
                    r.get(0)?, r.get(1)?,
                    r.get(2)?, r.get(3)?, r.get(4)?,
                    r.get(5)?, r.get(6)?, r.get(7)?,
                )),
            )
            .ok();

        let Some((name, db_title, cwd_opt, git_branch, model_opt, created_ms, updated_ms, rollout_path)) = thread else {
            return Ok(None);
        };
        let cwd = cwd_opt.unwrap_or_default();

        // Try to read items from the history DB.
        let items = if let Some(hp) = &history_path {
            if let Ok(hconn) = rusqlite::Connection::open_with_flags(
                hp,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ) {
                let _ = hconn.execute_batch("PRAGMA busy_timeout=2000;");
                read_items(&hconn, id)
            } else {
                None
            }
        } else {
            None
        };

        let rollout_full = rollout_path.as_deref().filter(|rp| !rp.is_empty()).map(|rp| {
            if Path::new(rp).is_absolute() {
                rp.to_string()
            } else {
                home.join(rp).to_string_lossy().into_owned()
            }
        });
        let forked_from = rollout_full.as_deref().and_then(|p| read_rollout_forked_from(Path::new(p)));

        let (messages, first_user, last_assistant, body_text) = if let Some(items) = items {
            parse_items(items)
        } else if let Some(full) = &rollout_full {
            // Fall back to rollout file.
            if Path::new(full).is_file() {
                parse_rollout_items(full)?
            } else {
                // No items and no rollout; use preview from DB.
                let preview: String = conn.query_row(
                    "SELECT COALESCE(preview,'') FROM threads WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                ).unwrap_or_default();
                (0, String::new(), String::new(), preview)
            }
        } else {
            let preview: String = conn.query_row(
                "SELECT COALESCE(preview,'') FROM threads WHERE id=?1",
                params![id],
                |r| r.get(0),
            ).unwrap_or_default();
            (0, String::new(), String::new(), preview)
        };

        if first_user.is_empty() && messages == 0 {
            return Ok(None);
        }

        let first_user_for_title = if first_user.is_empty() {
            name.clone()
        } else {
            first_user.clone()
        };
        let raw_title = [name.as_str(), db_title.as_str()]
            .iter()
            .find(|s| !s.is_empty())
            .copied()
            .unwrap_or_else(|| first_user_for_title.as_str());
        let mut title = one_line(raw_title, 120);
        if let Some(parent_id) = forked_from {
            let same_name_as_parent = !name.is_empty()
                && conn
                    .query_row(
                        "SELECT COALESCE(name,'') FROM threads WHERE id = ?1",
                        params![parent_id],
                        |r| r.get::<_, String>(0),
                    )
                    .ok()
                    .is_some_and(|pn| pn == name);
            if name.is_empty() || same_name_as_parent {
                title = prefix_fork_title(&title);
            }
        }

        let mut body = format!("{}\n{}", title, body_text);
        cap_body(&mut body, 1_000_000);

        Ok(Some(Parsed {
            id: id.to_string(),
            title,
            cwd,
            branch: git_branch.filter(|s| !s.is_empty()),
            model: model_opt.filter(|s| !s.is_empty()),
            created_ms,
            updated_ms,
            messages,
            first_user,
            last_assistant,
            body,
            resume: Resume::tab(
                String::new(),
                vec!["codex".to_string(), "resume".to_string(), id.to_string()],
            )
            .with_fork(vec!["codex".to_string(), "fork".to_string(), id.to_string()]),
        }))
    }
}

// ── Items parsing ────────────────────────────────────────────────────────────

fn read_items(conn: &rusqlite::Connection, thread_id: &str) -> Option<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT item_type, item_json FROM thread_items \
             WHERE thread_id=?1 AND item_type IN ('userMessage','agentMessage') \
             ORDER BY rollout_ordinal",
        )
        .ok()?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![thread_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok()?
        .flatten()
        .collect();
    if rows.is_empty() { None } else { Some(rows) }
}

fn parse_items(items: Vec<(String, String)>) -> (i64, String, String, String) {
    let mut messages: i64 = 0;
    let mut first_user = String::new();
    let mut last_assistant = String::new();
    let mut body_parts: Vec<String> = Vec::new();

    for (item_type, item_json) in &items {
        let v: serde_json::Value = match serde_json::from_str(item_json) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if item_type == "userMessage" {
            let text = extract_codex_user_text(&v);
            if let Some(t) = text.and_then(|t| clean_user(&t)) {
                if first_user.is_empty() {
                    first_user = t.clone();
                }
                body_parts.push(t);
                messages += 1;
            }
        } else if item_type == "agentMessage" {
            let phase = v.get("phase").and_then(|p| p.as_str()).unwrap_or("");
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if !text.is_empty() {
                let is_commentary = phase == "commentary";
                if !is_commentary {
                    last_assistant = one_line(&text, 300);
                    messages += 1;
                }
                body_parts.push(text);
            }
        }
    }

    (messages, first_user, last_assistant, body_parts.join("\n"))
}

fn extract_codex_user_text(v: &serde_json::Value) -> Option<String> {
    let content = v.get("content")?;
    if let Some(arr) = content.as_array() {
        let parts: Vec<&str> = arr
            .iter()
            .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    None
}

// ── Rollout file parsing ─────────────────────────────────────────────────────

fn rollout_candidates(home: &Path) -> anyhow::Result<Vec<Candidate>> {
    let base = home.join(".codex/sessions");
    if !base.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    // Recursive search for rollout-*.jsonl files.
    collect_rollouts(&base, &mut out)?;
    Ok(out)
}

fn collect_rollouts(dir: &Path, out: &mut Vec<Candidate>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            let _ = collect_rollouts(&p, out);
        } else if name_str.starts_with("rollout-") && name_str.ends_with(".jsonl") {
            let meta = p.metadata()?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let version = format!("{}:{}", mtime, meta.len());
            let source = format!("codex:{}", p.display());
            out.push(Candidate { source, version });
        }
    }
    Ok(())
}

fn rollout_parse_by_id(home: &Path, _id: &str) -> anyhow::Result<Option<Parsed>> {
    // Can't map id → path without a DB; skip.
    let _ = home;
    Ok(None)
}

fn parse_rollout_file(source_id: &str, path_str: &str) -> anyhow::Result<Option<Parsed>> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    parse_rollout_items_str(source_id, &content, path)
}

fn parse_rollout_items(path_str: &str) -> anyhow::Result<(i64, String, String, String)> {
    let content = std::fs::read_to_string(path_str)?;
    let mut messages: i64 = 0;
    let mut first_user = String::new();
    let mut last_assistant = String::new();
    let mut body_parts: Vec<String> = Vec::new();

    for line in content.lines() {
        let v: serde_json::Value = match serde_json::from_str(line) { Ok(v) => v, Err(_) => continue };
        let rec_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if rec_type != "response_item" { continue; }
        let payload = match v.get("payload") { Some(p) => p, None => continue };
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") { continue; }
        let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "user" {
            let parts: Vec<&str> = payload.get("content")
                .and_then(|c| c.as_array())
                .map(|arr| arr.iter()
                    .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("input_text"))
                    .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                    .collect())
                .unwrap_or_default();
            let text = parts.join("\n");
            if let Some(t) = clean_user(&text) {
                if first_user.is_empty() { first_user = t.clone(); }
                body_parts.push(t);
                messages += 1;
            }
        } else if role == "assistant" {
            let parts: Vec<&str> = payload.get("output")
                .and_then(|o| o.as_array())
                .map(|arr| arr.iter()
                    .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("output_text"))
                    .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                    .collect())
                .unwrap_or_default();
            let text = parts.join("\n");
            if !text.is_empty() {
                last_assistant = one_line(&text, 300);
                body_parts.push(text);
                messages += 1;
            }
        }
    }
    Ok((messages, first_user, last_assistant, body_parts.join("\n")))
}

fn parse_rollout_items_str(source_id: &str, content: &str, _path: &Path) -> anyhow::Result<Option<Parsed>> {
    let mut id = String::new();
    let mut cwd = String::new();
    let mut forked_from_id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut messages: i64 = 0;
    let mut first_user = String::new();
    let mut last_assistant = String::new();
    let mut body_parts: Vec<String> = Vec::new();
    let mut created_ms: Option<i64> = None;
    let mut updated_ms: Option<i64> = None;

    for line in content.lines() {
        let v: serde_json::Value = match serde_json::from_str(line) { Ok(v) => v, Err(_) => continue };
        let rec_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if rec_type == "session_meta" {
            if let Some(payload) = v.get("payload") {
                if id.is_empty() {
                    id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                }
                if cwd.is_empty() {
                    cwd = payload.get("cwd").and_then(|v| v.as_str()).unwrap_or("").to_string();
                }
                if forked_from_id.is_none() {
                    forked_from_id = payload.get("forked_from_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
                }
            }
        } else if rec_type == "turn_context" {
            if let Some(m) = v.pointer("/payload/model").and_then(|v| v.as_str()) {
                if !m.is_empty() { model = Some(m.to_string()); }
            }
        } else if rec_type == "response_item" {
            let payload = match v.get("payload") { Some(p) => p, None => continue };
            if payload.get("type").and_then(|t| t.as_str()) != Some("message") { continue; }
            let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "developer" { continue; }
            let ts = v.get("timestamp").and_then(|t| t.as_str()).and_then(crate::text::iso_ms);
            if let Some(ms) = ts {
                if created_ms.map(|c| ms < c).unwrap_or(true) { created_ms = Some(ms); }
                if updated_ms.map(|u| ms > u).unwrap_or(true) { updated_ms = Some(ms); }
            }
            if role == "user" {
                let text: String = payload.get("content")
                    .and_then(|c| c.as_array())
                    .map(|arr| arr.iter()
                        .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("input_text"))
                        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                if let Some(t) = clean_user(&text) {
                    if first_user.is_empty() { first_user = t.clone(); }
                    body_parts.push(t);
                    messages += 1;
                }
            } else if role == "assistant" {
                let text: String = payload.get("output")
                    .and_then(|o| o.as_array())
                    .map(|arr| arr.iter()
                        .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("output_text"))
                        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                if !text.is_empty() {
                    last_assistant = one_line(&text, 300);
                    body_parts.push(text);
                    messages += 1;
                }
            }
        }
    }

    if first_user.is_empty() { return Ok(None); }
    if id.is_empty() { id = source_id.strip_prefix("codex:").unwrap_or(source_id).to_string(); }

    let mut title = one_line(&first_user, 120);
    if forked_from_id.is_some() {
        title = prefix_fork_title(&title);
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
    let mut body = format!("{}\n{}", title, body_parts.join("\n"));
    cap_body(&mut body, 1_000_000);

    Ok(Some(Parsed {
        id: id.clone(),
        title,
        cwd,
        branch: None,
        model,
        created_ms: created_ms.unwrap_or(now_ms),
        updated_ms: updated_ms.unwrap_or(now_ms),
        messages,
        first_user,
        last_assistant,
        body,
        resume: Resume::tab(String::new(), vec!["codex".to_string(), "resume".to_string(), id.clone()])
            .with_fork(vec!["codex".to_string(), "fork".to_string(), id]),
    }))
}

fn has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    conn.prepare(&sql)
        .ok()
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(1))
                .ok()
                .map(|rows| rows.flatten().any(|c| c == column))
        })
        .unwrap_or(false)
}

fn find_versioned(home: &Path, dir: &str, prefix: &str, suffix: &str) -> Option<std::path::PathBuf> {
    let base = home.join(dir);
    if !base.exists() {
        return None;
    }
    let entries = std::fs::read_dir(&base).ok()?;
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with(prefix) || !name_str.ends_with(suffix) {
            continue;
        }
        // Extract the numeric part between prefix and suffix.
        let num_str = &name_str[prefix.len()..name_str.len() - suffix.len()];
        if let Ok(n) = num_str.parse::<u64>() {
            if best.as_ref().map(|(b, _)| n > *b).unwrap_or(true) {
                best = Some((n, entry.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn read_rollout_forked_from(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    for line in BufReader::new(file).lines().take(4) {
        let Ok(line) = line else { break };
        if line.trim().is_empty() { continue; }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if v.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
            return v.pointer("/payload/forked_from_id")
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forked_codex_rollout_gets_prefix_unless_distinct_name() {
        let tmp = tempfile::tempdir().unwrap();
        let codex_dir = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let rollout_fork = codex_dir.join("r_fork.jsonl");
        std::fs::write(&rollout_fork, concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"f1\",\"cwd\":\"E:\\\\work\",\"forked_from_id\":\"p1\"}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-11-12T01:00:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Parent task\"}]}}\n",
        )).unwrap();
        let rollout_renamed = codex_dir.join("r_ren.jsonl");
        std::fs::write(&rollout_renamed, concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"f2\",\"cwd\":\"E:\\\\work\",\"forked_from_id\":\"p1\"}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-11-12T01:00:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Parent task\"}]}}\n",
        )).unwrap();

        let db = codex_dir.join("state_5.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY, rollout_path TEXT, created_at INTEGER, updated_at INTEGER,
                source TEXT, cwd TEXT, title TEXT, archived INTEGER, git_branch TEXT,
                model TEXT, created_at_ms INTEGER, updated_at_ms INTEGER, preview TEXT, name TEXT
            );"
        ).unwrap();
        conn.execute(
            "INSERT INTO threads VALUES ('p1', '', 1, 1, 'cli', 'E:\\\\work', 'Parent task', 0, '', '', 1000, 1000, 'Parent task', 'Parent Title')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads VALUES ('f1', ?1, 2, 2, 'cli', 'E:\\\\work', 'Parent task', 0, '', '', 2000, 2000, 'Parent task', '')",
            params![rollout_fork.to_string_lossy().as_ref()],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads VALUES ('f2', ?1, 3, 3, 'cli', 'E:\\\\work', 'Parent task', 0, '', '', 3000, 3000, 'Parent task', 'Custom Fork Name')",
            params![rollout_renamed.to_string_lossy().as_ref()],
        ).unwrap();
        drop(conn);

        let mut adapter = CodexAdapter;
        let cands = adapter.candidates(tmp.path()).unwrap();
        let mut titles = std::collections::HashMap::new();
        for c in cands {
            if let Some(p) = adapter.parse(tmp.path(), &c).unwrap() {
                titles.insert(p.id, p.title);
            }
        }
        assert_eq!(titles["f1"], "[Fork] Parent task");
        assert_eq!(titles["f2"], "Custom Fork Name");
    }
}
