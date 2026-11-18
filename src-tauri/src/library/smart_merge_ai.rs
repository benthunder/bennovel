//! The AI step of smart merge (`smart_merge.rs` finds candidates on the device first).
//!
//! The AI is sent everything the book knows about the entries (`Records`): their
//! translated and original names, aliases, how each chapter translated those original
//! names, a description and the relationships. It answers with groups of entries that
//! are one, the entry to keep and the final name to use (`Group`); one entry alone with a
//! better name is a group too. Nothing is merged until the user reviews the groups and
//! picks them (`apply_groups`): the entries go into the kept one, it takes the final name,
//! and every old name, as entries and chapters wrote it, goes in the replace rules.
//!
//! For one entry (`for_entry`), the entry, the candidates found on the device and the
//! closest names of its kind go in one request. For the whole wiki (`for_wiki`), the
//! groups found on the device and the other entries are packed, kind by kind, into
//! blocks of about `BLOCK` letters, never cutting an entry or a group, and the blocks run
//! `PARALLEL` at a time until all are done.

use super::genre_terms::{embed, fold};
use super::llm::{AiError, Chat};
use super::rules::Rule;
use super::smart_merge::{self, distance, safe_find, shown};
use super::wiki::{self, Entry, EntryInput, WIKI_LANG};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Letters in one request's list, about: a unit is never cut, so one may go over.
pub const BLOCK: usize = 10_000;
/// Requests at the same time for the whole wiki.
const PARALLEL: usize = 3;
/// A description is cut to this many letters.
const SUMMARY: usize = 300;
/// Relationships told per entry at most.
const RELATIONS: usize = 6;
/// Chapters told per translated spelling at most.
const CHAPTERS: usize = 4;
/// Entries of its kind with the closest names sent with one entry at most.
const FOR_ENTRY: usize = 40;

/// A group the AI found: `members` go into `into`, which takes `name`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub members: Vec<Entry>,
    pub into: Entry,
    pub name: String,
    /// The replace rules applying it adds (find → `name`).
    pub rules: Vec<Rule>,
    pub note: String,
}

/// A group the user kept, maybe with another final name.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPick {
    pub into_uid: String,
    #[serde(default)]
    pub from_uids: Vec<String>,
    pub name: String,
}

/// What the book knows about its entries, for the AI.
pub struct Records {
    all: Vec<Entry>,
    /// Original name → (spelling the translation used, chapters).
    found: HashMap<String, Vec<(String, Vec<i64>)>>,
    /// Entry uid → its relationships in words.
    rels: HashMap<String, Vec<String>>,
}

impl Records {
    pub fn load(conn: &Connection, novel_id: i64) -> rusqlite::Result<Self> {
        let all = wiki::entries(conn, novel_id)?;
        let mut found: HashMap<String, Vec<(String, Vec<i64>)>> = HashMap::new();
        for (raw, tr, chapter) in wiki::found_names(conn, novel_id, WIKI_LANG)? {
            let list = found.entry(raw).or_default();
            match list.iter_mut().find(|(t, _)| *t == tr) {
                Some((_, ch)) => {
                    if !ch.contains(&chapter) {
                        ch.push(chapter);
                    }
                }
                None => list.push((tr, vec![chapter])),
            }
        }
        let by: HashMap<&str, &Entry> = all.iter().map(|e| (e.uid.as_str(), e)).collect();
        let mut rels: HashMap<String, Vec<String>> = HashMap::new();
        for r in wiki::relations(conn, novel_id)? {
            let (Some(a), Some(b)) = (by.get(r.from_uid.as_str()), by.get(r.to_uid.as_str()))
            else {
                continue;
            };
            let words = r
                .labels
                .get(WIKI_LANG)
                .cloned()
                .unwrap_or_else(|| r.kind.replace('_', " "));
            rels.entry(a.uid.clone())
                .or_default()
                .push(format!("{words} {}", shown(b)));
            rels.entry(b.uid.clone())
                .or_default()
                .push(format!("{} {words} (mục này)", shown(a)));
        }
        Ok(Self { all, found, rels })
    }

