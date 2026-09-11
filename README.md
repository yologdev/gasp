<div align="center">

<img src="docs/banner.png" alt="GASP — The Git Agent State Protocol" width="100%"/>

**A git-native standard for portable agent state.**

[Website](https://gasp.yolog.dev) · [Specification](SPEC.md) · [Reference Runtime](https://github.com/yologdev/yoagent-state) · [Live Agent Repo](https://github.com/yologdev/yoyo-gasp)

[![CI](https://github.com/yologdev/gasp/actions/workflows/ci.yml/badge.svg)](https://github.com/yologdev/gasp/actions/workflows/ci.yml)
[![yoagent-state](https://img.shields.io/crates/v/yoagent-state.svg?label=yoagent-state)](https://crates.io/crates/yoagent-state)
[![event schema](https://img.shields.io/badge/event_schema-v1-2563eb)](SPEC.md#event-log-format)
[![license](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

</div>

---

## What is GASP?

Every agent accumulates state: goals, attempts, test results, verdicts, memories. Today almost all of it is trapped — inside a vendor's session store, a runtime's process memory, or a pile of markdown notes with no lineage. Kill the process, swap the model, switch frameworks, and the *agent* — everything it learned about itself — is gone.

GASP takes a different position: **the repo is the agent.**

Everything that makes an agent *itself* lives in one git repository: its identity, its skills, its distilled memory, and — at the core — an **append-only event log** (`state/events.jsonl`) that folds deterministically into a typed graph of goals, runs, patches, evals, and decisions. The executor (model + loop + tools) is a swappable visitor that holds no durable state. Restoring the agent is:

```
git clone → replay the log → load identity + skills
```

Point any GASP-conformant runtime at the repo URL and the same agent resumes — on any machine, any model, any vendor's harness.

## Why a protocol?

| Problem today | What GASP gives you |
|---|---|
| **Lock-in.** Agent state lives in one vendor's session format; there is no export that another runtime can resume. | State lives in git — the most portable, replicated, tooled substrate there is. Any conformant runtime restores it. |
| **No lineage.** "Memory" is notes without provenance. *Why* does the agent behave this way? Nobody can say. | Every change traces a causal spine: `goal → run → patch → eval → decision → promotion`. The answer to "why" is a graph query. |
| **Fragile self-improvement.** Agents that modify themselves have no audit trail and no rollback story. | Self-improvement is a log pattern: a patch is only *Promoted* after a recorded eval passes and a recorded decision approves it — all append-only, all in git history. |

## How it works

<img src="docs/how-it-works.png" alt="Executor appends events to the agent repo; the log folds into a typed graph; restore = clone + replay" width="100%"/>

The executor appends **semantic events** — not chat transcripts — as JSONL lines. Line order is the authoritative total order; the log is append-only and enforced by walking git history, not by convention. A few (abridged) lines from the [canonical fixture](fixture/state/events.jsonl):

```jsonc
{"id":"event_01","kind":"goal.created",   "payload":{"id":"goal_retry","title":"Make retry reliable", ...}}
{"id":"event_04","kind":"patch.proposed", "payload":{"id":"patch_9","title":"persist retry counter", ...}}
{"id":"event_07","kind":"eval.finished",  "payload":{"id":"eval_5","status":"Passed", ...}}
{"id":"event_09","kind":"decision.created","payload":{"id":"decision_3","status":"Approved", ...}}
```

Folding the log yields a typed graph in which those facts are connected and queryable:

```
goal_retry ◄─advances─ patch_9 (Promoted, commit abc1234)
                          ├─validated_by─► eval_5    (Passed)
                          └─approved_by──► decision_3 (Approved)
```

Domain events pair with `state.ops_applied` events (the graph deltas), and the checker verifies the two families agree — so the graph you fold is provably the graph the events describe.

## The five conformance rules

Satisfy these and any other conformant runtime can resume your agent. None names a language or a library — that is what makes GASP a standard and not an API.

1. **State lives in a git repo** at the standard layout — log, identity, skills, projections.
2. **The event log is append-only, committed, and uses the event vocabulary.**
3. **Identity and skills are committed artifacts, loaded at runtime** — never compiled in.
4. **Restore is `clone + replay`** — fold the committed log into state; load identity and skills.
5. **The executor is swappable; state is independent of it** — swapping model or runtime touches `.agent/config.toml`, nothing else.

## Agent repository layout

```
agent-repo/
├── AGENT.md              # manifest: spec version, agent id, identity hash, executor binding
├── identity/             # FIRST-CLASS — who the agent is (human-gated)
├── skills/               # FIRST-CLASS — what it can do (versioned, one commit per change)
├── state/
│   └── events.jsonl      # SOURCE OF TRUTH — append-only semantic event log
├── memory/
│   ├── facts.jsonl       # append-only DISTILLED facts — never a history mirror
│   └── active_memory.md  # regenerated synthesis injected into context
├── journal/              # human-readable narrative — a projection of run events
├── transcripts/          # cold archive, optional, prunable
├── snapshots/            # projection checkpoints for fast restore (optional)
└── .agent/               # control plane (config.toml committed; lease/HEAD gitignored)
```

Four tiers, not two: **first-class committed** (log, identity, skills) → **derived append-only** (`facts.jsonl`) → **regenerable projections** (synthesis, journal, snapshots) → **cold prunable** (transcripts). The [SPEC](SPEC.md) defines the admission criterion that keeps memory distilled instead of becoming a second history.

## Conformance kit

This repo ships the standard *and* the machinery to verify it: a canonical [fixture](fixture/) every runtime must restore, and a [checker](conformance-check/) every emitter must pass — seven mechanical checks, fail-closed.

```sh
cargo run -q -- fixture --fixture   # verify the fixture
cargo run -q -- path/to/agent-repo  # verify any emitted repo
cargo test                          # kit self-tests (incl. corrupted-fixture negatives)
```

```
[PASS] check 1 — envelope round-trip
[PASS] check 2 — replay
[PASS] check 3 — vocabulary
[PASS] check 4 — append-only in git
[PASS] check 5 — causation integrity
[PASS] check 6 — restore
[PASS] check 7 — domain↔ops consistency
conformant: all checks passed
```

Check 4 is the one that makes "append-only" real: it walks the full git history and fails on any in-place edit or deletion of a protected path. Hooks are advisory; history is the gate.

## Proven in production

GASP is not a paper spec. [yoyo](https://github.com/yologdev/yoyo-evolve) — an autonomous, self-improving agent that has been evolving continuously for 125+ days — emits GASP from every session on an 8-hour cron into its own live agent repo, [yoyo-gasp](https://github.com/yologdev/yoyo-gasp): goals, patches, evals, and decisions from real self-improvement runs, skill evolution, and social sessions, all passing the conformance checker.

This is yoyo's actual `state/events.jsonl`, folded and rendered:

<img src="docs/yoyo-gasp-lineage.png" alt="yoyo's real event log folded into its goal/patch/eval/decision graph" width="100%"/>

## Extensions

- **[Permanence](extensions/PERMANENCE.md)** (draft) — durable, retrievable, owned agent state beyond any single host: encrypted payloads on Arweave (pay-once, ~$0.05), P2P redundancy via Radicle, and free Bitcoin-anchored provenance via OpenTimestamps. Optional; core conformance does not require it. Open thinking — feedback welcome.

## Ecosystem

| Repo | Role |
|---|---|
| [gasp](https://github.com/yologdev/gasp) (this repo) | The specification, canonical fixture, and conformance checker |
| [yoagent-state](https://github.com/yologdev/yoagent-state) | Reference runtime — Rust, on [crates.io](https://crates.io/crates/yoagent-state): typed graph, replay/fold, `GitEventStore` with durable append + boundary commits + cross-process lease |
| [yoyo-evolve](https://github.com/yologdev/yoyo-evolve) | Reference agent — the instrumented executor emitting GASP in production |
| [yoyo-gasp](https://github.com/yologdev/yoyo-gasp) | A living agent repo — yoyo's portable state, growing autonomously |
| [gasp.yolog.dev](https://gasp.yolog.dev) | Homepage with an animated log→graph fold |

## Credits & license

The "log is the agent" idea descends from [Yohei Nakajima's ActiveGraph](https://github.com/yoheinakajima/activegraph); `yoagent-state` is an independent Rust implementation of it, and GASP binds that idea to git as the interchange substrate.

[MIT](LICENSE)

Task-scoped execution recovery is specified in the optional draft [Task Execution Recovery extension](extensions/TASK_RECOVERY.md). Core clone-and-fold restores semantic state; session/workspace continuation requires the additional adapter contract.
