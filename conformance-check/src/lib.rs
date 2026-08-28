//! GASP conformance checks (SPEC.md Part VI).
//!
//! Each check takes the parsed log (and/or the repo path) and returns a
//! `CheckReport`. `run_all` runs the seven checks in spec order. A checker's
//! cardinal rule: an internal error must never flip a FAIL into a PASS —
//! every unexpected condition fails closed.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use yoagent_state::{replay, replay_with_diagnostics, Event, Graph, Pack, StateOp};

pub const EVENTS_PATH: &str = "state/events.jsonl";

/// Append-only paths per Part I commit rule 1. `state/events.jsonl`,
/// `JOURNAL.md` variants, and every tracked `facts.jsonl` (any directory —
/// the spec's `*/facts.jsonl` glob) are checked when present.
const APPEND_ONLY_STATIC: &[&str] = &["state/events.jsonl", "journal/JOURNAL.md", "JOURNAL.md"];

const ENVELOPE_KEYS: &[&str] = &[
    "id",
    "schema_version",
    "ts_ms",
    "actor",
    "kind",
    "payload",
    "causation_id",
    "correlation_id",
];

const BASELINE_NODE_KINDS: &[&str] = &[
    "goal",
    "task",
    "run",
    "observation",
    "failure",
    "hypothesis",
    "patch",
    "eval",
    "decision",
    "model_call",
    "tool_call",
    "frame",
    "project_snapshot",
    "policy",
    "behavior",
];

const BASELINE_RELS: &[&str] = &[
    "serves",
    "blocks",
    "advances",
    "observes",
    "explains",
    "addresses",
    "modifies",
    "validated_by",
    "approved_by",
    "rejected_by",
    "produced_by",
    "derived_from",
    "depends_on",
    "supersedes",
    "contained_in_frame",
    "forked_from",
    "references",
];

/// Domain-event kinds that create an entity and therefore REQUIRE a paired
/// `state.ops_applied` (check 7), mapped to the node kind the pair must create.
/// This table is normative (SPEC.md check 7); raw-layer kinds (`run.started`,
/// `model.called`, `tool.called`, ...) MAY pair but are not required to.
const PAIRED_KINDS: &[(&str, &str)] = &[
    ("goal.created", "goal"),
    ("task.created", "task"),
    ("observation.created", "observation"),
    ("failure.observed", "failure"),
    ("hypothesis.created", "hypothesis"),
    ("patch.proposed", "patch"),
    ("eval.finished", "eval"),
    ("decision.created", "decision"),
    ("frame.created", "frame"),
];

#[derive(Debug)]
pub struct CheckReport {
    pub number: u8,
    pub name: &'static str,
    pub failures: Vec<String>,
    pub notes: Vec<String>,
}

impl CheckReport {
    fn new(number: u8, name: &'static str) -> Self {
        Self {
            number,
            name,
            failures: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn failed(number: u8, name: &'static str, reason: String) -> Self {
        let mut report = Self::new(number, name);
        report.failures.push(reason);
        report
    }

    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

pub fn read_log_raw(repo: &Path) -> Result<String, String> {
    let path = repo.join(EVENTS_PATH);
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

pub fn log_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

pub fn parse_events(lines: &[String]) -> Result<Vec<Event>, String> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str::<Event>(line).map_err(|e| format!("line {}: {e}", i + 1))
        })
        .collect()
}

enum OpsPayload {
    NotOps,
    Ops(Vec<StateOp>),
    Malformed(serde_json::Error),
}

fn ops_of(event: &Event) -> OpsPayload {
    if event.kind != "state.ops_applied" {
        return OpsPayload::NotOps;
    }
    match serde_json::from_value(event.payload.clone()) {
        Ok(ops) => OpsPayload::Ops(ops),
        Err(err) => OpsPayload::Malformed(err),
    }
}

/// Check 1 — envelope round-trip: exact top-level key set, parses to the
/// envelope, re-serializes structurally equal.
pub fn check_envelope(lines: &[String]) -> CheckReport {
    let mut report = CheckReport::new(1, "envelope round-trip");
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                report.failures.push(format!("line {n}: not JSON: {e}"));
                continue;
            }
        };
        let Some(obj) = value.as_object() else {
            report.failures.push(format!("line {n}: not a JSON object"));
            continue;
        };
        for key in ENVELOPE_KEYS {
            if !obj.contains_key(*key) {
                report
                    .failures
                    .push(format!("line {n}: missing envelope field `{key}`"));
            }
        }
        for key in obj.keys() {
            if !ENVELOPE_KEYS.contains(&key.as_str()) {
                report
                    .failures
                    .push(format!("line {n}: unknown top-level field `{key}`"));
            }
        }
        match serde_json::from_str::<Event>(line) {
            Ok(event) => {
                let reserialized = serde_json::to_value(&event).expect("event serializes");
                if reserialized != value {
                    report
                        .failures
                        .push(format!("line {n}: does not round-trip structurally"));
                }
            }
            Err(e) => report
                .failures
                .push(format!("line {n}: not a valid envelope: {e}")),
        }
    }
    report
}

