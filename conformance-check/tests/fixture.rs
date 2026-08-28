use conformance_check::*;
use serde_json::{json, Value};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixture")
}

fn fixture_raw() -> String {
    read_log_raw(&fixture_dir()).expect("fixture log readable")
}

fn fixture_lines() -> Vec<String> {
    log_lines(&fixture_raw())
}

/// Locate a log line by parsed content instead of by index.
fn line_of(lines: &[String], pred: impl Fn(&Value) -> bool) -> usize {
    lines
        .iter()
        .position(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| pred(&v)))
        .expect("no line matches predicate")
}

fn fails_with(report: &CheckReport, needle: &str) -> bool {
    report.failures.iter().any(|f| f.contains(needle))
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Copy the fixture into a fresh git repo so checks run environment-
/// independently (check 4 walks THIS repo's history, not the project's).
fn fixture_in_temp_git() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new("cp")
        .arg("-R")
        .arg(fixture_dir().canonicalize().unwrap())
        .arg(dir.path().join("repo"))
        .status()
        .unwrap();
    assert!(status.success());
    let repo = dir.path().join("repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "fixture"]);
    dir
}

#[test]
fn fixture_passes_all_checks() {
    let dir = fixture_in_temp_git();
    for report in run_all(&dir.path().join("repo"), true) {
        assert!(
            report.passed(),
            "check {} ({}) failed: {:?}",
            report.number,
            report.name,
            report.failures
        );
    }
}

// ---- check 1: envelope ----

#[test]
fn envelope_rejects_unknown_field() {
    let mut lines = fixture_lines();
    lines[0] = lines[0].replace("\"causation_id\"", "\"extra\":1,\"causation_id\"");
    let report = check_envelope(&lines);
    assert!(
        fails_with(&report, "unknown top-level field `extra`"),
        "{report:?}"
    );
}

#[test]
fn envelope_rejects_missing_field() {
    let lines = vec![r#"{"id":"event_x","kind":"goal.created","payload":{}}"#.to_string()];
    let report = check_envelope(&lines);
    assert!(fails_with(&report, "missing envelope field"), "{report:?}");
}

#[test]
fn envelope_accepts_reordered_keys_and_nested_extras() {
    // structural equality is order-insensitive, and payload is opaque
    let lines = vec![
        r#"{"kind":"goal.created","id":"event_x","schema_version":1,"ts_ms":1,"actor":{"kind":"agent","id":"a"},"payload":{"id":"g","novel_nested_field":true},"causation_id":null,"correlation_id":null}"#.to_string(),
    ];
    assert!(check_envelope(&lines).passed());
}

// ---- check 2: replay + snapshots ----

fn write_snapshot(repo: &Path, name: &str, snapshot: &Value) {
    let dir = repo.join("snapshots").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("graph.json"),
        serde_json::to_vec(snapshot).unwrap(),
    )
    .unwrap();
}

fn valid_snapshot(repo: &Path, line_count: usize) -> Value {
    let raw = read_log_raw(repo).unwrap();
    let prefix: String = raw.split_inclusive('\n').take(line_count).collect();
    let lines = log_lines(&prefix);
    let events = parse_events(&lines).unwrap();
    let graph = yoagent_state::replay(&events).unwrap();
    let last_id = events.last().unwrap().id.as_str().to_string();
    json!({
        "graph": serde_json::to_value(&graph).unwrap(),
        "integrity": {
            "event_id": last_id,
            "line_count": line_count,
            "sha256": hex(&sha2::Sha256::digest(prefix.as_bytes())),
        }
    })
}

#[test]
fn snapshot_valid_passes_and_corruptions_fail() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    let raw = read_log_raw(&repo).unwrap();
    let events = parse_events(&log_lines(&raw)).unwrap();

    // positive: a correct snapshot at line 10 (boundary: whole log)
    write_snapshot(&repo, "event_10", &valid_snapshot(&repo, 10));
    let report = check_replay(&repo, &events, &raw);
    assert!(report.passed(), "{report:?}");

    // hash mismatch
    let mut bad = valid_snapshot(&repo, 5);
    bad["integrity"]["sha256"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    write_snapshot(&repo, "event_10", &bad);
    assert!(fails_with(
        &check_replay(&repo, &events, &raw),
        "integrity hash mismatch"
    ));

    // line_count out of range
    let mut bad = valid_snapshot(&repo, 5);
    bad["integrity"]["line_count"] = json!(0);
    write_snapshot(&repo, "event_10", &bad);
    assert!(fails_with(
        &check_replay(&repo, &events, &raw),
        "out of range"
    ));
    bad["integrity"]["line_count"] = json!(99);
    write_snapshot(&repo, "event_10", &bad);
    assert!(fails_with(
        &check_replay(&repo, &events, &raw),
        "out of range"
    ));

    // missing integrity fields are reported as what they are
    let mut bad = valid_snapshot(&repo, 5);
    bad["integrity"].as_object_mut().unwrap().remove("sha256");
    write_snapshot(&repo, "event_10", &bad);
    assert!(fails_with(
        &check_replay(&repo, &events, &raw),
        "missing `sha256`"
    ));

    // wrong terminal event id
    let mut bad = valid_snapshot(&repo, 5);
    bad["integrity"]["event_id"] = json!("event_99");
    write_snapshot(&repo, "event_10", &bad);
    assert!(fails_with(
        &check_replay(&repo, &events, &raw),
        "not the last event"
    ));

    // tampered graph diverges from the prefix fold
    let mut bad = valid_snapshot(&repo, 5);
    bad["graph"]["version"] = json!(999);
    write_snapshot(&repo, "event_10", &bad);
    assert!(fails_with(
        &check_replay(&repo, &events, &raw),
        "differs from folding"
    ));

    // a snapshot dir without graph.json is noted, not silently skipped
    std::fs::remove_file(repo.join("snapshots/event_10/graph.json")).unwrap();
    let report = check_replay(&repo, &events, &raw);
    assert!(report.passed());
    assert!(
        report.notes.iter().any(|n| n.contains("not verified")),
        "{report:?}"
    );
}

// ---- check 3: vocabulary + packs ----

#[test]
fn vocabulary_rejects_undeclared_kind() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_02");
    lines[i] = lines[i].replace("\"kind\":\"goal\"", "\"kind\":\"vibe\"");
    let events = parse_events(&lines).unwrap();
    let report = check_vocabulary(&fixture_dir(), &events);
    assert!(fails_with(&report, "undeclared kind `vibe`"), "{report:?}");
}

