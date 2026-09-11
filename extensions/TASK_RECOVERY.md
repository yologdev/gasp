# Task Execution Recovery

Status: DRAFT, optional extension `gasp.task-recovery/v1`. Core GASP v1 conformance is unchanged. MUST/SHOULD below apply only to implementations claiming this extension. This document specifies a contract; it does not claim an implemented adapter or automated conformance suite.

## Scope and guarantees

Core restore reconstructs semantic agent state. This extension selects one task and restores enough recorded state to continue its work. A task has one stable task ID and multiple run IDs; every resumed execution creates a new run linked to the task and prior checkpoint. A run ID is not a task ID. Correlation IDs alone do not establish task membership: explicit task/run relations must be present in the folded graph.

Two recovery modes MUST be distinguished:

- `session`: restore a compatible executor's serialized conversation/tool state plus workspace at a recorded safe boundary. This is session continuation, not a process-memory snapshot, deterministic model replay, or a guarantee of identical output.
- `semantic`: reconstruct the objective, decisions, progress, evidence, remaining work and action outcomes from portable records. Starting a new conversation or changing executor format MUST NOT silently be reported as session recovery.

A hash mismatch, wrong task, missing required artifact or incomplete commit MUST block that checkpoint. An adapter MAY offer explicitly recorded semantic recovery or an older valid checkpoint, after reconciling later effects. It MUST NOT silently start an empty session and call it a successful resume.

## Existing vocabulary and representation

Use existing tasks, runs, observations, artifact attachments, metadata and state operations. No new core event kinds or statuses are introduced. The core pairing rule still applies; domain audit events alone do not update the folded graph.

An immutable recovery manifest is attached to the producing run as an `ArtifactRef` of kind `gasp.task-recovery/v1`, with URI and required SHA-256 hash. A paired observation may describe checkpoint creation. An update to the task's namespaced metadata records `gasp.task-recovery/v1.latest` as the checkpoint ID and manifest reference. Attachments, task/run relations and this pointer MUST be committed together. The pointer is an index; its ownership and references must be validated.

The manifest MUST carry:

| Field | Meaning |
| --- | --- |
| `schema` | `gasp.task-recovery/v1` |
| `agent_id`, `task_id`, `run_id`, `checkpoint_id` | Explicit ownership and immutable checkpoint identity |
| `parent_checkpoint_id` | Previous checkpoint, or null for the first |
| `recovery_modes` | Supported modes: session and/or semantic |
| `state_boundary` | Last consumed event ID, physical line count and SHA-256 of that raw log prefix, using core snapshot integrity semantics |
| `context` | State-repo source commit, identity hash, skills commit/hash and selected memory/book revisions or artifact references |
| `executor` | Runtime name/version/build digest, session format/version, provider and model |
| `artifacts` | Required/optional roles, URIs, byte lengths, media types and SHA-256 hashes; include portable task-state, workspace and session when session recovery is offered |
| `actions` | Reference to a durable action ledger, recording planned, dispatched, confirmed or uncertain external effects and idempotency keys |
| `boundary` | Safe-boundary description and any pending tool calls; never pretend a dispatched action is an unstarted action |

The portable task-state artifact MUST include the objective, constraints, decisions, progress, remaining work and evidence references. Workspace state MUST include uncommitted changes and required files, not merely the source repository's commit. Paths must be relative to the task workspace and must not escape it when restored. Credentials and machine-local leases MUST NOT be checkpointed. Stored tool permissions are provenance, never authority to regrant permissions on another host.

The consumed state boundary precedes the checkpoint commit, avoiding a self-referential commit hash. The checkpoint's commit is recorded by the writer's acknowledgement. Required artifacts may live in git or external storage; external storage requires an authenticated resolver and an export/import mechanism that preserves the bytes and hashes. A clone without required external bytes is not a complete execution-recovery export.

## Persist and publish

Before any checkpoint publication, the adapter MUST exclude credentials from exported paths, session payloads and tool outputs and validate the exported bytes with host-controlled credential checks. Failure MUST block publication; a private object store is not an exemption. Credentials needed after resume must be rebound from the destination host secret store. Any sanitization occurs before hashing, and loss of required state must be reflected in the supported recovery modes. A task must not be able to disable its own writer checks.

1. Reach a declared boundary. Serialize session and workspace consistently; include unresolved action identifiers.
2. Persist immutable artifacts and manifest, verify hashes and durable readability. Failed session saving MUST NOT produce a session-recoverable checkpoint.
3. Submit the checkpoint with expected parent ID to the canonical writer. Serialize publication, deduplicate checkpoint IDs, and reject conflicting parent updates rather than overwriting a concurrent successor.
4. Append the applicable core events/operations and commit their task pointer, manifest reference and graph relations together. Acknowledge only after the canonical commit is durable and verified.
5. Retain any local outbox until acknowledgement. An upload without a committed reference is not a published checkpoint. Duplicate submissions must not duplicate semantic events.

Task execution leases and fencing generations belong to the live coordinator, not committed git leases. The canonical writer must validate the current owner/generation when accepting a checkpoint. Coordination across hosts cannot rely on the core machine-local lease file alone.

## Restore one task

1. Accept an explicit agent and task ID; acquire a task execution lease with fencing.
2. Verify the manifest/identity and fold the canonical log (a core graph snapshot remains only an optimization). Select the task's latest valid committed checkpoint through explicit task/run membership.
3. Verify the checkpoint manifest and all required artifacts, ownership, parent lineage and consumed state boundary. Reconcile subsequent task events and external actions before selecting an older checkpoint.
4. Restore only that task's workspace and session. Load the recorded identity/skills/context revisions; an intentional refresh or model change must be recorded and checked for compatibility. Unrelated task transcripts are not automatically loaded. Shared memory may be selected under an explicit retrieval policy.
5. Check executor/session compatibility and current tool permissions. Restore in the declared mode; an adapter must positively verify load success before sending the next task prompt.
6. Create a new run under the same task, record source checkpoint and recovery mode, and continue. Lease loss must prevent publishing further effects or checkpoints.
7. Persist subsequent safe boundaries using the protocol above. A crash between boundaries may lose local progress; the reported recovery point must state this limitation.

External effects require idempotency or reconciliation independent of the checkpoint. Never blindly replay a dispatched tool call. If an external service cannot distinguish success from failure after a timeout, record uncertainty and require resolution. This extension does not promise exactly-once delivery across arbitrary services.

## Retention and memory

Required session/workspace artifacts are not prunable cold transcripts while an advertised recovery point depends on them. Keep them under a declared retention policy until the checkpoint is superseded or recovery is explicitly retired. An optional transcript may still be pruned. Core `snapshots/` remain regenerable graph projections and MUST NOT be redefined to mean required executor checkpoints.

Task events describe work history. Shared facts remain subject to the core admission rule: only reusable lessons, linked to their originating events, belong in facts. Checkpoints must not turn facts into transcript archives.

## Extension acceptance cases

An implementation claiming support MUST demonstrate: isolated restoration of two tasks; multiple runs under one stable task; successful session and semantic recovery; rejection of wrong-task/corrupt/missing artifacts; explicit handling of incompatible executor formats; preservation of uncommitted workspace edits; failed-save detection; crash after upload but before commit; duplicate publication; conflicting parent/fenced owner rejection; reconciliation of a sent-but-unacknowledged external action; recovery after container deletion; and portable export/import of all required artifacts. Core conformance tests alone do not establish execution recovery.