/// Check 2 — replay: folding the log succeeds, and every snapshot equals the
/// fold of the log prefix named by its integrity record. The integrity hash
/// is SHA-256 over the RAW BYTES of the first `line_count` physical lines of
/// `state/events.jsonl` (including their newlines).
pub fn check_replay(repo: &Path, events: &[Event], raw_log: &str) -> CheckReport {
    let mut report = CheckReport::new(2, "replay");
    // Fold exactly as a conformant runtime does, not more strictly.
    //
    // This check asks "can a conformant runtime fold and restore this store?".
    // Since yoagent-state 0.5.1 the answer for a log containing an op that
    // references a missing node is *yes* — the op is skipped and the fold
    // completes. Failing here would report non-conformance for a store every
    // runtime can restore, which inverts what the badge means.
    //
    // But the skip is not swallowed: an op the graph cannot represent is worth
    // a reader's attention even when it is survivable, and a certificate that
    // hides it is not auditable. It lands in `notes`, which does not fail the
    // check.
    match replay_with_diagnostics(events) {
        Err(e) => {
            report.failures.push(format!("fold failed: {e}"));
            return report;
        }
        Ok((_graph, skipped)) => {
            for s in &skipped {
                // The reason already names the node, so do not repeat it.
                report.notes.push(format!(
                    "{} at batch index {} skipped — {}. The log is readable but malformed; a \
                     conformant runtime folds past this, and so does this check",
                    s.op, s.index, s.reason,
                ));
            }
        }
    }

    let snapshots = repo.join("snapshots");
    if !snapshots.is_dir() {
        report
            .notes
            .push("no snapshots/ — skipped seed check".into());
        return report;
    }
    let entries = match std::fs::read_dir(&snapshots) {
        Ok(entries) => entries,
        Err(e) => {
            report.failures.push(format!("cannot read snapshots/: {e}"));
            return report;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                report
                    .failures
                    .push(format!("cannot read snapshots/ entry: {e}"));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let graph_path = entry.path().join("graph.json");
        if !graph_path.is_file() {
            report
                .notes
                .push(format!("snapshot {name}: no graph.json — not verified"));
            continue;
        }
        let raw = match std::fs::read_to_string(&graph_path) {
            Ok(r) => r,
            Err(e) => {
                report
                    .failures
                    .push(format!("snapshot {name}: unreadable: {e}"));
                continue;
            }
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                report
                    .failures
                    .push(format!("snapshot {name}: not JSON: {e}"));
                continue;
            }
        };
        let (Some(graph_v), Some(integrity)) = (value.get("graph"), value.get("integrity")) else {
            report.failures.push(format!(
                "snapshot {name}: missing `graph` or `integrity` record"
            ));
            continue;
        };
        let Some(line_count) = integrity.get("line_count").and_then(Value::as_u64) else {
            report.failures.push(format!(
                "snapshot {name}: integrity record missing numeric `line_count`"
            ));
            continue;
        };
        let Some(expected_sha) = integrity.get("sha256").and_then(Value::as_str) else {
            report.failures.push(format!(
                "snapshot {name}: integrity record missing `sha256`"
            ));
            continue;
        };

        let prefix: String = raw_log
            .split_inclusive('\n')
            .take(line_count as usize)
            .collect();
        let physical_lines = raw_log.split_inclusive('\n').count() as u64;
        if line_count == 0 || line_count > physical_lines {
            report.failures.push(format!(
                "snapshot {name}: line_count {line_count} out of range (log has {physical_lines} lines)"
            ));
            continue;
        }
        let actual_sha = hex(&Sha256::digest(prefix.as_bytes()));
        if actual_sha != expected_sha {
            report.failures.push(format!(
                "snapshot {name}: integrity hash mismatch (log prefix changed?)"
            ));
            continue;
        }

        let prefix_lines = log_lines(&prefix);
        let prefix_events = match parse_events(&prefix_lines) {
            Ok(events) => events,
            Err(e) => {
                report
                    .failures
                    .push(format!("snapshot {name}: prefix does not parse: {e}"));
                continue;
            }
        };
        if let Some(event_id) = integrity.get("event_id").and_then(Value::as_str) {
            match prefix_events.last() {
                Some(last) if last.id.as_str() == event_id => {}
                _ => {
                    report.failures.push(format!(
                        "snapshot {name}: integrity event_id `{event_id}` is not the last event of the prefix"
                    ));
                    continue;
                }
            }
        }
        let snapshot_graph: Graph = match serde_json::from_value(graph_v.clone()) {
            Ok(g) => g,
            Err(e) => {
                report
                    .failures
                    .push(format!("snapshot {name}: graph does not parse: {e}"));
                continue;
            }
        };
        match replay(&prefix_events) {
            Ok(prefix_fold) if prefix_fold == snapshot_graph => {}
            Ok(_) => report.failures.push(format!(
                "snapshot {name}: snapshot differs from folding its log prefix"
            )),
            Err(e) => report
                .failures
                .push(format!("snapshot {name}: prefix fold failed: {e}")),
        }
    }
    report
}

