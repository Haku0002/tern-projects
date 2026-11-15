use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use serde_json::Value;
use crate::model::{Candidate, Parsed, Resume};
use crate::text::{clean_user, one_line, cap_body, prefix_fork_title, iso_ms};
use super::Adapter;

/// OMP moves a live session to a new file (a new session id, the history copied over) when the file is
/// open in another omp process, contested or replaced. The old file stays, so one conversation is several
/// files; omp's own session list follows the chain and shows it once. The adapter does the same: each
/// chain is listed as its most recently written file.
#[derive(Default)]
pub struct OmpAdapter {
    /// Listed source -> the chain it stands for, for chains of more than one file.
    chains: HashMap<String, Chain>,
    /// Listed source -> fork origin metadata when the chain's root has `parentSession`.
    forks: HashMap<String, ForkMeta>,
}

#[derive(Debug, Clone, Copy)]
struct ForkMeta {
    root_created_ms: Option<i64>,
    copied_from_parent: bool,
    definitely_new: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    /// The first session of the chain; the chat's uid, stable across later moves.
    pub root: String,
    /// Every session id in the chain (each file's own), sorted.
    pub ids: Vec<String>,
    /// Every file in the chain, the listed one included.
    pub files: Vec<PathBuf>,
}

impl Adapter for OmpAdapter {
    fn agent(&self) -> &'static str { "omp" }

    fn candidates(&mut self, home: &Path) -> anyhow::Result<Vec<Candidate>> {
        let (kept, chains, forks) = collapse_with_forks(candidates_at(home, ".omp/agent/sessions", "omp")?);
        self.chains = chains;
        self.forks = forks;
        Ok(kept)
    }

    fn parse(&mut self, _home: &Path, c: &Candidate) -> anyhow::Result<Option<Parsed>> {
        let fork = self.forks.get(&c.source).copied();
        let mut parsed = parse_omp_file(c, "omp", fork, |path| {
            Resume::tab(String::new(), vec!["omp".to_string(), "--resume".to_string(), path.to_string()])
                .with_fork(vec!["omp".to_string(), "--fork".to_string(), path.to_string()])
        })?;
        if let (Some(p), Some(chain)) = (parsed.as_mut(), self.chains.get(&c.source)) {
            // The window checks these against running omp processes before resuming the chat.
            p.resume.sessions = chain.ids.clone();
        }
        Ok(parsed)
    }

    fn uid_id(&self, c: &Candidate, parsed: &Parsed) -> String {
        self.chains.get(&c.source).map(|chain| chain.root.clone()).unwrap_or_else(|| parsed.id.clone())
    }
}

/// What the start of an OMP session file says about where it came from.
struct Head {
    id: String,
    continuation_of: Option<String>,
    parent: Option<String>,
    cache_key: bool,
    first_entry: Option<String>,
    /// True only when a pre-18.13 omp (< `continuationOf`) wrote new entries after this header's timestamp.
    /// Distinguishes an old omp's automatic rotation from `omp --fork`, which omits `continuationOf`.
    legacy_rotated: bool,
    session_ts: Option<i64>,
}

fn is_pre_18_13(ver: &str) -> bool {
    let mut parts = ver.split('.');
    let Some(maj) = parts.next().and_then(|s| s.parse::<u32>().ok()) else { return false };
    let Some(min) = parts.next().and_then(|s| s.parse::<u32>().ok()) else { return false };
    (maj, min) < (18, 13)
}

/// Reads the last JSON record of `file` by scanning the final 16 KiB, returning `(timestamp_ms, runtime_version)`.
fn read_tail_entry(mut file: &std::fs::File) -> (Option<i64>, Option<String>) {
    let Ok(len) = file.metadata().map(|m| m.len()) else { return (None, None) };
    let start = len.saturating_sub(16_384);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (None, None);
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return (None, None);
    }
    for line in String::from_utf8_lossy(&buf).lines().rev() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') { continue; }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else { continue };
        if matches!(v.get("type").and_then(Value::as_str), Some("session" | "title")) {
            continue;
        }
        let ts = get_ts(&v);
        let ver = v.get("runtime").and_then(|r| r.get("version")).and_then(Value::as_str).map(str::to_owned);
        return (ts, ver);
    }
    (None, None)
}

