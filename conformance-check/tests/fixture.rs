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

/// Rewrite the manifest's identity_hash line: `Some(digest)` replaces it,
/// `None` removes it.
fn set_manifest_identity_hash(repo: &Path, digest: Option<&str>) {
    let manifest = std::fs::read_to_string(repo.join("AGENT.md")).unwrap();
    let rewritten: Vec<String> = manifest
        .lines()
        .filter_map(|l| {
            if l.trim_start().starts_with("identity_hash:") {
                digest.map(|d| format!("identity_hash: {d}"))
            } else {
                Some(l.to_string())
            }
        })
        .collect();
    std::fs::write(repo.join("AGENT.md"), rewritten.join("\n") + "\n").unwrap();
}

#[test]
fn restore_verifies_the_fixture_identity_hash() {
    let events = parse_events(&fixture_lines()).unwrap();
    let report = check_restore(&fixture_dir(), &events, false);
    assert!(report.passed(), "{report:?}");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("identity hash verified")),
        "{report:?}"
    );
}

#[test]
fn restore_rejects_tampered_identity_bytes() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    let id = repo.join("identity/IDENTITY.md");
    let mut bytes = std::fs::read(&id).unwrap();
    bytes.extend_from_slice(b"\nsynthetic-tamper\n");
    std::fs::write(&id, bytes).unwrap();
    let events = parse_events(&fixture_lines()).unwrap();
    let report = check_restore(&repo, &events, false);
    assert!(
        fails_with(&report, "identity bytes do not match the manifest"),
        "{report:?}"
    );
}

#[test]
fn restore_rejects_malformed_identity_hash() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    set_manifest_identity_hash(&repo, Some("not-a-digest"));
    let events = parse_events(&fixture_lines()).unwrap();
    let report = check_restore(&repo, &events, false);
    assert!(fails_with(&report, "malformed"), "{report:?}");
}

#[test]
fn restore_rejects_missing_identity_hash() {
    // Was a note while the reference emitter predated digests; yoagent-state
    // 0.5.3 writes them at init, so an undeclared digest now fails closed.
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    set_manifest_identity_hash(&repo, None);
    let events = parse_events(&fixture_lines()).unwrap();
    let report = check_restore(&repo, &events, false);
    assert!(
        fails_with(&report, "declares no identity_hash"),
        "{report:?}"
    );
}

#[test]
fn identity_hash_recipe_covers_all_files_in_byte_order() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("identity/sub")).unwrap();
    std::fs::write(repo.join("identity/b.md"), "bee").unwrap();
    std::fs::write(repo.join("identity/sub/a.md"), "ay").unwrap();
    // Independent restatement of the recipe as one concatenated stream:
    // path, newline, bytes — "identity/b.md" sorts before "identity/sub/a.md".
    let expected = hex(&sha2::Sha256::digest(
        b"identity/b.md\nbeeidentity/sub/a.md\nay",
    ));
    assert_eq!(compute_identity_hash(repo).unwrap(), expected);
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

// ---- the Log Segments extension (extensions/LOG_SEGMENTS.md) ----

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", message]);
}

fn declare_segments(repo: &Path) {
    let manifest = std::fs::read_to_string(repo.join("AGENT.md")).unwrap();
    let declared = manifest.replacen("spec_version: 1\n", "spec_version: 1\nlog_segments: 1\n", 1);
    assert_ne!(manifest, declared, "the fixture manifest changed shape");
    std::fs::write(repo.join("AGENT.md"), declared).unwrap();
}

/// The fixture with its log split after line `at`: segment 1 committed
/// first, then sealed by segment 2 in a second commit.
fn segmented_fixture(at: usize) -> tempfile::TempDir {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    // A fresh history whose first commit holds only segment 1's lines.
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    let raw = fixture_raw();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    std::fs::write(repo.join(EVENTS_PATH), lines[..at].concat()).unwrap();
    declare_segments(&repo);
    commit_all(&repo, "segment 1");
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    std::fs::write(repo.join(segment_path(2)), lines[at..].concat()).unwrap();
    commit_all(&repo, "seal segment 1, start segment 2");
    dir
}