    fn get(&self, uid: &str) -> Option<&Entry> {
        self.all.iter().find(|e| e.uid == uid)
    }

    /// The original names of an entry: its name and aliases.
    fn raws<'a>(&self, e: &'a Entry) -> impl Iterator<Item = &'a str> {
        std::iter::once(e.name.trim()).chain(e.aliases.iter().map(|a| a.trim()))
    }

    /// The spellings the chapters used for an entry's original names.
    fn spellings(&self, e: &Entry) -> Vec<(String, Vec<i64>)> {
        let mut out: Vec<(String, Vec<i64>)> = Vec::new();
        for raw in self.raws(e) {
            for (tr, ch) in self.found.get(raw).into_iter().flatten() {
                match out.iter_mut().find(|(t, _)| t == tr) {
                    Some((_, c)) => {
                        c.extend(ch.iter().filter(|x| !c.contains(x)).collect::<Vec<_>>())
                    }
                    None => out.push((tr.clone(), ch.clone())),
                }
            }
        }
        out
    }

    /// One entry's record as the AI reads it, numbered `n`.
    pub fn line(&self, n: usize, e: &Entry) -> String {
        let mut s = format!("[{n}] {} | gốc: {}", shown(e), e.name.trim());
        if !e.aliases.is_empty() {
            s.push_str(&format!(" | tên khác: {}", e.aliases.join(", ")));
        }
        let sp = self.spellings(e);
        if !sp.is_empty() {
            let told: Vec<String> = sp
                .iter()
                .map(|(t, ch)| {
                    let mut ch = ch.clone();
                    ch.sort();
                    let list: Vec<String> = ch.iter().take(CHAPTERS).map(i64::to_string).collect();
                    format!("{t} (chương {})", list.join(", "))
                })
                .collect();
            s.push_str(&format!(" | các chương đã dịch thành: {}", told.join("; ")));
        }
        let sum = e
            .summaries
            .get(WIKI_LANG)
            .or_else(|| e.summaries.values().next())
            .map(|s| s.trim())
            .unwrap_or("");
        if !sum.is_empty() {
            let cut: String = sum.chars().take(SUMMARY).collect();
            s.push_str(&format!(" | mô tả: {cut}"));
        }
        if let Some(r) = self.rels.get(&e.uid) {
            let r: Vec<&str> = r.iter().take(RELATIONS).map(String::as_str).collect();
            s.push_str(&format!(" | quan hệ: {}", r.join("; ")));
        }
        s
    }

    /// The replace rules for a group: every name its entries were written with (their
    /// translated names, original names, and the chapters' spellings) → `name`. A name
    /// another entry outside the group still uses, or one unsafe as plain text, is left out.
    pub fn rules(&self, members: &[&Entry], into: &Entry, name: &str) -> Vec<Rule> {
        let inside: Vec<&str> = members
            .iter()
            .map(|m| m.uid.as_str())
            .chain(std::iter::once(into.uid.as_str()))
            .collect();
        let taken = |f: &str| {
            self.all
                .iter()
                .any(|o| !inside.contains(&o.uid.as_str()) && (shown(o) == f || o.name.trim() == f))
        };
        let mut finds: Vec<String> = Vec::new();
        for e in members.iter().copied().chain(std::iter::once(into)) {
            let mut mine = vec![shown(e).to_string(), e.name.trim().to_string()];
            mine.extend(self.spellings(e).into_iter().map(|(t, _)| t));
            for f in mine {
                if !finds.contains(&f) {
                    finds.push(f);
                }
            }
        }
        let mut out: Vec<Rule> = finds
            .into_iter()
            .filter(|f| safe_find(f, name) && !taken(f))
            .map(|find| Rule {
                find,
                replace: name.to_string(),
            })
            .collect();
        // Longest first, so a name inside a longer one is replaced after it.
        out.sort_by_key(|r| std::cmp::Reverse(r.find.chars().count()));
        out
    }
}

