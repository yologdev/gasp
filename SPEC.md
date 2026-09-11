# GASP — The Git Agent State Protocol

**GASP** (Git Agent State Protocol) is a standard for portable agent state, native to git. Throughout this document, "GASP" and "the protocol" both refer to the normative standard defined in Part I. The name states the substrate: an agent's durable state lives in a **git** repository — an append-only event log that folds into a typed graph of goals, patches, evals, and decisions, and that graph, not a flat transcript, is what makes it agent state.

**Status:** buildable.
**Part I** is the normative protocol — no yoyo, no library, no language. **Part II** is the reference runtime (`yoagent-state`, Rust). **Part III** is the reference adapters for wrapping closed agents (Claude Code, Codex, and any other). **Part IV** is the reference agent (yoyo). **Part V** is honest caveats. **Part VI** is the conformance kit (fixture + checker).

The one-sentence idea: **an agent is stateless; its durable state is an append-only event log that folds into a queryable graph, committed into a git repo, and that repo — not the model or the runtime — is the agent.** Point any GASP-conformant runtime at the repo URL and the same agent resumes anywhere, on any model.

---

# Part I — The Protocol (normative)

## The model

An agent has two parts: an **executor** (model + loop + tools) and its **state**. GASP governs only state. Swap the executor freely; the state is independent of it.

State spans two media inside one repo:

- **Git owns the concrete *what*** — code, `identity/`, `skills/`, commits, diffs. Lossless, bisectable, shippable.
- **An append-only event log owns the semantic *why*** — a stream of typed events (goals, tasks, runs, observations, tool calls, failures, patches, evals, decisions) committed into the same repo at `state/events.jsonl`. Folded on load, it reconstructs a queryable history: what the agent was trying to do, why each change exists, what tested it, what was decided.

The repo is the agent. The log makes its history a queryable graph. Git stores and ships it. The executor reads a projection of the log, acts, and appends new events. Events *reference* git artifacts (a patch points to a commit as its artifact); git stores the artifacts. Neither duplicates the other.

## Conformance contract

Five rules. Satisfy them and any other conformant runtime can resume your agent. None names a language or a library — that is what makes GASP a standard and not an API.

1. **State lives in a git repo at the layout below** — log, identity, skills, projections.
2. **The event log is append-only, committed, and uses the event vocabulary below.**
3. **Identity and skills are committed artifacts, loaded at runtime, never compiled in.**
4. **Restore is `clone + replay`** — fold the committed log into state; load identity and skills.
5. **The executor is swappable; state is independent of it** — swapping model or runtime touches `.agent/config.toml`, nothing else.

## Repository layout

```
agent-repo/
├── AGENT.md                  # Manifest: spec version, agent id, identity hash, executor binding
├── identity/                 # FIRST-CLASS, human-gated constitution
│   ├── IDENTITY.md           # Who the agent is, goals, hard rules (immutable-by-policy)
│   ├── PERSONALITY.md        # Voice, values, tone (immutable-by-policy)
│   └── PRINCIPLES.md         # Operating principles / constraints
├── skills/                   # FIRST-CLASS, versioned capabilities
│   └── <skill-name>/
│       ├── SKILL.md          # YAML frontmatter (name, description, tools) + body
│       ├── scripts/
│       └── references/
├── state/
│   └── events.jsonl          # SOURCE OF TRUTH — append-only semantic event log
├── memory/                   # derived memory layer (see "Memory" below)
│   ├── facts.jsonl           # append-only DISTILLED facts — never a history mirror
│   └── active_memory.md      # regenerated synthesis injected into context
├── journal/                  # human-readable narrative — a projection of run events
│   └── JOURNAL.md
├── transcripts/              # COLD archive, out of the hot path — optional, prunable
│   └── <run-id>.jsonl.gz     # raw turn-by-turn events, referenced by hash from a run event
├── snapshots/                # PROJECTION checkpoints for fast restore (optional)
│   └── <event-id>/graph.json # materialized projection at a log position (+ integrity record)
└── .agent/                   # control plane
    ├── HEAD                  # last applied event id (reserved; LOCAL-ONLY, gitignored)
    ├── config.toml           # executor binding: model, provider (COMMITTED — restore reads it)
    ├── packs/                # optional vocabulary packs, *.json (COMMITTED — check 3 reads them)
    └── lease                 # active-worker lease              (LOCAL-ONLY, gitignored)
```

What's first-class vs derived — four tiers, not two:

- **First-class, committed:** the event log, `identity/`, `skills/`. The log is append-only; identity is human-gated; skills are versioned (each change a commit).
- **Derived, append-only:** `memory/facts.jsonl`. Extracted from runs by a synthesis step (usually an LLM), so it is *not* a deterministic fold and *not* freely regenerable — it is committed and append-only like the log, but derived. See "Memory" below.
- **Projections, regenerable:** `memory/active_memory.md`, `journal/`, `snapshots/`. Deterministic (or freely re-runnable) functions of the log and facts. Free to overwrite — with one exception: `JOURNAL.md` itself is append-only (commit rule 1, check 4), so the journal is regenerated by *appending* new run entries, never by rewriting existing lines.
- **Cold, prunable:** `transcripts/`. Never the source of truth.

`.agent/` is the control plane: `config.toml` is committed (restore step 2 reads it); `HEAD` and `lease` are machine-local coordination state and MUST be gitignored — a lease in history is meaningless on clone and invites push races.

## Event log format

`state/events.jsonl` — append-only, one JSON object per line. Each line is an **event envelope**:

| field | meaning |
|---|---|
| `id` | unique event id (`event_<uuid>`) |
| `schema_version` | event schema version (currently `1`) |
| `ts_ms` | unix epoch milliseconds |
| `actor` | who caused it — `{ "kind": "agent\|user\|system\|tool", "id": "..." }` |
| `kind` | the event type (dotted; see below) |
| `payload` | JSON; shape depends on `kind` |
| `causation_id` | the event that caused this one (`null` at a root) — the lineage link |
| `correlation_id` | groups events of one unit of work (e.g. a run) |

**Ordering (normative).** The physical line order of `state/events.jsonl` is the authoritative total order of events. `ts_ms` is advisory (clock skew, backfilled adapters); replay determinism (check 2) is defined over line order, never over timestamps.

Two event families share the log:

1. **Domain events** — the semantic record. `kind` is `entity.verb`: `goal.created`, `goal.status_changed`, `task.created`, `task.status_changed`, `run.started`, `run.finished`, `observation.created`, `failure.observed`, `hypothesis.created`, `patch.proposed`, `patch.status_changed`, `eval.finished`, `decision.created`, `model.called`, `model.finished`, `tool.called`, `tool.finished`, `artifact.attached`, `frame.created`, `project.snapshot_recorded`. The `payload` is the entity.
2. **State-ops events** — `kind` is `state.ops_applied`, `payload` is an array of graph mutations. **This is the only family the reducer folds into the graph;** domain events are a parallel human-and-machine-readable audit layer. The ops are: `CreateNode {id, kind, props}`, `UpdateNode {id, props}` (deep-merge), `TombstoneNode {id, reason}`, `CreateRelation {from, rel, to, props}`, `DeleteRelation {from, rel, to}`, `MarkStale {id, reason}`, `AttachArtifact {id, artifact}`. They serialize externally-tagged: `{"CreateNode": { ... }}`.

**Pairing rule (normative).** An **entity-creating domain event** — `goal.created`, `task.created`, `observation.created`, `failure.observed`, `hypothesis.created`, `patch.proposed`, `eval.finished`, `decision.created`, `frame.created` — is followed by exactly one `state.ops_applied` event whose `causation_id` is the domain event's `id` and whose ops create the entity. The ops MUST be consistent with the domain event's payload — the entity's id, kind, and status are mechanically checked (check 7); richer ops in the same pair (extra relations, artifact attachments) are permitted so long as they do not contradict it. Raw-layer kinds (`run.started`, `model.called`, `tool.called`, …) and `*.status_changed` MAY pair the same way but are not required to. An ops event's `causation_id` must name a domain event, never another ops event. A `state.ops_applied` with a `null` causation (an ops-only maintenance mutation) is permitted but must not restate a domain entity. This is what keeps the audit layer and the folded graph from silently diverging.

Node **kinds** that `CreateNode` uses: `goal`, `task`, `run`, `observation`, `failure`, `hypothesis`, `patch`, `eval`, `decision`, `model_call`, `tool_call`, `frame`, `project_snapshot`, plus the governance types `policy` and `behavior`. (Artifacts attach to nodes via `AttachArtifact`.)

Relation **kinds** that `CreateRelation` uses: `serves`, `blocks`, `advances`, `observes`, `explains`, `addresses`, `modifies`, `validated_by`, `approved_by`, `rejected_by`, `produced_by`, `derived_from`, `depends_on`, `supersedes`, `contained_in_frame`, `forked_from`, `references`.

The common causal spine (a backbone, not a forced linear workflow):

```
goal -> task -> run -> observation -> failure -> hypothesis -> patch -> artifact -> eval -> decision -> promotion
```

A "session," in human terms, is a `run` (one execution) under a `task` under a `goal`. Each `model.called` records the model, so a model swap is fully auditable from the log alone. The vocabulary is open and pack-validated — a runtime may declare additional kinds in a `Pack`; the kinds above are the shared baseline every conformant runtime understands.

## Worked example — a self-improvement run