#[test]
fn pack_admits_custom_kind_and_malformed_pack_fails() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    let packs = repo.join(".agent/packs");
    std::fs::create_dir_all(&packs).unwrap();

    let mut lines = log_lines(&read_log_raw(&repo).unwrap());
    let i = line_of(&lines, |v| v["id"] == "event_02");
    lines[i] = lines[i].replace("\"kind\":\"goal\"", "\"kind\":\"experiment\"");
    let events = parse_events(&lines).unwrap();

    // without a pack: undeclared
    assert!(fails_with(
        &check_vocabulary(&repo, &events),
        "undeclared kind"
    ));

    // with a full pack declaring it: admitted
    std::fs::write(
        packs.join("custom.json"),
        json!({
            "id": "pack_custom", "name": "custom", "version": "1",
            "object_types": {"experiment": {"kind": "experiment", "required_props": [], "prop_docs": {}}},
            "relation_types": {}, "policies": [], "prompts": [], "settings": {}
        })
        .to_string(),
    )
    .unwrap();
    let report = check_vocabulary(&repo, &events);
    if !report.passed() {
        // The Pack schema requires fields exactly as yoagent-state defines them;
        // if the shape above drifts, this assertion tells us loudly.
        panic!("pack should admit `experiment`: {report:?}");
    }

    // a malformed pack is a failure, never silently ignored
    std::fs::write(packs.join("broken.json"), "{\"id\":").unwrap();
    assert!(fails_with(&check_vocabulary(&repo, &events), "broken.json"));
}

#[test]
fn vocabulary_rejects_malformed_ops_payload() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_02");
    lines[i] = lines[i].replace(
        "[{\"CreateNode\":{\"id\":\"goal_retry\",\"kind\":\"goal\",\"props\":{\"title\":\"Make retry reliable\",\"status\":\"Open\"}}}]",
        "{\"not\":\"ops\"}",
    );
    let events = parse_events(&lines).unwrap();
    let report = check_vocabulary(&fixture_dir(), &events);
    assert!(fails_with(&report, "does not parse as ops"), "{report:?}");
}

// ---- check 4: append-only ----