/// Reads the optional title slot, the session header and the first entry's id; nil without a header.
fn read_head(path: &Path) -> Option<Head> {
    let file = std::fs::File::open(path).ok()?;
    let mut head: Option<Head> = None;
    let mut header_ts: Option<i64> = None;
    for line in BufReader::new(&file).lines().take(4) {
        let Ok(line) = line else { break };
        if line.trim().is_empty() { continue; }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { break };
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        match head.as_mut() {
            None => match v.get("type").and_then(Value::as_str) {
                Some("title") => continue,
                Some("session") => {
                    header_ts = get_ts(&v);
                    head = Some(Head {
                        id: text("id")?,
                        continuation_of: text("continuationOf"),
                        parent: text("parentSession"),
                        cache_key: v.get("providerPromptCacheKey").is_some(),
                        first_entry: None,
                        legacy_rotated: false,
                        session_ts: header_ts,
                    });
                }
                _ => return None,
            },
            Some(h) => {
                h.first_entry = text("id");
                break;
            }
        }
    }
    if let Some(h) = head.as_mut() {
        if h.continuation_of.is_none() && h.parent.is_some() && h.cache_key {
            let (tail_ts, tail_ver) = read_tail_entry(&file);
            if let (Some(hts), Some(tts), Some(ver)) = (header_ts, tail_ts, tail_ver.as_deref()) {
                h.legacy_rotated = tts >= hts && is_pre_18_13(ver);
            }
        }
    }
    head
}

/// The file in the same folder that `heads[child]` was moved out of, if it is such a copy. omp ≥ 18.13 marks
/// automatic rotations with `continuationOf`, while `omp --fork` omits it so forks remain separate chats.
/// Pre-18.13 rotations lacked `continuationOf` and continued writing entries with a `< 18.13` runtime after
/// the rotated header's timestamp.
fn moved_from(heads: &[Option<Head>], ids: &HashMap<&str, usize>, child: usize) -> Option<usize> {
    let h = heads[child].as_ref()?;
    if let Some(from) = &h.continuation_of {
        return ids.get(from.as_str()).copied();
    }
    let parent = *ids.get(h.parent.as_deref()?)?;
    let copied = h.first_entry.is_some() && heads[parent].as_ref()?.first_entry == h.first_entry;
    (h.cache_key && copied && h.legacy_rotated).then_some(parent)
}

/// Lists each chain of moved session files once, as its most recently written file, whose version also
/// records the chain so the row is rewritten when the chain changes. Files that aren't moves pass through.
pub fn collapse_rotations(candidates: Vec<Candidate>) -> (Vec<Candidate>, HashMap<String, Chain>) {
    let (kept, chains, _) = collapse_with_forks(candidates);
    (kept, chains)
}