A complete run where a skill patch is tested and promoted. Ids are shortened for readability (the runtime generates `event_<uuid>`, `goal_<uuid>`, …). Each **entity-creating domain event** is followed by the **`state.ops_applied`** event that mutates the graph (note each ops event's `causation_id` is its domain event — the pairing rule); raw-layer events (`run.started`, `tool.called`) go unpaired here, as the rule permits. Only ops events fold into state.

```jsonl
{"id":"event_01","schema_version":1,"ts_ms":1739000000000,"actor":{"kind":"agent","id":"evolve"},"kind":"goal.created","payload":{"id":"goal_retry","title":"Make retry reliable","summary":"retries drop state after timeout","status":"Open","owner":{"kind":"agent","id":"evolve"},"metadata":{}},"causation_id":null,"correlation_id":"goal_retry"}
{"id":"event_02","schema_version":1,"ts_ms":1739000000001,"actor":{"kind":"agent","id":"evolve"},"kind":"state.ops_applied","payload":[{"CreateNode":{"id":"goal_retry","kind":"goal","props":{"title":"Make retry reliable","status":"Open"}}}],"causation_id":"event_01","correlation_id":"goal_retry"}
{"id":"event_03","schema_version":1,"ts_ms":1739000000100,"actor":{"kind":"agent","id":"evolve"},"kind":"run.started","payload":{"run_id":"run_42","task":"fix retry skill","metadata":{}},"causation_id":"event_01","correlation_id":"run_42"}
{"id":"event_04","schema_version":1,"ts_ms":1739000000200,"actor":{"kind":"agent","id":"evolve"},"kind":"patch.proposed","payload":{"id":"patch_9","title":"persist retry counter","status":"Proposed","created_by":{"kind":"agent","id":"evolve"}},"causation_id":"event_03","correlation_id":"run_42"}
{"id":"event_05","schema_version":1,"ts_ms":1739000000201,"actor":{"kind":"agent","id":"evolve"},"kind":"state.ops_applied","payload":[{"CreateNode":{"id":"patch_9","kind":"patch","props":{"title":"persist retry counter","status":"Proposed","references_commit":"abc1234"}}},{"CreateRelation":{"from":"patch_9","rel":"advances","to":"goal_retry","props":{}}}],"causation_id":"event_04","correlation_id":"run_42"}
{"id":"event_06","schema_version":1,"ts_ms":1739000000300,"actor":{"kind":"tool","id":"cargo"},"kind":"tool.called","payload":{"id":"tool_1","run_id":"run_42","tool":"cargo test","input_summary":"cargo test retry","output_summary":"ok, 1 passed","success":true,"metadata":{}},"causation_id":"event_04","correlation_id":"run_42"}
{"id":"event_07","schema_version":1,"ts_ms":1739000000400,"actor":{"kind":"agent","id":"evolve"},"kind":"eval.finished","payload":{"id":"eval_5","command":"cargo test retry","status":"Passed","score":1.0,"metadata":{}},"causation_id":"event_06","correlation_id":"run_42"}
{"id":"event_08","schema_version":1,"ts_ms":1739000000401,"actor":{"kind":"agent","id":"evolve"},"kind":"state.ops_applied","payload":[{"CreateNode":{"id":"eval_5","kind":"eval","props":{"command":"cargo test retry","status":"Passed","score":1.0}}},{"CreateRelation":{"from":"patch_9","rel":"validated_by","to":"eval_5","props":{}}}],"causation_id":"event_07","correlation_id":"run_42"}
{"id":"event_09","schema_version":1,"ts_ms":1739000000500,"actor":{"kind":"agent","id":"evolve"},"kind":"decision.created","payload":{"id":"decision_3","status":"Approved","reason":"eval passed; promote","decided_by":{"kind":"agent","id":"evolve"},"metadata":{}},"causation_id":"event_07","correlation_id":"run_42"}
{"id":"event_10","schema_version":1,"ts_ms":1739000000501,"actor":{"kind":"agent","id":"evolve"},"kind":"state.ops_applied","payload":[{"CreateNode":{"id":"decision_3","kind":"decision","props":{"status":"Approved"}}},{"CreateRelation":{"from":"patch_9","rel":"approved_by","to":"decision_3","props":{}}},{"UpdateNode":{"id":"patch_9","props":{"status":"Promoted"}}}],"causation_id":"event_09","correlation_id":"run_42"}
```

Fold the ten lines and the graph holds: `goal_retry` advanced by `patch_9` (status `Promoted`, `references_commit` `abc1234`), the patch `validated_by` a passed `eval_5` and `approved_by` `decision_3`. That subgraph **is** the version's track record — copy these shapes to write a first emitter or a transcript projector.

## Memory — facts are distilled, never a history mirror

`memory/` is where the agent keeps what it *learned*, not what it *did*. What it did is `state/events.jsonl`, completely and exclusively — `facts.jsonl` MUST NOT mirror the work history, or it becomes an unbounded second log that defeats its purpose.

**Admission criterion.** A fact earns a line in `facts.jsonl` only if it is (a) useful without its originating context — it changes *future* behavior rather than recording *past* behavior — and (b) not derivable from a lineage query over the fold. "The flaky test is `retry_timeout`; ignore single failures", "this API rate-limits at 10 rps", "the maintainer prefers squash merges" belong. "Ran cargo test at 14:32, passed" never does — that is a `tool.called`/`eval.finished` event.

**Format.** One JSON object per line, append-only: `{"id": "fact_<uuid>", "ts_ms": ..., "text": "...", "derived_from": ["event_..." | "run_..."], "supersedes": "fact_..." | null}`. `derived_from` keeps every fact auditable back to the events that taught it. Superseding a stale fact appends a new line pointing at the old one — append-only compaction; the synthesis step that writes `active_memory.md` uses only the latest version of each fact chain.

**Regenerability.** Fact *extraction* is a synthesis step (usually an LLM), not a deterministic fold — so `facts.jsonl` is committed and append-only like the log, and only `active_memory.md` (the current synthesis of live facts) is the freely-regenerable projection. Version the synthesis prompt; a bad synthesis can distort the human-facing story but never destroys facts.

## SKILL.md format

YAML frontmatter with required `name` and `description` (plus optional `tools`/`allowed-tools`) and a markdown body. Progressive disclosure: frontmatter loaded at startup, body on demand. Compatible with the open Agent Skills convention.

## Self-improvement is a log pattern

How a self-modifying agent avoids silently rotting, expressed entirely in the vocabulary above so any conformant runtime reads it the same way:

- A skill change is a **`patch`** that `advances` a goal (and `addresses` a failure if it fixes one). The patch pins the skill commit — as a prop or an attached artifact — which fixes the record to a skill *version*.
- The verification gate (tests, an oracle) produces an **`eval`**: `patch --validated_by--> eval`.
- Promote or revert is a **`decision`**: the patch is `approved_by` (or `rejected_by`) the decision, and the patch's `status` is set to `Promoted` (or reverted) via an `UpdateNode`. Promotion is a status on the patch, not a separate node.
- A version's track record (pass rate, sample size, trend) is a fold over its patches + evals + decisions — data that travels with the repo, so a swapped executor knows how good the agent's own skills are.

The boundary: the **eval and decision facts live in the log** (part of GASP); the **promotion policy** — minimum sample size, win-margin to replace an incumbent — is the executor's, never GASP's. The oracle is a plug-in; the policy that consumes its evals stays out of the log format.

## Restore contract

```
gasp restore <git-url> [--at <event-id>] [--model <model>]
```

1. `git clone <url>` (optionally a partial clone to skip cold transcripts).
2. Read `AGENT.md` + `.agent/config.toml`; verify spec version and identity hash.
3. Load `identity/` and `skills/` directly (runtime, never compiled).
4. **Fold the committed `state/events.jsonl` into state** (optionally seeded from the latest `snapshots/` checkpoint at or before `--at`, then fold forward). This reconstructs open goals, patch lifecycle, lineage.
5. Bind the chosen model/provider as the executor.
6. Resume: take the lease, continue the loop.

**Snapshot integrity (normative).** A snapshot at `snapshots/<event-id>/graph.json` is a JSON object `{"graph": <the folded graph>, "integrity": {"event_id": "...", "line_count": N, "sha256": "..."}}` where `line_count` is the number of physical lines of `state/events.jsonl` the snapshot folds through, `event_id` is the id of the last event in that prefix, and `sha256` is the SHA-256 of the **raw bytes** of those `line_count` lines, including their newlines. Restore verifies the record against the cloned log before seeding; on mismatch (rebased history, sharded log) it falls back to a full fold. A snapshot is an optimization, never an authority.

"Same agent, everywhere" = the same clone + fold on any machine. "Knows about you" = `identity/`, `memory/`, and the log's lineage all travel with the repo.

### Optional task execution recovery

Core restore recovers semantic agent state; it does not guarantee reconstruction of an executor conversation, uncommitted workspace, or in-flight tool execution. The optional [Task Execution Recovery extension](extensions/TASK_RECOVERY.md) defines task-scoped checkpoints, artifact integrity and retention, session versus semantic continuation, canonical publication and external-action reconciliation. It uses existing core vocabulary and leaves core v1 conformance unchanged. Its status is draft; runtime support must be demonstrated separately.

## Integration contract — wiring an existing agent in

GASP governs state, not your executor. Keep your model, loop, prompts, and tools; route their state through GASP. Three additive steps — closer to adding OpenTelemetry than adopting a framework:

1. **Point at a repo.** On start, clone-or-init the state repo from a URL. One line of config; this is the agent's identity + memory + log.
2. **Instrument the transitions.** Your loop already has the moments that matter — it picked a goal, ran a tool, a test passed, a change landed. Emit an event at each, in the vocabulary. The only adapter you write is the map from your internal concepts (your "task" → `task`, your test gate → `eval`, your accepted change → `patch` + `decision`).
3. **Load on start, commit on boundary.** Fold the log at startup, run your loop, persist at run close.

Everything else stays yours. You don't rewrite the agent; you instrument its state.

## Control-loop shape

A conformant loop, stateless over the committed log:

```
take_lease()
state = fold(state/events.jsonl)        # reconstruct projection from the log
start_run(goal)                          # append goal / task / run events
while not run_done:
    decision = model.next(view(state))   # PURE: state in, decision out (model swappable)
    if decision.is_tool_call:
        result = run_tool(decision)      # the gate (tests/oracle) yields an `eval`
        append(events_from(result))      # tool_call / observation / eval
close_run():                             # persist
    append(patch / decision)             # promote or revert per executor policy
    regenerate(journal, active_memory)   # projections
    commit(state/events.jsonl + projections + code) with Run-Id / Outcome trailer
release_lease()
```

The model is a pure function `model.next(view) -> decision`; it holds no durable state. Swapping the model touches `.agent/config.toml` only — log, identity, and skills are untouched, and each `model_call` event records which model produced it.

## Commit / append rules

1. **The log is committed, append-only.** Structural immutability is enforced by the conformance checker walking git history (check 4): any commit that modifies or deletes existing lines of `state/events.jsonl`, `*/facts.jsonl`, or `JOURNAL.md` is non-conformant. A local pre-commit hook that rejects such diffs is recommended as a fast guard — but hooks do not clone with the repo, so the hook is advisory; the history walk is the gate.
2. **One run = one closing commit** (code work commits freely inside it — don't squash away bisectable history). The closing commit carries a trailer for in-git orientation (the log fold, not the trailer, is the real query surface):
   ```
   Run-Id: 01J...
   Goal: goal_retry_reliability
   Outcome: promoted
   ```
3. **Projections are free to rewrite** — they are deterministic folds of the log. (`facts.jsonl` is not a projection — it is append-only; see Memory.)
4. **Identity changes are human-gated** and follow a defined procedure: one commit, made or approved by a human, that (a) edits `identity/`, (b) updates the identity hash in `AGENT.md`, and (c) appends a `decision` event recording who approved the change and why. All three in the same commit, so the hash in the manifest never disagrees with `identity/` at any point in history — an identity edit that skips the hash update would otherwise permanently fail restore step 2. The identity hash is SHA-256 over each `identity/` file's relative path followed by a newline and its bytes, in byte-order-sorted path order:
   ```
   find identity -type f | LC_ALL=C sort | while IFS= read -r f; do printf '%s\n' "$f"; cat "$f"; done | shasum -a 256
   ```
5. **Skills are versioned commits**, one change per commit, so each version is a pinnable hash a `patch` references as its `artifact`.
6. **Single-writer lease** before appending (`.agent/lease`, worker id + TTL, **local-only and gitignored**) prevents concurrent-writer corruption. Multiplayer and A/B use branches, not concurrent writes to one ref.
7. **Snapshots on cadence** (`snapshots/<event-id>/graph.json` + integrity record) so restore folds from a checkpoint instead of all of history.

---

## Adaptation contract — third-party and closed agents

Conformance is always an adapter that translates an agent's native record into the GASP vocabulary. The only variable is *placement*. For agents you build, the adapter is inline — the `emit` calls of the integration contract, inside your loop. For agents you don't control (Claude Code, Codex, and others), the adapter sits *outside* the loop and translates their output; you never touch their internals, because GASP is defined over state, not the loop.

**Fidelity depends on what the agent exposes — three rungs, best to worst:**

1. **Native hooks / session-store** — the agent fires callbacks at its own boundaries; emit GASP events in-band as it runs. Real-time, highest fidelity, and (where hooks are documented) the only *supported* surface.
2. **Transcript projector** — the universal fallback: every such agent leaves a session transcript on disk. Tail or post-process it into `state/events.jsonl` and commit. Works for anything that writes a transcript, hooks or not — but transcript formats are usually undocumented internals; pin the agent version and expect breakage on upgrade.
3. **Model/API proxy** — run the agent behind a shim that sees its model calls and tool I/O; emit from there. Medium fidelity, for when there is neither hook nor transcript.

Whatever an agent exposes picks its rung. This is how any agent — named or not — slots in.

**The lifting ceiling (inherent).** Closed agents emit *raw* events (messages, tool calls), not *semantic* ones. The raw layer — `tool_call`, `observation`, `model_call` — maps nearly 1:1, cheap and faithful. The lineage spine — `goal`, `hypothesis`, `patch`, `eval`, `decision` — is not in a raw transcript; you infer it (a passing test command → `eval`; an accepted edit or commit → `patch`; the task prompt → `goal`), supply it out-of-band, or leave it thin. A closed agent therefore yields a **transcript-faithful conformant log**; the rich graph is only as good as what the adapter can lift. You cannot extract decisions the agent never externalized — a property of the source, not a defect of the adapter.

**Export is easy; import is partial.** You can rehydrate identity + skills + context into a *fresh* session of a closed agent — map identity onto its own config surface, prime context from the fold — but you cannot inject state back mid-tool-call, because you do not own its loop. Restore-into-a-closed-agent means "start a new session that knows who it is and what happened," not "resume exact loop state." Agents you own get full mid-loop resume (the restore contract); closed agents get session rehydration.

**What this buys for closed agents:** not resumability — a uniform, ownable record across vendors. Run one agent today and another tomorrow, point both adapters at the same repo, and identity, memory, and lineage are vendor-independent; they survive switching harnesses. One portable spine under a heterogeneous fleet.

**The contract.** An adapter is conformant if its emitted log (a) uses only the GASP vocabulary, (b) is appended durably and committed per the rules, and (c) round-trips — a conformant runtime can `restore` the resulting repo and continue. The unit of work is one adapter per agent (`claude-code`, `codex`, …), validated exactly as a native runtime is.

---

# Part II — Reference runtime: `yoagent-state` (Rust)

The cheap path for a Rust agent to satisfy Part I. A Python or TS agent implements the same layout itself and does not need this.

`yoagent-state` (v0.4.0, MIT) is an ActiveGraph-inspired continuity runtime. It records the append-only event log, folds it into the semantic graph, and ships the read-side primitives Part I only implies: **lineage queries, replay, fork, diff, typed packs, policy gates, behavior subscriptions.** You don't hand-author the log format — the runtime writes it via `apply_ops`.

**Why this and not ActiveGraph directly.** ActiveGraph is the inspiration, credited in the repo's `ACKNOWLEDGMENTS.md`. But it is Python, and the target agents are Rust single binaries shipped via brew/git-URL init; a Python sidecar or FFI in the hot path breaks the distribution story and fragments the event vocabulary across two implementations. Using ActiveGraph directly wouldn't even save work — it stores to its own event store, so a git adapter is needed either way. Being the canonical Rust implementation of "the log is the agent" is the stronger position; attribution plus protocol-compatibility claims the lineage without the dependency.

## The pluggable store + the store contract

Persistence is the `EventStore` trait (`append` / `scan` / `scan_after`) — the seam between the graph logic (above) and storage (below); `SnapshotStore`, `ForkStore`, `IndexStore`, and `ArtifactStore` are sibling traits on the same seam. `MemoryEventStore` is the in-memory impl; git-backed durable state is another impl of the same trait, so the runtime stays git-unaware and intentionally small. The store is *the* place Part I's "append-only, committed log" rule is actually satisfied, and a naive append-and-commit gets two things wrong:

1. **Durable append, boundary commit — two different jobs.** Appending a line to `events.jsonl` and `git commit` are separate operations. The JSONL append is the *durability* guarantee: flush each event immediately, so a crash mid-run loses nothing. The commit is the *shipping/restore* guarantee: one commit per run boundary, so history stays bisectable and you avoid commit spam. Committing per event destroys history; buffering in memory until the commit loses the tail on crash. The only correct shape is **append-and-flush per event, commit at the run boundary**.
2. **The lease lives in the append path, not beside it.** The single-writer lease and a git-backed append store protect the same thing — concurrent writes to one ref, the failure that corrupts SQLite-backed agents. If the store is the only writer and checks the lease *inside* its append, corruption is structurally impossible. Bolt the lease on beside the store and a second writer slips through.

Neither guarantee is part of GASP — another language's impl meets conformance rule 2 its own way.

## API shape

Verified against the v0.4.0 source. `load` folds the log into the graph (the projector folds **only** `state.ops_applied`, exactly as Part I specifies). The **paired** typed helpers — `record_goal`, `record_task`, `record_observation`, `record_failure`, `record_hypothesis`, `propose_patch`, `record_eval`, `record_decision_node`, `record_frame`, plus `record_run_started`, `record_run_finished`, `record_model_call`, `record_tool_call`, `record_project_snapshot`, and the `update_*_status` family — emit a domain event **and** the `state.ops_applied` that mutates it, linked per the pairing rule. Events recorded during an open run auto-chain to `run.started` and carry the run id as `correlation_id`; the finish pair closes the folded run node (`status: finished` + outcome). (`record_decision`, `record_eval_result`, and `link` are lower-level ops-only paths; `apply_ops` is the raw escape hatch.)

```rust
let state = YoAgentState::load(store).await?;                  // fold events -> graph
state.record_goal(Goal::new(goal_id, "Make retry reliable", "...", actor)).await?;
state.record_eval(actor, eval_result, Some(patch_id)).await?; // eval + patch validated_by eval
state.record_decision_node(actor, decision, Some(patch_node)).await?; // decision + approved_by
print!("{}", state.lineage(node).await.to_markdown());        // the read side Part I implies
```

For callback-driven agents, the `YoAgentStateSink` trait is the in-process adapter — `on_run_started`, `on_run_finished`, `on_model_called`, `on_model_finished`, `on_tool_called`, `on_tool_finished` — i.e. the integration contract as a typed interface; `YoAgentStateAdapter` is the shipped impl. Scoring uses the same primitives — `patch`/`eval`/`decision`, **policy gates** for the promotion gate (thresholds as config), **fork** for counterfactual A/B.

**The store contract ships in 0.3.0+.** Three `EventStore` impls: `MemoryEventStore` (tests), `JsonlEventStore` (small logs; rewrites the whole file per append, in-process lock only — kept for compatibility), and **`GitEventStore`**, which satisfies both store-contract guarantees: append + flush + fsync per batch (plus a parent-directory fsync on first creation), an atomically-acquired cross-process single-writer lease checked *inside* `append`, and pathspec-scoped boundary commits via `commit_run(&RunId, &GoalId, outcome, extra_paths)` carrying the `Run-Id`/`Goal`/`Outcome` trailers. The pairing rule is implemented (the paired helpers link each ops event's `causation_id` to its domain event), and run transitions are validated (double-start / mismatched-finish are errors). Repos emitted through the paired `record_*` helpers or the **`YoAgentStateSink` adapter** (conformant as of 0.4.0), on `GitEventStore`, pass all 7 conformance checks (Part VI).

**Known 0.4.0 gaps**: there is no snapshot emitter yet — the conformance checker is the executable definition of the snapshot format; the open-run marker is in-memory only (`load` does not recover an unfinished run, so a process restarted mid-run records uncorrelated roots until it starts a new run); the adapter's `*_finished` callbacks record raw events without updating the call nodes' outcome props; and the run structs' `metadata` fields are not persisted by the paired helpers.

## What yoagent-state subsumes

Earlier drafts of this spec had the agent inventing two mechanisms the library already provides. Defer to the library:

| Earlier draft invented | yoagent-state primitive that replaces it |
|---|---|
| a flat session index file | `goal`/`task`/`run` events; fast query is a **lineage query**, not a grepped file |
| a separate scoring stream | the **patch → eval → decision → promotion** lifecycle |

---

# Part III — Reference adapters: wrapping closed agents

Concrete adapters for two common coding agents — the outside-the-loop case of the adaptation contract. **Transcript locations and hook surfaces shift between releases; treat every path and field below as a starting point and verify against current docs before building.** (Terminology: yoagent-state's own `projector` module is the event→graph reducer and its `adapter`/`YoAgentStateSink` is the in-process event sink; the transcript adapters here are external and new — they emit the same events from an agent you don't control.)

## Claude Code

- **Rung 1 (hooks) — the primary path.** Hooks are the *documented, supported* surface and the adapter should lead with them: `PreToolUse`/`PostToolUse` (carrying `tool_name`, `tool_input`, `tool_response`), `Stop`, `SessionStart`/`SessionEnd`, `UserPromptSubmit`, subagent and compaction events, and more — every hook payload includes `session_id`, `transcript_path`, and `cwd`. Projection: `PostToolUse` → `tool_call` + `observation`, `SessionStart`/`Stop` → `run.started`/`run.finished`, with `transcript_path` linking the cold archive. The Agent SDK can drive sessions programmatically and receives each message; sessions it creates persist to the same local store.
- **Rung 2 (transcript projector) — fallback, explicitly unstable.** The session transcript is JSONL at `~/.claude/projects/<encoded-cwd>/<session-uuid>.jsonl`, but the entry format is **officially internal and changes between versions** — the docs state that scripts parsing these files directly can break on any release. If you project from it anyway: top-level line types include `user`, `assistant`, `system`, `summary` (plus housekeeping types), linked by `parentUuid`, carrying `cwd`, `gitBranch`, and token usage; **tool calls are `tool_use`/`tool_result` content blocks inside `message.content`, not line types**. Projection: `tool_use` block → `tool_call`, `tool_result` block → `observation`, each assistant turn → `model_call`; the `parentUuid` chain gives causal ordering for free. Pin the Claude Code version, and treat the projector as per-version.
- **Lifting.** A test/build command that passes → `eval`; an applied edit or resulting git commit → `patch` referencing that commit as `artifact`; the initiating user task → `goal`. Everything above the raw layer is heuristic.
- **Import.** Map `identity/` onto `CLAUDE.md`, prime context from the fold; a new session starts knowing who it is and what happened.

## Codex

- **Rung 2 (transcript projector).** Rollout transcript is `rollout-*.jsonl` (recent versions compress to `.jsonl.zst`, first line a `SessionMeta` header) under `~/.codex/sessions/YYYY/MM/DD/`, with session metadata indexed in a local SQLite (`state_5.sqlite` as of mid-2026; the rollout files remain the source of truth). There is **no parent-event field** — causality is inferred from order, so the projector reconstructs the chain positionally. Model/tool/observation projection is otherwise the same shape as Claude Code's.
- **Caveat.** Codex's compaction summaries are genuinely ecosystem-locked: compaction is a server-side call returning an `encrypted_content` blob the client cannot read. Do not rely on them — project from the raw rollout lines, which you control.
- **Import.** Map `identity/` onto Codex's `AGENTS.md` surface (its documented instruction chain); prime context from the fold.

## Any other agent (the "etc." case)

The contract, not a per-agent special case. Whatever the agent exposes picks the rung: native callbacks → rung 1; a transcript on disk → rung 2; neither → run it behind a model/API proxy (rung 3). Write one adapter, emit the GASP vocabulary, validate by round-trip. An agent with no hooks, no transcript, and no proxyable boundary cannot be adapted — the only hard exclusion.

---

# Part IV — Reference agent: yoyo (first adopter)

yoyo is the proof GASP works end to end. It is one agent; GASP is agent-agnostic. Its own motto is the layering:

```
yoagent executes.   yoagent-state remembers.   yoyo evolve improves.   git ships.
```

Today `yoyo-evolve` depends on `yoagent` (the executor) but **not yet on `yoagent-state`** — the wiring below is the integration to build, not a description of what exists. The mapping (verified against the public repo, July 2026):

| yoyo today | GASP home | Notes |
|---|---|---|
| `IDENTITY.md` / `PERSONALITY.md` (top-level, gate-protected) | `identity/` | The 🐙 persona; `evolve.sh`'s protected-file gate is the "immutable-by-policy" enforcement |
| `ECONOMICS.md` (top-level) | `identity/PRINCIPLES.md` | Operating constraints |
| `scripts/yoyo_context.sh` (identity loaded at runtime) | restore step 3 | "Identity is runtime, never `include_str!`" = conformance rule 3 |
| `memory/learnings.jsonl` → `memory/active_learnings.md` | `memory/facts.jsonl` → `active_memory.md` | Append-only-raw → regenerated-view; yoyo already honors the admission criterion — learnings, not history |
| `journals/` (+ `dreams/`) | `journal/` projection | Now a projection of `run` events |
| `skills/` + `skills_attic/` (top-level, auto-scanned) | `skills/` | Already the right shape; the attic is retired versions |
| audit-log branch + session tags | `transcripts/` cold store | Session evidence already lives out of the hot path — a branch instead of a directory |
| GH Actions: `evolve.yml` → `evolve.sh`: assess → plan → ≤3 tasks → `cargo build`+`cargo test` + evaluator gate → pass=commit, fail=`git reset --hard` + auto-filed issue | the control loop + patch/eval/decision | "pass→commit, fail→revert" *is* the patch lifecycle |
| (no semantic log / lineage today) | `state/events.jsonl` via yoagent-state | The core addition: lineage, replay, fork, version scoring |

Because yoyo's existing layout predates GASP, `AGENT.md` declares where identity and memory actually live (the manifest binds locations; the layout above is the default, not a straitjacket for adopters with history). The one thing yoyo gains is the semantic log itself — turning git history into a queryable lineage graph and making the self-improvement ratchet a primitive rather than an ad-hoc commit-or-revert script.

**Skill evolution maps onto the Part I self-improvement pattern directly.** yoyo's skill-evolve cycle (one `refine | create | retire` change per cycle, one commit per change) is the canonical case: each cycle is a `run` under a standing `goal` (`goal_skill_quality`), a completed change is a `patch` pinning the skill commit as its artifact, the harness gate (diff-scope hard rules + build/test) is the **oracle** producing the `eval`, and the keep/revert verdict is the `decision`. A *retirement* is a patch like any other — the commit it pins is the move into the attic; `refused`/`NO-OP` cycles close their run with that outcome and no patch. Two boundaries hold: (1) **skill changes never enter `memory/facts.jsonl`** — they are history, fully derivable from the patch spine, and the Memory admission criterion excludes them; a *distilled insight* behind a change may become a fact later, written by the agent's own synthesis with `derived_from` pointing at these events. (2) **Mirror-on-change keeps conformance rule 3 honest during the two-repo interim**: while the executor still carries its own live skill tree, every promoted skill change is mirrored into the state repo's `skills/` (rebound to the state layout's paths) and rides the *same boundary commit* as its events — so a restored agent gets current skills, and each skill version ships with its lineage. The same session-end mirror keeps the memory streams current: new learnings convert to the fact envelope (`derived_from` naming the run that taught them) and append to `facts.jsonl`; journal entries append (the state copy honors check 4 even though the executor prepends); regenerable projections (active syntheses, dream arc, day counter) are overwritten. The end-state (the executor loading skills and memory from the state repo, retiring the mirrors) remains the migration target.

---

# Part V — Sharp edges (honest)

**Inherent to the approach** (any conformant impl inherits these):

- **Compaction.** The semantic log is far sparser than raw tokens, so you keep it whole. The lossy steps are the *derived layers* (`facts.jsonl` extraction, `JOURNAL.md`, `active_*.md`) and the pruning of cold transcripts — a bad synthesis can distort the human-facing story. Keep the log + code commit always; regenerate projections freely; version the synthesis prompt; facts stay auditable via `derived_from`.
- **World-state outside the log.** Replaying events rebuilds the projection, not the filesystem or external APIs. Restore is faithful to what was logged, not the live world.
- **Multiplayer merges.** Append-only JSONL keyed by node id union-merges cleanly; *semantic* conflicts (contradictory observations/decisions) need application-level reconciliation, not `git merge`. Keep identity merges human-gated.
- **Git scaling.** Sparse semantic events keep file/commit counts far below per-token logging, but a long-lived agent can still approach GitHub's soft limits (≤3,000 dir entries; <1 GB ideal; files >100 MiB blocked). Shard `state/`, snapshot on cadence, keep transcripts out of the main tree, use partial/shallow clone. Git is the right *interchange* substrate; at extreme scale the hot-path store may want a Merkle-chunked log.
- **Secrets/PII.** Events are immutable and clonable, so a leaked secret is permanent and propagates on clone. Pre-commit secret scanning + redaction tokens; "right to be forgotten" fights append-only semantics.
- **Closed-agent fidelity is bounded by liftability.** Wrapping an agent you don't control (Part III) gives a transcript-faithful log, but the lineage spine is only as rich as the adapter can infer — you can't extract decisions the agent never externalized.

**Reference-impl specific:**

- **Determinism.** True *transcript* replay needs cached model/tool responses (an idea inherited from ActiveGraph). Not needed for the semantic-log hot path; only if you enable transcript replay.
- **Maturity.** `yoagent-state` is young (v0.4.0) and small by design — it deliberately does not handle identity/skills/memory, which is Part I's job, not a gap. The API will move (0.3.0 added run-transition validation, 0.4.0 changed adapter payload shapes); pin versions, track its CHANGELOG, and keep the event vocabulary protocol-compatible with ActiveGraph so the lineage claim stays clean.
- **Lease scope.** `GitEventStore`'s file lease is decided by atomic exclusive create — sound on a local filesystem, advisory on network filesystems, and stealing an *expired* lease retains a narrow multi-process race window. One agent repo per machine (the intended deployment) is well inside the guarantee.

---

# Part VI — Conformance kit (fixture + checker)

A spec is not a standard until "conformant" is verifiable. The kit is two things: a **fixture** every runtime must restore, and a **checker** every emitter must pass.

## The fixture — a canonical agent-repo

A minimal repo at the Part I layout, shipped as the reference:

```
fixture/
├── AGENT.md                  # spec_version, agent id, identity hash
├── identity/IDENTITY.md      # a one-line constitution
├── skills/retry/SKILL.md     # one skill, frontmatter + body
├── .agent/config.toml        # executor binding (model, provider)
└── state/events.jsonl        # the ten lines from the worked example above
```

Folding `state/events.jsonl` MUST yield a graph in which:

- `goal_retry` exists and is the target of an `advances` edge from `patch_9`;
- `patch_9` has `status = Promoted` and `references_commit = abc1234`;
- `patch_9 --validated_by--> eval_5`, and `eval_5.status = Passed`;
- `patch_9 --approved_by--> decision_3`, and `decision_3.status = Approved`.

A runtime that points at a fixture checkout, loads `identity/` + `skills/`, folds the log, and reports that graph has proven Part I restore.

## The checker — what "conformant" verifies

`conformance-check <repo>` runs seven checks and exits non-zero on any failure:

1. **Envelope round-trip.** Every line parses to the event envelope (`id, schema_version, ts_ms, actor, kind, payload, causation_id, correlation_id`) and re-serializes structurally equal — no unknown top-level fields, none missing.
2. **Replay.** Folding the log succeeds, and every snapshot equals the fold of the log prefix named by its integrity record (whose raw-byte hash and terminal event id must match the log — the Part I snapshot format). (If model/tool replay is enabled, responses must be cached so folds stay reproducible.)
3. **Vocabulary.** Every `CreateNode.kind` and `CreateRelation.rel` is in the baseline vocabulary or in a `Pack` the repo declares at `.agent/packs/*.json`. Unknown kinds without a declaring pack fail.
4. **Append-only-in-git.** For every commit touching an append-only path (`state/events.jsonl`, `*/facts.jsonl`, `JOURNAL.md`), the diff adds lines at EOF only — no modification or deletion of existing lines, and the working tree must extend the last committed version. Walk the git history (full history, no simplification); any in-place edit or deletion fails, a directory that is not a git repository fails (conformance rule 1), and any error during the walk fails closed. (This, not a pre-commit hook, is the enforcement.)
5. **Causation integrity.** Event ids are unique; every non-null `causation_id` references an earlier event id in the log (which makes the chain acyclic); roots (null causation) are `*.created` / `*.started` events or ops-only `state.ops_applied` maintenance events (per the pairing rule).
6. **Restore.** The manifest (`AGENT.md`) and identity are present and the log folds; run with `--fixture` against the fixture, the fold must additionally reproduce the four asserted graph facts. (Manifest-declared alternate locations, skills loading, and identity-hash verification are the runtime's restore obligations — not yet mechanically checked here.)
7. **Domain↔ops consistency (the pairing rule).** Every entity-creating domain event (the Part I table: `goal.created`, `task.created`, `observation.created`, `failure.observed`, `hypothesis.created`, `patch.proposed`, `eval.finished`, `decision.created`, `frame.created`) is materialized by **exactly one** paired `state.ops_applied` whose `CreateNode` matches the payload's id, kind, and status — all claimants are checked, so a decoy pair cannot shadow a contradiction. An ops event chained to another ops event fails.

## The conformance claim

A **runtime** is conformant iff it can `restore` the fixture (check 6) and its own emitted repos pass checks 1–5 and 7. An **adapter** (Part III) is conformant iff its projected repo passes the same checks and round-trips through restore — the same bar, whether events came from a native loop or a wrapped agent's transcript. Conformance is per-emitter and mechanically checkable; that is what turns "compatible" from a claim into a test.