#[test]
fn append_only_rejects_history_rewrite_and_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("state")).unwrap();
    let log = repo.join("state/events.jsonl");

    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.name", "t"]);
    git(repo, &["config", "user.email", "t@t"]);
    std::fs::write(&log, "line one\nline two\n").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "one"]);

    // append-only extension is fine
    std::fs::write(&log, "line one\nline two\nline three\n").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "two"]);
    assert!(check_append_only(repo).passed());

    // working-tree edit of committed lines fails, before any commit
    std::fs::write(&log, "line ONE\nline two\nline three\n").unwrap();
    assert!(fails_with(&check_append_only(repo), "working tree edits"));

    // in-place edit committed fails
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "three"]);
    assert!(fails_with(&check_append_only(repo), "edits existing lines"));

    // deletion committed fails
    std::fs::remove_file(&log).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "four"]);
    assert!(fails_with(&check_append_only(repo), "deleted in commit"));
}

#[test]
fn append_only_covers_facts_glob_and_missing_worktree_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("knowledge")).unwrap();
    let facts = repo.join("knowledge/facts.jsonl");

    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.name", "t"]);
    git(repo, &["config", "user.email", "t@t"]);
    std::fs::write(&facts, "fact one\n").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "one"]);
    assert!(check_append_only(repo).passed());

    // a facts.jsonl OUTSIDE memory/ is still enforced (the */facts.jsonl glob)
    std::fs::write(&facts, "fact ONE\n").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "two"]);
    assert!(fails_with(
        &check_append_only(repo),
        "knowledge/facts.jsonl"
    ));

    // committed file missing from the working tree fails
    let dir2 = tempfile::tempdir().unwrap();
    let repo2 = dir2.path();
    std::fs::create_dir_all(repo2.join("state")).unwrap();
    git(repo2, &["init", "-q"]);
    git(repo2, &["config", "user.name", "t"]);
    git(repo2, &["config", "user.email", "t@t"]);
    std::fs::write(repo2.join("state/events.jsonl"), "e1\n").unwrap();
    git(repo2, &["add", "-A"]);
    git(repo2, &["commit", "-qm", "one"]);
    std::fs::remove_file(repo2.join("state/events.jsonl")).unwrap();
    assert!(fails_with(
        &check_append_only(repo2),
        "missing from working tree"
    ));
}

#[test]
fn append_only_fails_on_non_git_directory() {
    let dir = tempfile::tempdir().unwrap();
    let report = check_append_only(dir.path());
    assert!(fails_with(&report, "not a git repository"), "{report:?}");
}

// ---- check 5: causation ----

#[test]
fn causation_rejects_dangling_reference() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_10");
    lines[i] = lines[i].replace(
        "\"causation_id\":\"event_09\"",
        "\"causation_id\":\"event_99\"",
    );
    let events = parse_events(&lines).unwrap();
    assert!(fails_with(
        &check_causation(&events),
        "does not reference an earlier event"
    ));
}

#[test]
fn causation_rejects_bad_root_but_allows_ops_root() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_07"); // eval.finished
    lines[i] = lines[i].replace("\"causation_id\":\"event_06\"", "\"causation_id\":null");
    let events = parse_events(&lines).unwrap();
    assert!(fails_with(&check_causation(&events), "root event"));

    // an ops-only maintenance root is permitted by the pairing rule
    let lines = vec![
        r#"{"id":"event_m1","schema_version":1,"ts_ms":1,"actor":{"kind":"system","id":"s"},"kind":"state.ops_applied","payload":[{"MarkStale":{"id":"goal_old","reason":"maintenance"}}],"causation_id":null,"correlation_id":null}"#.to_string(),
    ];
    let events = parse_events(&lines).unwrap();
    assert!(check_causation(&events).passed());
}

#[test]
fn causation_rejects_duplicate_and_self_referencing_ids() {
    // duplicate id
    let mut lines = fixture_lines();
    let dup = lines[0].replace("event_01", "event_dup");
    lines.push(dup.clone());
    lines.push(dup);
    let events = parse_events(&lines).unwrap();
    assert!(fails_with(&check_causation(&events), "duplicate event id"));

    // self-reference: an event citing its own id must fail
    let lines = vec![
        r#"{"id":"event_x","schema_version":1,"ts_ms":1,"actor":{"kind":"agent","id":"a"},"kind":"goal.created","payload":{"id":"g"},"causation_id":"event_x","correlation_id":null}"#.to_string(),
    ];
    let events = parse_events(&lines).unwrap();
    assert!(fails_with(
        &check_causation(&events),
        "does not reference an earlier event"
    ));
}

// ---- check 6: restore ----