fn collapse_with_forks(candidates: Vec<Candidate>) -> (Vec<Candidate>, HashMap<String, Chain>, HashMap<String, ForkMeta>) {
    let path_of = |c: &Candidate| PathBuf::from(c.source.split_once(':').map_or(c.source.as_str(), |(_, p)| p));
    let dir_of = |c: &Candidate| path_of(c).parent().map(Path::to_path_buf).unwrap_or_default();
    let heads: Vec<Option<Head>> = candidates.iter().map(|c| read_head(&path_of(c))).collect();
    // Rotations stay in their folder, so session ids are looked up per folder.
    let mut by_dir: HashMap<PathBuf, HashMap<&str, usize>> = HashMap::new();
    for (i, h) in heads.iter().enumerate() {
        if let Some(h) = h {
            by_dir.entry(dir_of(&candidates[i])).or_default().insert(h.id.as_str(), i);
        }
    }
    let moved: Vec<Option<usize>> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| by_dir.get(&dir_of(c)).and_then(|folder| moved_from(&heads, folder, i)).filter(|&p| p != i))
        .collect();
    let root_of = |mut i: usize| {
        let mut steps = 0;
        while let Some(p) = moved[i] {
            i = p;
            steps += 1;
            if steps > moved.len() { break; }
        }
        i
    };
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..candidates.len() {
        groups.entry(root_of(i)).or_default().push(i);
    }
    let mtime = |c: &Candidate| c.version.split(':').next().and_then(|m| m.parse::<u128>().ok()).unwrap_or(0);
    let mut keep = vec![false; candidates.len()];
    let mut chains = HashMap::new();
    let mut forks = HashMap::new();
    let mut versions: HashMap<usize, String> = HashMap::new();
    for (root, members) in groups {
        let newest = *members
            .iter()
            .max_by(|&&a, &&b| mtime(&candidates[a]).cmp(&mtime(&candidates[b])).then_with(|| candidates[a].source.cmp(&candidates[b].source)))
            .expect("a group has a member");
        keep[newest] = true;
        if let Some(rh) = heads[root].as_ref() {
            if let Some(parent_id) = rh.parent.as_deref() {
                if rh.continuation_of.is_none() && !rh.legacy_rotated {
                    let parent_head = by_dir
                        .get(&dir_of(&candidates[root]))
                        .and_then(|f| f.get(parent_id))
                        .and_then(|&idx| heads[idx].as_ref());
                    let (copied_from_parent, definitely_new) = match parent_head {
                        Some(ph) => {
                            let copied = rh.first_entry.is_some() && ph.first_entry == rh.first_entry;
                            (copied, !copied)
                        }
                        None => (false, false),
                    };
                    forks.insert(candidates[newest].source.clone(), ForkMeta {
                        root_created_ms: rh.session_ts,
                        copied_from_parent,
                        definitely_new,
                    });
                }
            }
        }
        if members.len() < 2 {
            continue;
        }
        let mut chain_ids: Vec<String> = members.iter().filter_map(|&m| heads[m].as_ref().map(|h| h.id.clone())).collect();
        chain_ids.sort();
        let mut files: Vec<PathBuf> = members.iter().map(|&m| path_of(&candidates[m])).collect();
        files.sort();
        let root_id = heads[root].as_ref().map(|h| h.id.clone()).unwrap_or_default();
        versions.insert(newest, format!("{}|{}", candidates[newest].version, chain_ids.join(",")));
        chains.insert(candidates[newest].source.clone(), Chain { root: root_id, ids: chain_ids, files });
    }
    let kept = candidates
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(i, mut c)| {
            if let Some(v) = versions.remove(&i) { c.version = v; }
            c
        })
        .collect();
    (kept, chains, forks)
}

/// The chain the depth-two OMP session file `file` belongs to, every file in it included; nil when the file
/// stands alone. Used by permanent deletion, which removes the whole chain.
pub fn chain_of(file: &Path) -> anyhow::Result<Option<Chain>> {
    let dir = file.parent().ok_or_else(|| anyhow::anyhow!("Session file has no folder"))?;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_file() && p.extension().is_some_and(|e| e == "jsonl")
            && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            candidates.push(Candidate { source: format!("omp:{}", p.to_string_lossy()), version: String::new() });
        }
    }
    let (_, chains) = collapse_rotations(candidates);
    Ok(chains.into_values().find(|c| c.files.iter().any(|f| f == file)))
}

/// Shared depth-2 candidate listing for OMP and Pi.
pub fn candidates_at(home: &Path, rel: &str, agent: &str) -> anyhow::Result<Vec<Candidate>> {
    let base = home.join(rel);
    if !base.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry1 in std::fs::read_dir(&base)? {
        let entry1 = entry1?;
        if entry1.file_name().to_string_lossy().starts_with('.') { continue; }
        let p1 = entry1.path();
        if !p1.is_dir() { continue; }
        for entry2 in std::fs::read_dir(&p1)? {
            let entry2 = entry2?;
            let name = entry2.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') { continue; }
            let p2 = entry2.path();
            if !p2.is_file() { continue; }
            if p2.extension().map(|e| e != "jsonl").unwrap_or(true) { continue; }
            let meta = p2.metadata()?;
            let mtime = meta.modified().ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()).unwrap_or(0);
            let version = format!("{}:{}", mtime, meta.len());
            let source = format!("{}:{}", agent, p2.to_string_lossy());
            out.push(Candidate { source, version });
        }
    }
    Ok(out)
}