/// The prompt: what a record says and the JSON to answer.
pub fn prompt(one: bool) -> String {
    let mut s = String::from(
        "Bạn là biên tập viên wiki của một bộ truyện dịch sang tiếng Việt. Người dùng gửi các mục wiki, mỗi dòng một mục: [số] tên dịch đang dùng | gốc: tên trong nguyên tác | tên khác | các chương đã dịch tên gốc thành: những cách bản dịch đã viết tên này và ở chương nào | mô tả | quan hệ.\n\
Nhiệm vụ:\n\
1. Tìm các mục thực chất là MỘT nhân vật / một nơi / một vật / một kỹ năng bị tách thành nhiều mục: cùng một tên viết khác (phiên âm Latin và Hán Việt, có hay không gạch nối, sai một chữ hay một dấu), hoặc tên thật và danh hiệu, biệt danh của cùng một người khi mô tả và quan hệ cho thấy rõ là một.\n\
2. Với mỗi nhóm, chọn mục giữ lại (\"keep\") và TÊN CUỐI CÙNG bản dịch phải dùng (\"name\"): tên tiếng Việt đúng và đầy đủ nhất; truyện bối cảnh Trung Hoa (kể cả võ hiệp Hàn Quốc) dùng tên Hán Việt thay cho phiên âm Latin; chọn cách viết được nhiều chương dùng nhất nếu đều đúng.\n\
3. Một mục đứng một mình mà tên dịch đang dùng sai hoặc còn để phiên âm Latin thì cũng trả về một nhóm chỉ có mục đó, với tên đúng.\n\
KHÔNG gộp: người thân cùng họ, thầy trò, hai người khác nhau có tên gần giống, một môn phái và chưởng môn của nó, một nơi và nơi nằm trong nó, hai mục khác loại. Không chắc thì bỏ qua.\n\
Chỉ trả về đúng một JSON, không giải thích thêm: {\"groups\":[{\"ids\":[số,...],\"keep\":số,\"name\":\"tên cuối cùng\",\"why\":\"lý do ngắn bằng tiếng Việt\"}]}. Không có gì thì trả {\"groups\":[]}.",
    );
    if one {
        s.push_str("\nChỉ trả về nhóm có mục số [1]; bỏ qua mọi nhóm khác.");
    }
    s
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    groups: Vec<AnswerGroup>,
}

#[derive(Deserialize)]
struct AnswerGroup {
    #[serde(default)]
    ids: Vec<usize>,
    #[serde(default)]
    keep: Option<usize>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    why: String,
}

fn parse(reply: &str) -> Option<Answer> {
    let (a, b) = (reply.find('{')?, reply.rfind('}')?);
    serde_json::from_str(reply.get(a..=b)?).ok()
}

/// Asks about one block of entries (numbered from 1 in order) and reads the groups back.
/// The user's own entry is the one kept; two of the user's are never merged; a group of
/// one entry counts only when its name changes.
fn ask(rec: &Records, llm: &dyn Chat, block: &[&Entry], one: bool) -> Result<Vec<Group>, AiError> {
    let list: Vec<String> = block
        .iter()
        .enumerate()
        .map(|(i, e)| rec.line(i + 1, e))
        .collect();
    let reply = llm.chat(&prompt(one), &list.join("\n"))?;
    let mut out = Vec::new();
    for g in parse(&reply).map(|a| a.groups).unwrap_or_default() {
        let name = g.name.trim().to_string();
        let mut ids: Vec<usize> = g
            .ids
            .into_iter()
            .filter(|i| *i >= 1 && *i <= block.len())
            .collect();
        ids.sort();
        ids.dedup();
        if name.is_empty() || ids.is_empty() || (one && !ids.contains(&1)) {
            continue;
        }
        let entries: Vec<&Entry> = ids.iter().map(|i| block[i - 1]).collect();
        if entries.iter().any(|e| e.kind != entries[0].kind) {
            continue;
        }
        let users: Vec<&&Entry> = entries.iter().filter(|e| e.source == "user").collect();
        if users.len() > 1 {
            continue;
        }
        let into: &Entry = match users.first() {
            Some(u) => u,
            None => g
                .keep
                .filter(|k| ids.contains(k))
                .map(|k| block[k - 1])
                .unwrap_or(entries[0]),
        };
        let members: Vec<&Entry> = entries.into_iter().filter(|e| e.uid != into.uid).collect();
        if members.is_empty() && name == shown(into) {
            continue;
        }
        out.push(Group {
            rules: rec.rules(&members, into, &name),
            members: members.into_iter().cloned().collect(),
            into: into.clone(),
            name,
            note: g.why.trim().to_string(),
        });
    }
    Ok(out)
}