#[test]
fn restore_asserts_fixture_facts() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_10");
    lines[i] = lines[i].replace(
        ",{\"UpdateNode\":{\"id\":\"patch_9\",\"props\":{\"status\":\"Promoted\"}}}",
        "",
    );
    let events = parse_events(&lines).unwrap();
    let report = check_restore(&fixture_dir(), &events, true);
    assert!(
        fails_with(&report, "patch_9.status != Promoted"),
        "{report:?}"
    );
}

// ---- check 7: pairing ----

#[test]
fn pairing_rejects_missing_ops_event() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_02");
    lines.remove(i);
    let events = parse_events(&lines).unwrap();
    assert!(fails_with(
        &check_pairing(&events),
        "no paired state.ops_applied"
    ));
}

#[test]
fn pairing_rejects_status_contradiction_even_with_decoy() {
    // A decoy CORRECT pair inserted after the contradicting one must not
    // shadow the contradiction: all claimants are checked.
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_10");
    let correct = lines[i].clone();
    lines[i] = lines[i].replace(
        "{\"CreateNode\":{\"id\":\"decision_3\",\"kind\":\"decision\",\"props\":{\"status\":\"Approved\"}}}",
        "{\"CreateNode\":{\"id\":\"decision_3\",\"kind\":\"decision\",\"props\":{\"status\":\"Rejected\"}}}",
    );
    lines.push(correct.replace("event_10", "event_11"));
    let events = parse_events(&lines).unwrap();
    let report = check_pairing(&events);
    assert!(fails_with(&report, "status contradicts"), "{report:?}");
    assert!(fails_with(&report, "created by 2 ops events"), "{report:?}");
}

#[test]
fn pairing_rejects_wrong_created_kind() {
    let mut lines = fixture_lines();
    let i = line_of(&lines, |v| v["id"] == "event_02");
    lines[i] = lines[i].replace("\"kind\":\"goal\"", "\"kind\":\"task\"");
    let events = parse_events(&lines).unwrap();
    // `task` is baseline vocabulary, so ONLY pairing catches this
    assert!(fails_with(
        &check_pairing(&events),
        "created as kind `task`"
    ));
}

#[test]
fn pairing_rejects_ops_chained_to_ops() {
    let mut lines = fixture_lines();
    lines.push(
        r#"{"id":"event_x","schema_version":1,"ts_ms":1739000001000,"actor":{"kind":"agent","id":"evolve"},"kind":"state.ops_applied","payload":[{"MarkStale":{"id":"goal_retry","reason":"x"}}],"causation_id":"event_02","correlation_id":null}"#.to_string(),
    );
    let events = parse_events(&lines).unwrap();
    assert!(fails_with(
        &check_pairing(&events),
        "chained to another ops event"
    ));
}

// ---- run_all + CLI ----

#[test]
fn missing_log_is_a_conformance_failure_not_a_tool_error() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    let reports = run_all(dir.path(), false);
    assert_eq!(reports.len(), 7);
    assert!(!reports[0].passed(), "check 1 must fail on a missing log");
    assert!(reports.iter().filter(|r| !r.passed()).count() >= 5);
}

#[test]
fn cli_exit_codes() {
    let bin = env!("CARGO_BIN_EXE_conformance-check");
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");

    let ok = Command::new(bin)
        .arg(&repo)
        .arg("--fixture")
        .output()
        .unwrap();
    assert_eq!(
        ok.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ok.stdout)
    );

    // non-conformant repo -> 1
    let raw = std::fs::read_to_string(repo.join("state/events.jsonl")).unwrap();
    std::fs::write(
        repo.join("state/events.jsonl"),
        raw.replace("event_09", "event_99"),
    )
    .unwrap();
    let fail = Command::new(bin).arg(&repo).output().unwrap();
    assert_eq!(fail.status.code(), Some(1));

    // usage errors -> 2
    assert_eq!(Command::new(bin).output().unwrap().status.code(), Some(2));
    assert_eq!(
        Command::new(bin)
            .arg(&repo)
            .arg("--fixtrue")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2),
        "a typo'd flag must be a usage error, not a silently skipped assertion"
    );
    assert_eq!(
        Command::new(bin)
            .arg(&repo)
            .arg(&repo)
            .output()
            .unwrap()
            .status
            .code(),
        Some(2),
        "multiple repo paths must be a usage error, not last-wins"
    );
}

// ---- integration: the spec's headline claim ----