/// Shared JSONL parser for OMP-style files.
fn parse_omp_file<F>(
    c: &Candidate,
    agent: &str,
    fork_meta: Option<ForkMeta>,
    make_resume: F,
) -> anyhow::Result<Option<Parsed>>
where
    F: FnOnce(&str) -> Resume,
{
    let path_str = c.source.strip_prefix(&format!("{agent}:")).unwrap_or(&c.source);
    let path = Path::new(path_str);
    let content = std::fs::read_to_string(path)?;

    let mut id = String::new();
    let mut cwd = String::new();
    let mut session_ts: Option<i64> = None;
    let mut session_title: Option<String> = None;
    let mut first_title: Option<String> = None;
    let mut first_title_source: Option<String> = None;
    let mut first_title_updated_ms: Option<i64> = None;
    let mut last_title_change: Option<String> = None;
    let mut last_title_change_source: Option<String> = None;
    let mut last_title_change_ms: Option<i64> = None;
    let mut model: Option<String> = None;
    let mut created_ms: Option<i64> = None;
    let mut updated_ms: Option<i64> = None;
    let mut first_msg_ts: Option<i64> = None;
    let mut first_user: Option<String> = None;
    let mut last_assistant = String::new();
    let mut body_parts: Vec<String> = Vec::new();
    let mut messages: i64 = 0;
    let mut last_user_ms: Option<i64> = None;

    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Timestamp tracking.
        let ts_ms = get_ts(&v);
        if let Some(ms) = ts_ms {
            if created_ms.map(|c| ms < c).unwrap_or(true) { created_ms = Some(ms); }
            if updated_ms.map(|u| ms > u).unwrap_or(true) { updated_ms = Some(ms); }
        }

        let rec_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        if rec_type == "session" {
            id = v.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            cwd = v.get("cwd").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if session_ts.is_none() { session_ts = ts_ms; }
            if let Some(t) = v.get("title").and_then(|v| v.as_str()) {
                if !t.is_empty() { session_title = Some(t.to_string()); }
            }
        } else if rec_type == "title" {
            // First `title` record wins.
            if first_title.is_none() {
                if let Some(t) = v.get("title").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        first_title = Some(t.to_string());
                        first_title_source = v.get("source").and_then(Value::as_str).map(str::to_owned);
                        first_title_updated_ms = v.get("updatedAt").and_then(Value::as_str).and_then(iso_ms);
                    }
                }
            }
        } else if rec_type == "title_change" {
            if let Some(t) = v.get("title").and_then(|v| v.as_str()) {
                if !t.is_empty() {
                    last_title_change = Some(t.to_string());
                    last_title_change_source = v.get("source").and_then(Value::as_str).map(str::to_owned);
                    last_title_change_ms = ts_ms;
                }
            }
        } else if rec_type == "model_change" {
            if let Some(m) = v.get("model").and_then(|v| v.as_str()) {
                if !m.is_empty() { model = Some(m.to_string()); }
            }
        } else if rec_type == "message" {
            if let Some(ms) = ts_ms {
                if first_msg_ts.map(|m| ms < m).unwrap_or(true) { first_msg_ts = Some(ms); }
            }
            // OMP nests role+content under a "message" sub-object.
            let msg = v.get("message").unwrap_or(&v);
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            if role == "user" {
                let text = extract_content_text(msg);
                if let Some(t) = text.and_then(|t| clean_user(&t)) {
                    if first_user.is_none() { first_user = Some(t.clone()); }
                    if ts_ms > last_user_ms { last_user_ms = ts_ms; }
                    body_parts.push(t);
                    messages += 1;
                }
            } else if role == "assistant" {
                let text = extract_assistant_parts(msg);
                if !text.is_empty() {
                    last_assistant = one_line(&text, 300);
                    body_parts.push(text);
                    messages += 1;
                }
            }
        }
    }

    if first_user.is_none() { return Ok(None); }
    let first_user = first_user.unwrap();

    // Title priority: newer title_change > first_title > last_title_change > session_title > fallback.
    let change_is_newer = matches!((last_title_change_ms, first_title_updated_ms), (Some(tc), Some(ft)) if tc > ft);
    let raw_title = if change_is_newer {
        last_title_change
            .or(first_title)
            .or(session_title)
            .unwrap_or_else(|| one_line(&first_user, 80))
    } else {
        first_title
            .or(last_title_change)
            .or(session_title)
            .unwrap_or_else(|| one_line(&first_user, 80))
    };
    let mut title = one_line(&raw_title, 120);

    if let Some(fk) = fork_meta {
        if !fk.definitely_new {
            let root_ms = fk.root_created_ms.or(session_ts);
            let is_fork = fk.copied_from_parent
                || matches!((first_msg_ts, root_ms), (Some(m), Some(r)) if m < r);
            let renamed_after_fork = root_ms.is_some_and(|r| {
                let title_renamed = first_title_source.as_deref() == Some("user")
                    && first_title_updated_ms.is_some_and(|t| t > r + 1500);
                let change_renamed = last_title_change_source.as_deref().unwrap_or("user") == "user"
                    && last_title_change_ms.is_some_and(|t| t > r + 1500);
                title_renamed || change_renamed
            });
            // A fork nobody wrote in holds only its parent's copied history: listing it would show the
            // same chat twice. It appears once a message is sent in it.
            if is_fork && !matches!((last_user_ms, root_ms), (Some(u), Some(r)) if u >= r) {
                return Ok(None);
            }
            if is_fork && !renamed_after_fork {
                title = prefix_fork_title(&title);
            }
        }
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;

    // updated_ms fallback: file mtime (already in the source version string).
    let upd = updated_ms.unwrap_or_else(|| {
        path.metadata().ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(now_ms)
    });

    let mut body = format!("{}\n{}", title, body_parts.join("\n"));
    cap_body(&mut body, 1_000_000);

    let resume = make_resume(path_str);

    if id.is_empty() {
        id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }

    Ok(Some(Parsed {
        id,
        title,
        cwd,
        branch: None,
        model,
        created_ms: created_ms.unwrap_or(now_ms),
        updated_ms: upd,
        messages,
        first_user,
        last_assistant,
        body,
        resume,
    }))
}