/// The size of an entry's record, in letters.
fn size(rec: &Records, e: &Entry) -> usize {
    rec.line(0, e).chars().count() + 1
}

/// The AI's groups for one entry: the entry first, then the candidates found on the
/// device, then the closest names of its kind, as many as fit one block.
pub fn for_entry(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    uid: &str,
) -> Result<Vec<Group>, AiError> {
    let rec = Records::load(conn, novel_id)?;
    let Some(me) = rec.get(uid) else {
        return Ok(vec![]);
    };
    let local = smart_merge::for_entry(conn, novel_id, uid)?;
    let mut uids: Vec<String> = vec![me.uid.clone()];
    for p in &local.pairs {
        uids.push(p.from.uid.clone());
        uids.push(p.into.uid.clone());
    }
    for r in &local.raws {
        if let Some(e) = r.entry_uid.as_deref().and_then(|u| rec.get(u)) {
            if e.kind == me.kind {
                uids.push(e.uid.clone());
            }
        }
    }
    let v = embed(shown(me));
    let mut near: Vec<(&Entry, f32)> = rec
        .all
        .iter()
        .filter(|e| e.kind == me.kind && e.uid != me.uid)
        .map(|e| (e, distance(&v, &embed(shown(e)))))
        .collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    uids.extend(near.into_iter().take(FOR_ENTRY).map(|(e, _)| e.uid.clone()));
    let mut picked: Vec<&Entry> = Vec::new();
    for u in &uids {
        if let Some(e) = rec.get(u) {
            if !picked.iter().any(|p| p.uid == e.uid) {
                picked.push(e);
            }
        }
    }
    // As many as fit one block, the entry and the device's candidates first.
    let mut used = 0;
    let block: Vec<&Entry> = picked
        .into_iter()
        .enumerate()
        .take_while(|(i, e)| {
            used += size(&rec, e);
            *i == 0 || used <= BLOCK
        })
        .map(|(_, e)| e)
        .collect();
    ask(&rec, llm, &block, true)
}