#[tokio::test]
async fn gitventstore_emitted_repo_passes_all_checks() {
    use yoagent_state::{
        init_agent_repo, ActorRef, Decision, DecisionId, DecisionStatus, EvalId, EvalResult,
        EvalStatus, Goal, GoalId, NodeId, PatchId, PatchStatus, RunId, StatePatch, YoAgentState,
    };
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let store = init_agent_repo(repo, "it-agent", "worker-it").unwrap();
    git(repo, &["config", "user.name", "t"]);
    git(repo, &["config", "user.email", "t@t"]);
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "init"]);

    let state = YoAgentState::load(store.clone()).await.unwrap();
    let actor = ActorRef::agent("it");
    state
        .record_goal(Goal::new(GoalId::new("goal_it"), "t", "s", actor.clone()))
        .await
        .unwrap();
    state
        .record_run_started(actor.clone(), RunId::new("run_it"), "task")
        .await
        .unwrap();
    let patch_id = state
        .propose_patch(StatePatch::new(
            PatchId::new("patch_it"),
            "t",
            "s",
            actor.clone(),
        ))
        .await
        .unwrap();
    state
        .record_eval(
            actor.clone(),
            EvalResult {
                id: EvalId::new("eval_it"),
                command: "test".into(),
                status: EvalStatus::Passed,
                score: Some(1.0),
                metadata: json!({}),
            },
            Some(patch_id.clone()),
        )
        .await
        .unwrap();
    state
        .record_decision_node(
            actor.clone(),
            Decision {
                id: DecisionId::new("decision_it"),
                status: DecisionStatus::Approved,
                reason: "r".into(),
                decided_by: actor.clone(),
                metadata: json!({}),
            },
            Some(NodeId::new("patch_it")),
        )
        .await
        .unwrap();
    state
        .update_patch_status(patch_id, PatchStatus::Promoted, None)
        .await
        .unwrap();
    state
        .record_run_finished(actor, RunId::new("run_it"), "promoted")
        .await
        .unwrap();
    store
        .commit_run(
            &RunId::new("run_it"),
            &GoalId::new("goal_it"),
            "promoted",
            &[],
        )
        .unwrap()
        .expect("boundary commit");

    for report in run_all(repo, false) {
        assert!(
            report.passed(),
            "emitted repo failed check {} ({}): {:?}",
            report.number,
            report.name,
            report.failures
        );
    }
}

/// A store with a dangling op is **conformant**, and the certificate says so.
///
/// The checker answers "can a conformant runtime fold and restore this store?".
/// Since yoagent-state 0.5.1 the answer for an op naming a missing node is
/// yes — the op is skipped and the fold completes (yologdev/yoagent#168).
///
/// Failing here would report non-conformance for a store every runtime can
/// restore, which inverts what the badge means. It is also the only outcome a
/// user could not act on: an append-only log cannot have the op removed, and
/// inserting a fix before it rewrites published history — permanently failing
/// check 4, the property the format exists to guarantee.
///
/// Regression for the real case: `yologdev/yoyo-gasp` at 8,882 events with one
/// dangling `UpdateNode`, which failed checks 2 and 6 before this.
#[test]
fn a_dangling_op_is_readable_and_reported_not_failed() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    let log = repo.join("state/events.jsonl");

    // Append an ops event updating a node nobody created — exactly the shape a
    // multi-process writer produces when the creating process ran an older
    // build.
    // Derive the envelope from a real event rather than inventing one, so this
    // tests the fold rather than the parser's required fields.
    let mut raw = std::fs::read_to_string(&log).unwrap();
    let template: Value = log_lines(&raw)
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["kind"] == "state.ops_applied")
        .expect("fixture has an ops event to model");
    let mut dangling = template.clone();
    dangling["id"] = json!("event_dangling_probe");
    dangling["ts_ms"] = json!(template["ts_ms"].as_i64().unwrap_or(0) + 1);
    dangling["payload"] =
        json!([{"UpdateNode": {"id": "node_never_created", "props": {"status": "closed"}}}]);
    raw.push_str(&serde_json::to_string(&dangling).unwrap());
    raw.push('\n');
    std::fs::write(&log, &raw).unwrap();

    let raw = read_log_raw(&repo).unwrap();
    let events = parse_events(&log_lines(&raw)).expect("log parses");
    let report = check_replay(&repo, &events, &raw);

    assert!(
        report.passed(),
        "a store a runtime can restore must certify as conformant; failures: {:?}",
        report.failures
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("UpdateNode") && n.contains("skipped")),
        "the skip must appear in the certificate — a report that hides it cannot be \
         audited, and masking corruption is the failure mode this must not become. \
         notes: {:?}",
        report.notes
    );
}