fn get_ts(v: &serde_json::Value) -> Option<i64> {
    // Try numeric ms first, then ISO string.
    if let Some(n) = v.get("timestamp").and_then(|t| t.as_i64()) {
        return Some(n);
    }
    if let Some(s) = v.get("timestamp").and_then(|t| t.as_str()) {
        return crate::text::iso_ms(s);
    }
    None
}

fn extract_content_text(v: &serde_json::Value) -> Option<String> {
    let content = v.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        let parts: Vec<&str> = arr.iter()
            .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}

fn extract_assistant_parts(v: &serde_json::Value) -> String {
    let content = match v.get("content") { Some(c) => c, None => return String::new() };
    if let Some(arr) = content.as_array() {
        let parts: Vec<&str> = arr.iter()
            .filter(|i| i.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
            .collect();
        return parts.join("\n");
    }
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a session file: title slot, header (`extra` JSON fields), then entries with the given ids.
    fn session(dir: &Path, name: &str, id: &str, extra: &str, entries: &[&str], mtime_s: u64) -> Candidate {
        let mut text = format!("{{\"type\":\"title\",\"v\":1,\"title\":\"t\"}}\n{{\"type\":\"session\",\"version\":3,\"id\":\"{id}\"{extra}}}\n");
        for e in entries {
            text.push_str(&format!("{{\"type\":\"message\",\"id\":\"{e}\",\"message\":{{\"role\":\"user\",\"content\":\"hi\"}}}}\n"));
        }
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        Candidate { source: format!("omp:{}", path.display()), version: format!("{}:1", mtime_s * 1000) }
    }

    #[test]
    fn moved_sessions_list_once_and_forks_stay_separate() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let a = session(d, "a.jsonl", "A", "", &["e1", "e2"], 1);
        // omp >= 18.13 move of A, then a move of that.
        let b = session(d, "b.jsonl", "B", ",\"parentSession\":\"A\",\"continuationOf\":\"A\",\"providerPromptCacheKey\":\"A\"", &["e1", "e2", "e3"], 3);
        let c = session(d, "c.jsonl", "C", ",\"parentSession\":\"B\",\"continuationOf\":\"B\",\"providerPromptCacheKey\":\"A\"", &["e1", "e2", "e3", "e4"], 2);
        // `/new` from A: a parent link, a cache key, but none of A's history.
        let fresh = session(d, "n.jsonl", "N", ",\"parentSession\":\"A\",\"providerPromptCacheKey\":\"A\"", &["x1"], 4);
        // A branch of A: A's history, no inherited cache key.
        let branch = session(d, "f.jsonl", "F", ",\"parentSession\":\"A\"", &["e1"], 5);
        // `omp --fork` of A (omp >= 18.13): parent link, inherited cache key, copied history, NO continuationOf -> stays separate!
        let forked = session(d, "k.jsonl", "K", ",\"timestamp\":\"2026-11-12T01:00:00Z\",\"parentSession\":\"A\",\"providerPromptCacheKey\":\"A\"", &["e1", "e2"], 8);
        // A pre-18.13 omp rotation (no continuationOf, continued writing with runtime < 18.13 after header timestamp).
        let old_path = d.join("o.jsonl");
        std::fs::write(&old_path, concat!(
            "{\"type\":\"title\",\"v\":1,\"title\":\"t\"}\n",
            "{\"type\":\"session\",\"version\":3,\"id\":\"O\",\"timestamp\":\"2026-10-24T10:00:00Z\",\"parentSession\":\"Q\",\"providerPromptCacheKey\":\"Q\"}\n",
            "{\"type\":\"message\",\"id\":\"q1\",\"timestamp\":\"2026-10-24T09:00:00Z\",\"runtime\":{\"version\":\"18.12.6\"},\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
            "{\"type\":\"message\",\"id\":\"q2\",\"timestamp\":\"2026-10-24T10:01:00Z\",\"runtime\":{\"version\":\"18.12.6\"},\"message\":{\"role\":\"user\",\"content\":\"more\"}}\n",
        )).unwrap();
        let old = Candidate { source: format!("omp:{}", old_path.display()), version: "7000:1".into() };
        let q = session(d, "q.jsonl", "Q", "", &["q1"], 6);

        let (kept, chains) = collapse_rotations(vec![a, b, c.clone(), fresh.clone(), branch.clone(), forked.clone(), old.clone(), q]);
        let mut names: Vec<String> = kept.iter().map(|k| Path::new(&k.source).file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        // The A chain shows once (b), old Q->O rotation shows once (o), while `omp --fork` (k), branch (f) and `/new` (n) stay separate.
        assert_eq!(names, ["b.jsonl", "f.jsonl", "k.jsonl", "n.jsonl", "o.jsonl"]);
        let ab = &chains[&kept.iter().find(|k| k.source.ends_with("b.jsonl")).unwrap().source];
        assert_eq!(ab.root, "A");
        assert_eq!(ab.ids, ["A", "B", "C"]);
        assert_eq!(chains[&old.source].ids, ["O", "Q"]);
        assert_eq!(chains[&old.source].root, "Q");
        assert!(!chains.contains_key(&fresh.source) && !chains.contains_key(&branch.source) && !chains.contains_key(&forked.source));
        // The version names the chain, so a file joining it re-parses the row.
        assert!(kept.iter().find(|k| k.source.ends_with("b.jsonl")).unwrap().version.ends_with("|A,B,C"));
        // Deletion finds the whole chain from any of its files.
        let found = chain_of(Path::new(c.source.strip_prefix("omp:").unwrap())).unwrap().unwrap();
        assert_eq!(found.files.len(), 3);
    }

    #[test]
    fn unrenamed_forks_get_fork_prefix_and_renamed_or_new_do_not() {
        let tmp = tempfile::tempdir().unwrap();
        let sess = tmp.path().join(".omp/agent/sessions/proj");
        std::fs::create_dir_all(&sess).unwrap();

        std::fs::write(sess.join("parent.jsonl"), concat!(
            "{\"type\":\"title\",\"v\":1,\"title\":\"Unity Dev\",\"source\":\"user\",\"updatedAt\":\"2026-11-10T01:00:00Z\"}\n",
            "{\"type\":\"session\",\"version\":3,\"id\":\"P1\",\"timestamp\":\"2026-11-10T01:00:00Z\",\"cwd\":\"E:\\\\work\"}\n",
            "{\"type\":\"message\",\"id\":\"e1\",\"timestamp\":\"2026-11-10T01:00:05Z\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
        )).unwrap();

        // Unrenamed `omp --fork`: inherited title written within 2ms of session creation.
        std::fs::write(sess.join("fork_unrenamed.jsonl"), concat!(
            "{\"type\":\"title\",\"v\":1,\"title\":\"Unity Dev\",\"source\":\"user\",\"updatedAt\":\"2026-11-12T03:39:39.472Z\"}\n",
            "{\"type\":\"session\",\"version\":3,\"id\":\"F1\",\"timestamp\":\"2026-11-12T03:39:39.470Z\",\"cwd\":\"E:\\\\work\",\"parentSession\":\"P1\",\"providerPromptCacheKey\":\"P1\",\"title\":\"Unity Dev\"}\n",
            "{\"type\":\"message\",\"id\":\"e1\",\"timestamp\":\"2026-11-10T01:00:05Z\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
            "{\"type\":\"message\",\"id\":\"f1\",\"timestamp\":\"2026-11-12T03:40:00Z\",\"message\":{\"role\":\"user\",\"content\":\"go on\"}}\n",
        )).unwrap();

        // A fork opened and closed without a message: only the parent's copied history.
        std::fs::write(sess.join("fork_empty.jsonl"), concat!(
            "{\"type\":\"title\",\"v\":1,\"title\":\"Unity Dev\",\"source\":\"user\",\"updatedAt\":\"2026-11-12T06:00:00.002Z\"}\n",
            "{\"type\":\"session\",\"version\":3,\"id\":\"F3\",\"timestamp\":\"2026-11-12T06:00:00Z\",\"cwd\":\"E:\\\\work\",\"parentSession\":\"P1\",\"providerPromptCacheKey\":\"P1\",\"title\":\"Unity Dev\"}\n",
            "{\"type\":\"message\",\"id\":\"e1\",\"timestamp\":\"2026-11-10T01:00:05Z\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
            "{\"type\":\"custom\",\"customType\":\"session_exit\",\"id\":\"z1\",\"timestamp\":\"2026-11-12T06:00:09Z\"}\n",
        )).unwrap();

        // User-renamed fork: title updated > 1.5s after fork creation.
        std::fs::write(sess.join("fork_renamed.jsonl"), concat!(
            "{\"type\":\"title\",\"v\":1,\"title\":\"Custom Branch Name\",\"source\":\"user\",\"updatedAt\":\"2026-11-12T04:00:00Z\"}\n",
            "{\"type\":\"session\",\"version\":3,\"id\":\"F2\",\"timestamp\":\"2026-11-12T03:39:39Z\",\"cwd\":\"E:\\\\work\",\"parentSession\":\"P1\",\"providerPromptCacheKey\":\"P1\"}\n",
            "{\"type\":\"message\",\"id\":\"e1\",\"timestamp\":\"2026-11-10T01:00:05Z\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
            "{\"type\":\"message\",\"id\":\"f2\",\"timestamp\":\"2026-11-12T04:01:00Z\",\"message\":{\"role\":\"user\",\"content\":\"branch work\"}}\n",
        )).unwrap();

        // `/new` session: parentSession present, fresh entry id and later timestamp -> NOT a fork.
        std::fs::write(sess.join("new_session.jsonl"), concat!(
            "{\"type\":\"title\",\"v\":1,\"title\":\"Fresh Chat\",\"source\":\"auto\",\"updatedAt\":\"2026-11-12T05:00:00Z\"}\n",
            "{\"type\":\"session\",\"version\":3,\"id\":\"N1\",\"timestamp\":\"2026-11-12T05:00:00Z\",\"cwd\":\"E:\\\\work\",\"parentSession\":\"P1\",\"providerPromptCacheKey\":\"P1\"}\n",
            "{\"type\":\"message\",\"id\":\"x1\",\"timestamp\":\"2026-11-12T05:00:10Z\",\"message\":{\"role\":\"user\",\"content\":\"new work\"}}\n",
        )).unwrap();

        let mut adapter = OmpAdapter::default();
        let candidates = adapter.candidates(tmp.path()).unwrap();
        let mut titles: HashMap<String, String> = HashMap::new();
        for c in candidates {
            if let Some(p) = adapter.parse(tmp.path(), &c).unwrap() {
                titles.insert(p.id, p.title);
            }
        }
        assert_eq!(titles["P1"], "Unity Dev");
        assert_eq!(titles["F1"], "[Fork] Unity Dev");
        assert_eq!(titles["F2"], "Custom Branch Name");
        assert_eq!(titles["N1"], "Fresh Chat");
        // The parent already shows everything the empty fork holds.
        assert!(!titles.contains_key("F3"));
    }
}