#[test]
fn segmented_fixture_passes_all_checks() {
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    assert_eq!(log_segments(&repo).unwrap(), vec![EVENTS_PATH.to_string(), segment_path(2)]);
    assert_eq!(read_log_raw(&repo).unwrap(), fixture_raw(), "the logical log is the concatenation");
    for report in run_all(&repo, true) {
        assert!(report.passed(), "check {} ({}) failed: {:?}", report.number, report.name, report.failures);
    }
}

#[test]
fn segments_without_the_declaration_fail_check_1() {
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    // Undeclared: check 1 fails (the log is unavailable), never a silent fold of segment 1.
    let manifest = std::fs::read_to_string(repo.join("AGENT.md")).unwrap();
    std::fs::write(repo.join("AGENT.md"), manifest.replace("log_segments: 1\n", "")).unwrap();
    let reports = run_all(&repo, false);
    assert!(fails_with(&reports[0], "does not declare"), "{:?}", reports[0]);
}

#[test]
fn a_gap_in_segment_numbers_fails() {
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    std::fs::rename(repo.join(segment_path(2)), repo.join(segment_path(3))).unwrap();
    assert!(read_log_raw(&repo).unwrap_err().contains("contiguous"));
}

#[test]
fn a_declared_segments_directory_holds_only_segments() {
    for (name, dir_not_file) in [("notes.txt", false), (".DS_Store", false), ("000001.jsonl", false), ("2.jsonl", false), ("0000003.jsonl", false), ("000003.jsonl", true)] {
        let dir = segmented_fixture(6);
        let repo = dir.path().join("repo");
        let path = repo.join(SEGMENTS_DIR).join(name);
        if dir_not_file {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            std::fs::write(&path, "x\n").unwrap();
        }
        let e = read_log_raw(&repo).unwrap_err();
        assert!(e.contains("not a log segment") || e.contains("is not one"), "{name}: {e}");
    }
}

#[test]
fn an_empty_later_segment_fails() {
    for content in ["", "\n"] {
        let dir = segmented_fixture(6);
        let repo = dir.path().join("repo");
        std::fs::write(repo.join(segment_path(3)), content).unwrap();
        assert!(read_log_raw(&repo).unwrap_err().contains("holds no events"), "{content:?}");
    }
}

#[test]
fn every_segment_of_a_segmented_log_ends_with_a_newline() {
    for segment in [segment_path(2), EVENTS_PATH.to_string()] {
        let dir = segmented_fixture(6);
        let repo = dir.path().join("repo");
        let text = std::fs::read_to_string(repo.join(&segment)).unwrap();
        std::fs::write(repo.join(&segment), text.trim_end_matches('\n')).unwrap();
        assert!(read_log_raw(&repo).unwrap_err().contains("does not end with a newline"), "{segment}");
    }
}

#[test]
fn a_declared_log_with_one_segment_checks_as_core() {
    // No state/events/ at all, and an empty one: both a one-segment log.
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    declare_segments(&repo);
    commit_all(&repo, "declare");
    assert_eq!(log_segments(&repo).unwrap(), vec![EVENTS_PATH.to_string()]);
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    assert_eq!(read_log_raw(&repo).unwrap(), fixture_raw());
    for report in run_all(&repo, true) {
        assert!(report.passed(), "check {} failed: {:?}", report.number, report.failures);
    }
}