/// The units of the whole wiki: each group the device found (entries joined by its
/// pairs, or by an original name a chapter translated as another entry's name) and each
/// other entry alone, by kind, then by name. A block holds many units.
fn units(rec: &Records, pairs: &[smart_merge::Suggestion]) -> Vec<Vec<usize>> {
    let index: HashMap<&str, usize> = rec
        .all
        .iter()
        .enumerate()
        .map(|(i, e)| (e.uid.as_str(), i))
        .collect();
    let mut parent: Vec<usize> = (0..rec.all.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    // Near names are not joined: a chain of them ("Tên 1", "Tên 2"…) would make one unit
    // of a whole kind.
    for s in pairs
        .iter()
        .filter(|s| s.reason != smart_merge::Reason::NearName)
    {
        if let (Some(&a), Some(&b)) = (
            index.get(s.from.uid.as_str()),
            index.get(s.into.uid.as_str()),
        ) {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            parent[ra] = rb;
        }
    }
    // As for one entry: an original name of B that a chapter translated as A's name
    // joins A and B (many sources, one target).
    let mut by_name: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, e) in rec.all.iter().enumerate() {
        by_name
            .entry((e.kind.clone(), smart_merge::key(shown(e))))
            .or_default()
            .push(i);
    }
    for (b, e) in rec.all.iter().enumerate() {
        for (tr, _) in rec.spellings(e) {
            for &a in by_name
                .get(&(e.kind.clone(), smart_merge::key(&tr)))
                .into_iter()
                .flatten()
            {
                if a != b {
                    let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
                    parent[ra] = rb;
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..rec.all.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort_by_key(|u| {
        let e = &rec.all[u[0]];
        (e.kind.clone(), fold(shown(e)))
    });
    out
}

/// Packs units into blocks of about `BLOCK` letters, one kind per block, never cutting a
/// unit. A block of one entry alone is still sent: its name may be wrong.
fn blocks(rec: &Records, units: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut used = 0;
    // A unit bigger than a block (rare: many entries written alike) is cut by name.
    let mut parts: Vec<Vec<usize>> = Vec::new();
    for u in units {
        let mut cur: Vec<usize> = Vec::new();
        let mut used = 0;
        for &i in u {
            let n = size(rec, &rec.all[i]);
            if !cur.is_empty() && used + n > BLOCK {
                parts.push(std::mem::take(&mut cur));
                used = 0;
            }
            cur.push(i);
            used += n;
        }
        parts.push(cur);
    }
    for u in &parts {
        let n: usize = u.iter().map(|&i| size(rec, &rec.all[i])).sum();
        let kind_changes = cur
            .first()
            .is_some_and(|&i| rec.all[i].kind != rec.all[u[0]].kind);
        if !cur.is_empty() && (kind_changes || used + n > BLOCK) {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.extend(u);
        used += n;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The AI's groups for the whole wiki, the blocks `PARALLEL` at a time. `progress` gets
/// (blocks done, blocks). A block that fails is left out when others answered; when
/// every block failed, the first error is returned.
pub fn for_wiki(
    conn: &Connection,
    llm: &dyn Chat,
    novel_id: i64,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<Vec<Group>, AiError> {
    let rec = Records::load(conn, novel_id)?;
    let pairs = smart_merge::suggest(conn, novel_id)?;
    let blocks = blocks(&rec, &units(&rec, &pairs));
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Result<Vec<Group>, AiError>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..PARALLEL.min(blocks.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(b) = blocks.get(i) else { break };
                let entries: Vec<&Entry> = b.iter().map(|&k| &rec.all[k]).collect();
                let r = ask(&rec, llm, &entries, false);
                results.lock().unwrap().push((i, r));
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(p) = progress {
                    p(d, blocks.len());
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    let mut out = Vec::new();
    let mut first_err = None;
    let mut answered = false;
    for (_, r) in results {
        match r {
            Ok(g) => {
                answered = true;
                out.extend(g);
            }
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        Some(e) if !answered => Err(e),
        _ => Ok(out),
    }
}

/// Applies the groups the user kept, after the pairs (`smart_merge::apply`) already
/// merged: each group's entries go into the kept one (following earlier merges), it
/// takes the final name, and its old names go in the replace rules.
pub fn apply_groups(
    conn: &Connection,
    novel_id: i64,
    picks: &[GroupPick],
    moved: &mut HashMap<String, String>,
) -> Result<smart_merge::Applied, AiError> {
    let follow = |moved: &HashMap<String, String>, uid: &str| {
        let mut u = uid.to_string();
        for _ in 0..64 {
            match moved.get(&u) {
                Some(n) => u = n.clone(),
                None => break,
            }
        }
        u
    };
    let mut done = smart_merge::Applied::default();
    for g in picks {
        let name = g.name.trim();
        let into_uid = follow(moved, &g.into_uid);
        if name.is_empty() || !wiki::by_uid_live(conn, &into_uid) {
            continue;
        }
        let rec = Records::load(conn, novel_id)?;
        let Some(into) = rec.get(&into_uid).cloned() else {
            continue;
        };
        let mut from: Vec<String> = Vec::new();
        for f in &g.from_uids {
            let f = follow(moved, f);
            if f != into_uid && wiki::by_uid_live(conn, &f) && !from.contains(&f) {
                from.push(f);
            }
        }
        let members: Vec<&Entry> = from.iter().filter_map(|f| rec.get(f)).collect();
        let rules = rec.rules(&members, &into, name);
        for f in &from {
            wiki::merge(conn, novel_id, f, &into_uid, true)?;
            moved.insert(f.clone(), into_uid.clone());
            done.merged += 1;
        }
        if shown(&into) != name {
            let kept = wiki::by_uid(conn, &into_uid)?.unwrap_or(into);
            let mut names = kept.names.clone();
            names.insert(WIKI_LANG.to_string(), name.to_string());
            wiki::save(
                conn,
                novel_id,
                EntryInput {
                    uid: Some(kept.uid.clone()),
                    kind: kept.kind.clone(),
                    name: kept.name.clone(),
                    names,
                    aliases: kept.aliases.clone(),
                    summaries: kept.summaries.clone(),
                    rank: kept.rank,
                    meta: kept.meta.clone(),
                },
            )?;
            done.renamed += 1;
        }
        done.rules += smart_merge::add_rules(conn, novel_id, &rules)?;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::rules;
    use crate::library::translate::tests::book;

    fn add(
        conn: &Connection,
        novel: i64,
        kind: &str,
        raw: &str,
        vi: &str,
        summary: &str,
    ) -> String {
        let mut names = BTreeMap::new();
        names.insert("vi".to_string(), vi.to_string());
        let mut summaries = BTreeMap::new();
        if !summary.is_empty() {
            summaries.insert("vi".to_string(), summary.to_string());
        }
        let e = wiki::save(
            conn,
            novel,
            EntryInput {
                uid: None,
                kind: kind.into(),
                name: raw.into(),
                names,
                aliases: vec![],
                summaries,
                rank: None,
                meta: Default::default(),
            },
        )
        .unwrap();
        conn.execute(
            "update wiki_entries set source = 'ai' where uid = ?1",
            [&e.uid],
        )
        .unwrap();
        e.uid
    }

    struct Judge {
        reply: String,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl Chat for Judge {
        fn chat(&self, system: &str, user: &str) -> Result<String, AiError> {
            self.sent.lock().unwrap().push((system.into(), user.into()));
            Ok(self.reply.clone())
        }
        fn model(&self) -> &str {
            "judge"
        }
    }

    fn judge(reply: &str) -> Judge {
        Judge {
            reply: reply.into(),
            sent: Default::default(),
        }
    }

    fn facts(conn: &Connection, chapter: i64, items: &[(&str, &str)]) {
        let entries: Vec<serde_json::Value> = items
            .iter()
            .map(|(n, t)| serde_json::json!({ "name": n, "translated": t }))
            .collect();
        let found = vec![serde_json::json!({ "entries": entries }).to_string()];
        conn.execute(
            "insert or replace into chapter_wiki_facts (chapter_id, lang, facts, created_at) values (?1, 'vi', ?2, 0)",
            rusqlite::params![chapter, serde_json::to_string(&found).unwrap()],
        )
        .unwrap();
    }

    #[test]
    fn one_entry_gets_its_group_and_final_name_and_everything_is_replaced() {
        let (conn, novel) = book("smart-ai-entry");
        let a = add(
            &conn,
            novel,
            "character",
            "Seol-Hwi",
            "Seol-Hwi",
            "Đệ tử Hoa Sơn.",
        );
        let b = add(&conn, novel, "character", "Seol Hwi", "Tuyết Hui", "");
        add(&conn, novel, "character", "Gu Dae-yeom", "Cừu Đại Diễm", "");
        // Chapter 1 wrote her "Tuyết Hy".
        facts(&conn, 10, &[("Seol-Hwi", "Tuyết Hy")]);
        let j = judge(
            r#"{"groups":[{"ids":[1,2],"keep":1,"name":"Tuyết Huy","why":"cùng một người"},{"ids":[3],"name":"Cừu Đại Diễm"}]}"#,
        );
        let g = for_entry(&conn, &j, novel, &a).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        assert_eq!(g[0].into.uid, a);
        assert_eq!(g[0].members.len(), 1);
        assert_eq!(g[0].name, "Tuyết Huy");
        // Everything the AI was told about the entry.
        let (system, user) = j.sent.lock().unwrap()[0].clone();
        assert!(system.contains("Chỉ trả về nhóm có mục số [1]"));
        let first = user.lines().next().unwrap();
        assert!(first.starts_with("[1] Seol-Hwi | gốc: Seol-Hwi"), "{first}");
        assert!(
            first.contains("các chương đã dịch thành: Tuyết Hy (chương 1)"),
            "{first}"
        );
        assert!(first.contains("mô tả: Đệ tử Hoa Sơn."), "{first}");

        let mut moved = HashMap::new();
        let done = apply_groups(
            &conn,
            novel,
            &[GroupPick {
                into_uid: a.clone(),
                from_uids: vec![b.clone()],
                name: "Tuyết Huy".into(),
            }],
            &mut moved,
        )
        .unwrap();
        assert_eq!((done.merged, done.renamed), (1, 1));
        let w = wiki::load(&conn, novel).unwrap();
        let kept = w.entries.iter().find(|e| e.uid == a).unwrap();
        assert_eq!(kept.names.get("vi").map(String::as_str), Some("Tuyết Huy"));
        assert!(!w.entries.iter().any(|e| e.uid == b));
        let list = rules::get(&conn, novel, "vi").unwrap().unwrap();
        for find in ["Seol-Hwi", "Seol Hwi", "Tuyết Hui", "Tuyết Hy"] {
            assert!(
                list.contains(&Rule {
                    find: find.into(),
                    replace: "Tuyết Huy".into()
                }),
                "{find}: {list:?}"
            );
        }
    }

    #[test]
    fn the_wiki_is_cut_in_whole_blocks_and_asked_in_parallel() {
        let (conn, novel) = book("smart-ai-wiki");
        // Long descriptions, so the characters need several blocks.
        let long = "Một mô tả dài. ".repeat(20);
        for i in 0..80 {
            add(
                &conn,
                novel,
                "character",
                &format!("Raw {i:02}"),
                &format!("Tên {i:02}"),
                &long,
            );
        }
        add(&conn, novel, "place", "Hua", "Hoa Sơn", "");
        let rec = Records::load(&conn, novel).unwrap();
        let pairs = smart_merge::suggest(&conn, novel).unwrap();
        let bl = blocks(&rec, &units(&rec, &pairs));
        assert!(bl.len() >= 3, "{}", bl.len());
        for b in &bl {
            let kinds: Vec<&str> = b.iter().map(|&i| rec.all[i].kind.as_str()).collect();
            assert!(kinds.iter().all(|k| *k == kinds[0]));
            let n: usize = b.iter().map(|&i| size(&rec, &rec.all[i])).sum();
            assert!(n <= BLOCK || b.len() == 1, "{n}");
        }
        // Every entry is sent once.
        let total: usize = bl.iter().map(Vec::len).sum();
        assert_eq!(total, 81);
        let j = judge(r#"{"groups":[]}"#);
        let seen = Mutex::new(Vec::new());
        let g = for_wiki(
            &conn,
            &j,
            novel,
            Some(&|d, n| seen.lock().unwrap().push((d, n))),
        )
        .unwrap();
        assert!(g.is_empty());
        assert_eq!(j.sent.lock().unwrap().len(), bl.len());
        assert_eq!(seen.lock().unwrap().len(), bl.len());
    }

    #[test]
    fn the_wiki_groups_sources_translated_as_one_target_before_cutting() {
        let (conn, novel) = book("smart-ai-units");
        let a = add(&conn, novel, "character", "Seol-Hwi", "Tuyết Huy", "");
        let b = add(&conn, novel, "character", "薛辉", "Tiết Huy", "");
        let c = add(&conn, novel, "character", "Gu", "Cừu Đại Diễm", "");
        // A chapter translated 薛辉 as "Tuyết Huy": B joins A.
        facts(&conn, 10, &[("薛辉", "Tuyết Huy")]);
        let rec = Records::load(&conn, novel).unwrap();
        let u = units(&rec, &[]);
        let uid = |i: usize| rec.all[i].uid.clone();
        let with_a = u.iter().find(|x| x.iter().any(|&i| uid(i) == a)).unwrap();
        assert!(with_a.iter().any(|&i| uid(i) == b));
        assert!(!with_a.iter().any(|&i| uid(i) == c));
    }
}