/// Pack files plus per-file problems — a corrupt declaration is a defect of
/// the repo under test, never silently ignored.
fn load_packs(repo: &Path) -> (Vec<Pack>, Vec<String>) {
    let dir = repo.join(".agent/packs");
    if !dir.exists() {
        return (Vec::new(), Vec::new());
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => return (Vec::new(), vec![format!("cannot read .agent/packs/: {e}")]),
    };
    let mut packs = Vec::new();
    let mut problems = Vec::new();
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                problems.push(format!("cannot read .agent/packs/ entry: {e}"));
                continue;
            }
        };
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Pack>(&raw) {
                Ok(pack) => packs.push(pack),
                Err(e) => problems.push(format!("pack {name} does not parse as a Pack: {e}")),
            },
            Err(e) => problems.push(format!("pack {name} unreadable: {e}")),
        }
    }
    (packs, problems)
}

/// Check 3 — vocabulary: node kinds and relation kinds are baseline or
/// declared by a pack at `.agent/packs/*.json`. Malformed packs and malformed
/// ops payloads are failures, not skips.
pub fn check_vocabulary(repo: &Path, events: &[Event]) -> CheckReport {
    let mut report = CheckReport::new(3, "vocabulary");
    let (packs, problems) = load_packs(repo);
    report.failures.extend(problems);
    let mut kinds: HashSet<&str> = BASELINE_NODE_KINDS.iter().copied().collect();
    let mut rels: HashSet<&str> = BASELINE_RELS.iter().copied().collect();
    for pack in &packs {
        kinds.extend(pack.object_types.keys().map(String::as_str));
        rels.extend(pack.relation_types.keys().map(String::as_str));
    }
    for (i, event) in events.iter().enumerate() {
        let ops = match ops_of(event) {
            OpsPayload::NotOps => continue,
            OpsPayload::Malformed(e) => {
                report.failures.push(format!(
                    "line {}: state.ops_applied payload does not parse as ops: {e}",
                    i + 1
                ));
                continue;
            }
            OpsPayload::Ops(ops) => ops,
        };
        for op in ops {
            match op {
                StateOp::CreateNode { kind, id, .. } if !kinds.contains(kind.as_str()) => {
                    report.failures.push(format!(
                        "line {}: CreateNode {} uses undeclared kind `{kind}`",
                        i + 1,
                        id.as_str()
                    ));
                }
                StateOp::CreateRelation { rel, from, .. } if !rels.contains(rel.as_str()) => {
                    report.failures.push(format!(
                        "line {}: CreateRelation from {} uses undeclared rel `{rel}`",
                        i + 1,
                        from.as_str()
                    ));
                }
                _ => {}
            }
        }
    }
    report
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Check 4 — append-only-in-git: for every commit touching an append-only
/// path, the previous version must be a byte prefix of the next (additions at
/// EOF only), and the working tree must extend the last committed version.
/// This check is the spec's enforcement mechanism — every internal error
/// fails closed.
pub fn check_append_only(repo: &Path) -> CheckReport {
    let mut report = CheckReport::new(4, "append-only in git");
    if git(repo, &["rev-parse", "--git-dir"]).is_err() {
        report
            .failures
            .push("not a git repository (conformance rule 1: state lives in a git repo)".into());
        return report;
    }
    if git(repo, &["rev-parse", "--verify", "HEAD"]).is_err() {
        report
            .notes
            .push("no commits yet — nothing to violate".into());
        return report;
    }

    let mut paths: BTreeSet<String> = APPEND_ONLY_STATIC.iter().map(|s| s.to_string()).collect();
    // the spec's `*/facts.jsonl` glob, resolved against tracked files
    match git(repo, &["ls-files", "--", "facts.jsonl", "*/facts.jsonl"]) {
        Ok(listed) => paths.extend(listed.lines().map(str::to_string)),
        Err(e) => {
            report
                .failures
                .push(format!("cannot enumerate facts.jsonl paths: {e}"));
        }
    }

    for path in &paths {
        let shas = match git(
            repo,
            &[
                "rev-list",
                "--reverse",
                "--full-history",
                "HEAD",
                "--",
                path,
            ],
        ) {
            Ok(shas) => shas,
            Err(e) => {
                report
                    .failures
                    .push(format!("{path}: cannot walk history: {e}"));
                continue;
            }
        };
        let shas: Vec<&str> = shas.split_whitespace().collect();
        if shas.is_empty() {
            continue;
        }
        let spec = |sha: &str| format!("{sha}:./{path}");
        let mut prev: Option<String> = None;
        for sha in &shas {
            match git(repo, &["show", &spec(sha)]) {
                Ok(content) => {
                    if let Some(prev) = &prev {
                        if !content.starts_with(prev.as_str()) {
                            report.failures.push(format!(
                                "{path}: commit {} edits existing lines (not append-only)",
                                &sha[..7.min(sha.len())]
                            ));
                        }
                    }
                    prev = Some(content);
                }
                Err(_) => {
                    // Path touched but absent in this commit = deleted.
                    if prev.is_some() {
                        report.failures.push(format!(
                            "{path}: deleted in commit {}",
                            &sha[..7.min(sha.len())]
                        ));
                        prev = None;
                    }
                }
            }
        }
        if let Some(prev) = &prev {
            match std::fs::read_to_string(repo.join(path)) {
                Ok(on_disk) => {
                    if !on_disk.starts_with(prev.as_str()) {
                        report
                            .failures
                            .push(format!("{path}: working tree edits committed lines"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    report
                        .failures
                        .push(format!("{path}: committed file missing from working tree"));
                }
                Err(e) => {
                    report
                        .failures
                        .push(format!("{path}: working tree unreadable: {e}"));
                }
            }
        }
    }
    report
}

/// Check 5 — causation integrity: event ids are unique; every non-null
/// causation_id references an EARLIER event; roots (null causation) are
/// `*.created` / `*.started` events or ops-only `state.ops_applied`
/// maintenance events (permitted by the Part I pairing rule).
pub fn check_causation(events: &[Event]) -> CheckReport {
    let mut report = CheckReport::new(5, "causation integrity");
    let mut seen: HashSet<&str> = HashSet::new();
    for (i, event) in events.iter().enumerate() {
        let n = i + 1;
        match &event.causation_id {
            Some(cause) => {
                if !seen.contains(cause.as_str()) {
                    report.failures.push(format!(
                        "line {n}: causation_id {} does not reference an earlier event",
                        cause.as_str()
                    ));
                }
            }
            None => {
                let allowed_root = event.kind.ends_with(".created")
                    || event.kind.ends_with(".started")
                    || event.kind == "state.ops_applied";
                if !allowed_root {
                    report.failures.push(format!(
                        "line {n}: root event (null causation) has kind `{}`, expected *.created / *.started / state.ops_applied",
                        event.kind
                    ));
                }
            }
        }
        if !seen.insert(event.id.as_str()) {
            report.failures.push(format!(
                "line {n}: duplicate event id {} (ids must be unique for the causal order to be well-defined)",
                event.id.as_str()
            ));
        }
    }
    report
}

/// Check 6 — restore: the manifest and identity are present at their default
/// locations and the log folds. With `fixture_facts`, additionally asserts the
/// Part VI fixture graph. (Manifest-declared alternate locations are not yet
/// mechanically checked.)
pub fn check_restore(repo: &Path, events: &[Event], fixture_facts: bool) -> CheckReport {
    let mut report = CheckReport::new(6, "restore");
    if !repo.join("AGENT.md").is_file() {
        report.failures.push("AGENT.md manifest missing".into());
    }
    let identity_ok = repo.join("identity").is_dir() || repo.join("IDENTITY.md").is_file();
    if !identity_ok {
        report.failures.push(
            "no identity/ directory or IDENTITY.md (manifest-declared alternate locations are not yet checked)"
                .into(),
        );
    }
    let graph = match replay(events) {
        Ok(g) => g,
        Err(e) => {
            report.failures.push(format!("fold failed: {e}"));
            return report;
        }
    };
    if !fixture_facts {
        return report;
    }

    let node = |id: &str| graph.get_node(&yoagent_state::NodeId::new(id));
    let prop =
        |id: &str, key: &str| -> Option<Value> { node(id).and_then(|n| n.props.get(key)).cloned() };
    let edge = |from: &str, rel: &str, to: &str| -> bool {
        graph
            .outgoing(&yoagent_state::NodeId::new(from), Some(rel))
            .iter()
            .any(|r| r.to.as_str() == to)
    };

    if node("goal_retry").is_none() {
        report.failures.push("fixture: goal_retry missing".into());
    }
    if !edge("patch_9", "advances", "goal_retry") {
        report
            .failures
            .push("fixture: patch_9 --advances--> goal_retry missing".into());
    }
    if prop("patch_9", "status") != Some(Value::from("Promoted")) {
        report
            .failures
            .push("fixture: patch_9.status != Promoted".into());
    }
    if prop("patch_9", "references_commit") != Some(Value::from("abc1234")) {
        report
            .failures
            .push("fixture: patch_9.references_commit != abc1234".into());
    }
    if !edge("patch_9", "validated_by", "eval_5")
        || prop("eval_5", "status") != Some(Value::from("Passed"))
    {
        report
            .failures
            .push("fixture: patch_9 validated_by passed eval_5 missing".into());
    }
    if !edge("patch_9", "approved_by", "decision_3")
        || prop("decision_3", "status") != Some(Value::from("Approved"))
    {
        report
            .failures
            .push("fixture: patch_9 approved_by approved decision_3 missing".into());
    }
    report
}

/// Check 7 — domain↔ops consistency (the pairing rule): every entity-creating
/// domain event (the PAIRED_KINDS table) is materialized by EXACTLY ONE
/// `state.ops_applied` event whose causation_id names it and whose CreateNode
/// matches the payload's id, kind, and status. All claimants are checked —
/// a decoy pair cannot shadow a contradiction. Ops events must chain to
/// domain events, never to other ops events.
pub fn check_pairing(events: &[Event]) -> CheckReport {
    let mut report = CheckReport::new(7, "domain↔ops consistency");
    let paired: HashMap<&str, &str> = PAIRED_KINDS.iter().copied().collect();
    let by_id: HashMap<&str, &Event> = events.iter().map(|e| (e.id.as_str(), e)).collect();

    // ALL ops events indexed by the domain event they claim to apply
    let mut ops_for: HashMap<&str, Vec<&Event>> = HashMap::new();
    for event in events {
        if event.kind == "state.ops_applied" {
            if let Some(cause) = &event.causation_id {
                ops_for.entry(cause.as_str()).or_default().push(event);
            }
        }
    }

    for (i, event) in events.iter().enumerate() {
        let n = i + 1;
        let Some(expected_kind) = paired.get(event.kind.as_str()) else {
            continue;
        };
        let Some(entity_id) = event.payload.get("id").and_then(Value::as_str) else {
            report
                .failures
                .push(format!("line {n}: {} payload has no `id`", event.kind));
            continue;
        };
        let claimants = ops_for
            .get(event.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if claimants.is_empty() {
            report.failures.push(format!(
                "line {n}: {} `{entity_id}` has no paired state.ops_applied",
                event.kind
            ));
            continue;
        }
        let mut creators = 0;
        for ops_event in claimants {
            let ops = match ops_of(ops_event) {
                OpsPayload::Ops(ops) => ops,
                OpsPayload::Malformed(e) => {
                    report.failures.push(format!(
                        "line {n}: paired ops event {} has malformed payload: {e}",
                        ops_event.id.as_str()
                    ));
                    continue;
                }
                OpsPayload::NotOps => unreachable!("indexed by kind above"),
            };
            for op in ops {
                let StateOp::CreateNode { id, kind, props } = op else {
                    continue;
                };
                if id.as_str() != entity_id {
                    continue;
                }
                creators += 1;
                if kind != *expected_kind {
                    report.failures.push(format!(
                        "line {n}: `{entity_id}` created as kind `{kind}`, domain event implies `{expected_kind}`"
                    ));
                }
                if let (Some(evt_status), Some(node_status)) =
                    (event.payload.get("status"), props.get("status"))
                {
                    if evt_status != node_status {
                        report.failures.push(format!(
                            "line {n}: `{entity_id}` status contradicts domain event ({evt_status} vs {node_status})"
                        ));
                    }
                }
            }
        }
        if creators == 0 {
            report.failures.push(format!(
                "line {n}: paired ops for {} do not create node `{entity_id}`",
                event.kind
            ));
        } else if creators > 1 {
            report.failures.push(format!(
                "line {n}: `{entity_id}` created by {creators} ops events (pairing must be exactly one)"
            ));
        }
    }

    // Ops events must chain to domain events, never to other ops events.
    for event in events {
        if event.kind != "state.ops_applied" {
            continue;
        }
        if let Some(cause) = &event.causation_id {
            if let Some(domain) = by_id.get(cause.as_str()) {
                if domain.kind == "state.ops_applied" {
                    report.failures.push(format!(
                        "ops event {} chained to another ops event, not a domain event",
                        event.id.as_str()
                    ));
                }
            }
        }
    }
    report
}

/// Run all seven checks. A missing or unparseable log is a CONFORMANCE
/// failure (the log is the source of truth), never a tool error — dependent
/// checks are marked failed rather than silently skipped, and check 4 (which
/// needs only git) always runs.
pub fn run_all(repo: &Path, fixture_facts: bool) -> Vec<CheckReport> {
    let raw = match read_log_raw(repo) {
        Ok(raw) => raw,
        Err(e) => {
            let unavailable = |n: u8, name: &'static str| {
                CheckReport::failed(n, name, "log unavailable (see check 1)".into())
            };
            return vec![
                CheckReport::failed(1, "envelope round-trip", format!("{EVENTS_PATH}: {e}")),
                unavailable(2, "replay"),
                unavailable(3, "vocabulary"),
                check_append_only(repo),
                unavailable(5, "causation integrity"),
                unavailable(6, "restore"),
                unavailable(7, "domain↔ops consistency"),
            ];
        }
    };
    let lines = log_lines(&raw);
    let envelope = check_envelope(&lines);
    match parse_events(&lines) {
        Ok(events) => vec![
            envelope,
            check_replay(repo, &events, &raw),
            check_vocabulary(repo, &events),
            check_append_only(repo),
            check_causation(&events),
            check_restore(repo, &events, fixture_facts),
            check_pairing(&events),
        ],
        Err(_) => {
            let unparsed = |n: u8, name: &'static str| {
                CheckReport::failed(n, name, "log does not fully parse; see check 1".into())
            };
            vec![
                envelope,
                unparsed(2, "replay"),
                unparsed(3, "vocabulary"),
                check_append_only(repo),
                unparsed(5, "causation integrity"),
                unparsed(6, "restore"),
                unparsed(7, "domain↔ops consistency"),
            ]
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