#[test]
fn the_declaration_is_exactly_one_line_of_the_manifest_block() {
    let base = std::fs::read_to_string(fixture_dir().join("AGENT.md")).unwrap();
    let with = |line: &str| base.replacen("spec_version: 1\n", &format!("spec_version: 1\n{line}\n"), 1);
    assert_eq!(manifest_declares_segments(&with("log_segments: 1")), Ok(true));
    assert_eq!(manifest_declares_segments(&with("log_segments: 1   ")), Ok(true));
    assert_eq!(manifest_declares_segments(&base), Ok(false));
    for refused in ["log_segments: 2", "log_segments: \"1\"", "log_segments: 1 # segmented", "  log_segments: 1", "log_segments:  1"] {
        assert!(manifest_declares_segments(&with(refused)).is_err(), "{refused:?} was accepted or ignored");
    }
    // Only the first fenced block is the manifest: a later fence is prose.
    let later = format!("{base}\n```yaml\nlog_segments: 1\n```\n");
    assert_eq!(manifest_declares_segments(&later), Ok(false));
}

#[test]
fn an_unreadable_declaration_fails_check_1() {
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    let manifest = std::fs::read_to_string(repo.join("AGENT.md")).unwrap();
    std::fs::write(repo.join("AGENT.md"), manifest.replace("log_segments: 1\n", "log_segments: 2\n")).unwrap();
    assert!(read_log_raw(&repo).unwrap_err().contains("not a log-segments declaration"));
}

#[test]
fn an_undeclared_repository_keeps_core_behavior() {
    // Without the declaration, files in state/events/ that are not named
    // like segments are not the extension's business (core v1 repositories
    // check as before); a segment-named file without it fails.
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    std::fs::write(repo.join(SEGMENTS_DIR).join("notes.txt"), "x").unwrap();
    assert_eq!(read_log_raw(&repo).unwrap(), fixture_raw());
    std::fs::write(repo.join(segment_path(2)), "x\n").unwrap();
    assert!(read_log_raw(&repo).unwrap_err().contains("does not declare"));
    std::fs::remove_file(repo.join(segment_path(2))).unwrap();
    // Named like a segment but numbered below 2, or not a file: refused, not ignored.
    std::fs::write(repo.join(SEGMENTS_DIR).join("000001.jsonl"), "x\n").unwrap();
    assert!(read_log_raw(&repo).unwrap_err().contains("is not one"));
    std::fs::remove_file(repo.join(SEGMENTS_DIR).join("000001.jsonl")).unwrap();
    std::fs::create_dir_all(repo.join(segment_path(2))).unwrap();
    assert!(read_log_raw(&repo).unwrap_err().contains("is not one"));
}

#[test]
fn an_undeclared_repository_s_state_events_history_is_not_append_only() {
    // Core v1 does not make state/events/ append-only: a core repository
    // that once committed and removed a file there still passes check 4.
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    // Named like a segment, but the repository never declares the extension.
    std::fs::write(repo.join(segment_path(2)), "x\n").unwrap();
    commit_all(&repo, "scratch");
    git(&repo, &["rm", "-q", "-r", SEGMENTS_DIR]);
    commit_all(&repo, "drop scratch");
    assert!(check_append_only(&repo).passed(), "{:?}", check_append_only(&repo));
}

#[test]
fn dropping_the_declaration_with_the_segments_is_still_a_deletion() {
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    git(&repo, &["rm", "-q", "-r", SEGMENTS_DIR]);
    let manifest = std::fs::read_to_string(repo.join("AGENT.md")).unwrap();
    std::fs::write(repo.join("AGENT.md"), manifest.replace("log_segments: 1\n", "")).unwrap();
    commit_all(&repo, "back to one file, history shortened");
    let report = check_append_only(&repo);
    assert!(fails_with(&report, &format!("{}: deleted in commit", segment_path(2))), "{report:?}");
}

#[test]
fn a_deleted_facts_file_fails_check_4() {
    // Core check 4: facts files are found in history, not only in the index,
    // so `git rm` of one fails as a deletion (since conformance-check 0.3.0).
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("old")).unwrap();
    std::fs::write(repo.join("old/facts.jsonl"), "fact one\n").unwrap();
    commit_all(&repo, "a fact");
    git(&repo, &["rm", "-q", "old/facts.jsonl"]);
    commit_all(&repo, "drop it");
    let report = check_append_only(&repo);
    assert!(fails_with(&report, "old/facts.jsonl: deleted in commit"), "{report:?}");
}

#[test]
fn deleting_a_segment_fails_check_4() {
    let dir = segmented_fixture(4);
    let repo = dir.path().join("repo");
    let raw = fixture_raw();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    // Split segment 2 so that segment 3 holds the last two lines.
    std::fs::write(repo.join(segment_path(2)), lines[4..lines.len() - 2].concat()).unwrap();
    // (rebuild a clean history: segment 2 without its tail, then segment 3)
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    commit_all(&repo, "segments 1-2");
    std::fs::write(repo.join(segment_path(3)), lines[lines.len() - 2..].concat()).unwrap();
    commit_all(&repo, "segment 3");
    for report in run_all(&repo, true) {
        assert!(report.passed(), "check {} failed: {:?}", report.number, report.failures);
    }
    // Deleting the last segment leaves a shorter log that still folds; check 4 catches it.
    git(&repo, &["rm", "-q", &segment_path(3)]);
    commit_all(&repo, "drop segment 3");
    assert!(fails_with(&check_append_only(&repo), "deleted in commit"), "{:?}", check_append_only(&repo));
}

#[test]
fn the_sealing_commit_may_extend_the_segment_it_seals() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    let raw = fixture_raw();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    std::fs::write(repo.join(EVENTS_PATH), lines[..4].concat()).unwrap();
    declare_segments(&repo);
    commit_all(&repo, "segment 1");
    // One commit appends to segment 1 and starts segment 2.
    std::fs::write(repo.join(EVENTS_PATH), lines[..6].concat()).unwrap();
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    std::fs::write(repo.join(segment_path(2)), lines[6..].concat()).unwrap();
    commit_all(&repo, "extend and seal segment 1");
    for report in run_all(&repo, true) {
        assert!(report.passed(), "check {} failed: {:?}", report.number, report.failures);
    }
}

#[test]
fn a_sealed_segment_never_changes() {
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    assert!(check_append_only(&repo).passed());
    // Moving the last line of segment 2 back into segment 1 keeps the
    // logical log identical, but appends to a sealed segment: check 4 fails.
    let seg1 = std::fs::read_to_string(repo.join(EVENTS_PATH)).unwrap();
    let seg2 = std::fs::read_to_string(repo.join(segment_path(2))).unwrap();
    let (first, rest) = seg2.split_at(seg2.find('\n').unwrap() + 1);
    std::fs::write(repo.join(EVENTS_PATH), format!("{seg1}{first}")).unwrap();
    assert!(fails_with(&check_append_only(&repo), "working tree changes a sealed segment"));
    std::fs::write(repo.join(segment_path(2)), rest).unwrap();
    commit_all(&repo, "append to a sealed segment");
    assert!(fails_with(&check_append_only(&repo), "changed after it was sealed"));
}

#[test]
fn a_segment_cannot_precede_its_predecessor() {
    let dir = fixture_in_temp_git();
    let repo = dir.path().join("repo");
    declare_segments(&repo);
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    std::fs::write(repo.join(segment_path(3)), "x\n").unwrap();
    commit_all(&repo, "segment 3 first");
    std::fs::write(repo.join(segment_path(2)), "y\n").unwrap();
    commit_all(&repo, "then segment 2");
    let report = check_append_only(&repo);
    assert!(fails_with(&report, &format!("{}: added in commit", segment_path(3))), "{report:?}");
    assert!(fails_with(&report, &format!("before {} existed", segment_path(2))), "{report:?}");
}

#[test]
fn a_snapshot_over_the_logical_log_verifies() {
    let dir = segmented_fixture(4);
    let repo = dir.path().join("repo");
    // Line 8 lies in segment 2: the integrity record counts logical lines.
    let snapshot = valid_snapshot(&repo, 8);
    let name = snapshot["integrity"]["event_id"].as_str().unwrap().to_string();
    write_snapshot(&repo, &name, &snapshot);
    let events = parse_events(&log_lines(&read_log_raw(&repo).unwrap())).unwrap();
    let report = check_replay(&repo, &events, &read_log_raw(&repo).unwrap());
    assert!(report.passed(), "{report:?}");
}

/// The fixture in three segments: 1 sealed by 2, then 2 sealed by 3.
fn three_segment_fixture() -> tempfile::TempDir {
    let dir = segmented_fixture(4);
    let repo = dir.path().join("repo");
    let raw = fixture_raw();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    let cut = lines.len() - 2;
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    std::fs::remove_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    commit_all(&repo, "segment 1");
    std::fs::create_dir_all(repo.join(SEGMENTS_DIR)).unwrap();
    std::fs::write(repo.join(segment_path(2)), lines[4..cut].concat()).unwrap();
    commit_all(&repo, "segment 2");
    std::fs::write(repo.join(segment_path(3)), lines[cut..].concat()).unwrap();
    commit_all(&repo, "segment 3");
    dir
}

#[test]
fn segment_2_is_sealed_by_segment_3() {
    let dir = three_segment_fixture();
    let repo = dir.path().join("repo");
    for report in run_all(&repo, true) {
        assert!(report.passed(), "check {} failed: {:?}", report.number, report.failures);
    }
    // Move segment 3's first line to the end of segment 2: the logical log
    // is unchanged, but segment 2 is sealed.
    let seg2 = std::fs::read_to_string(repo.join(segment_path(2))).unwrap();
    let seg3 = std::fs::read_to_string(repo.join(segment_path(3))).unwrap();
    let (first, rest) = seg3.split_at(seg3.find('\n').unwrap() + 1);
    std::fs::write(repo.join(segment_path(2)), format!("{seg2}{first}")).unwrap();
    std::fs::write(repo.join(segment_path(3)), rest).unwrap();
    commit_all(&repo, "append to sealed segment 2");
    let report = check_append_only(&repo);
    assert!(fails_with(&report, &format!("{}: changed after it was sealed by {}", segment_path(2), segment_path(3))), "{report:?}");
}

#[test]
fn a_segment_added_in_a_merge_and_deleted_later_fails_check_4() {
    // git log --name-only lists no files for a merge without -m: a segment
    // that only a merge adds must still be found in history.
    let dir = segmented_fixture(6);
    let repo = dir.path().join("repo");
    let raw = fixture_raw();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    // Rebuild: segments 1 and 2 hold all but the last line on main.
    std::fs::write(repo.join(segment_path(2)), lines[6..lines.len() - 1].concat()).unwrap();
    std::fs::remove_dir_all(repo.join(".git")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    commit_all(&repo, "segments 1-2");
    git(&repo, &["checkout", "-q", "-b", "side"]);
    std::fs::write(repo.join("side.txt"), "x\n").unwrap();
    commit_all(&repo, "side work");
    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("main.txt"), "y\n").unwrap();
    commit_all(&repo, "main work");
    // The merge resolution itself starts segment 3.
    git(&repo, &["merge", "-q", "--no-ff", "--no-commit", "side"]);
    std::fs::write(repo.join(segment_path(3)), lines[lines.len() - 1..].concat()).unwrap();
    commit_all(&repo, "merge, starting segment 3");
    for report in run_all(&repo, true) {
        assert!(report.passed(), "check {} failed: {:?}", report.number, report.failures);
    }
    // Dropped again in another merge's resolution: no ordinary commit ever
    // touches segment 3, so only `log -m` lists it.
    git(&repo, &["checkout", "-q", "side"]);
    std::fs::write(repo.join("side.txt"), "x\nz\n").unwrap();
    commit_all(&repo, "more side work");
    git(&repo, &["checkout", "-q", "main"]);
    git(&repo, &["merge", "-q", "--no-ff", "--no-commit", "side"]);
    git(&repo, &["rm", "-q", &segment_path(3)]);
    commit_all(&repo, "merge, dropping segment 3");
    let report = check_append_only(&repo);
    assert!(fails_with(&report, &format!("{}: deleted in commit", segment_path(3))), "{report:?}");
}
