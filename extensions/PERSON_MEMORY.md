# Person and Multimodal Memory Extension

**Status:** DRAFT — optional profile `gasp.person-memory/v0`. This is a proposal for review, not a claim of runtime support. Core GASP event kinds, replay, and conformance remain unchanged.

## Purpose

An agent may meet the same person across conversations, accounts, photos, audio, and video. It should be able to carry forward useful, correctable knowledge without treating a visual resemblance as an identity proof or turning every encounter into a permanent fact.

GASP can provide the portable *history of how a memory was learned and corrected*. The executor still performs perception, identity verification, retrieval, and access control. A clone of a public GASP repo must not be mistaken for a complete backup of private person memory.

## Boundaries

- **Subject:** a stable, opaque ID scoped to the agent's private memory domain. A subject may be a person or a fictional/public character; those are different kinds. A name, handle, face, or costume is not the ID.
- **Binding:** an account or contact may be linked to a person only with recorded verification, such as an authenticated account interaction or explicit confirmation. A visual match alone may suggest a candidate, but cannot create or merge person identities.
- **Observation:** a sourced, time-bounded account of one encounter. It records what was visible, heard, or said and what portion of the source was examined. It is not a durable claim about every future encounter.
- **Fact:** a distilled item useful in later interactions, with source lineage and a validity/scope decision. Not every observation earns a fact in `memory/facts.jsonl`; the core [memory admission criterion](../SPEC.md#memory--facts-are-distilled-never-a-history-mirror) still applies.
- **Card:** a retrievable projection of current bindings and live facts for one subject. It is not an additional authority that can silently override its sources.

For example, “a purple crowned mascot appears in four frames of this post” is an observation. “This is the same real person as account X” is a separate identity claim requiring separate evidence. “This colleague prefers concise release summaries” may become a scoped fact if the colleague said so or confirmed it; the picture did not establish that preference.

## Proposed profile record

The profile may be carried in private observation metadata and attached artifacts using existing GASP events. This illustrative record is **not** a new core event envelope:

```json
{
  "profile": "gasp.person-memory/v0",
  "subject_id": "subject_opaque_7c2f",
  "subject_kind": "person",
  "source_event_id": "event_observation_42",
  "source": {"kind": "conversation", "id": "message_123", "observed_at": "2026-09-29T11:17:28Z"},
  "modality": "video",
  "coverage": {"kind": "sampled_frames", "positions_seconds": [0, 4.1, 8.3, 12.4]},
  "claim": "A person wearing a purple crowned hood is visible near a desk",
  "confidence": "possible",
  "identity_link": "unconfirmed",
  "evidence": {"artifact_id": "artifact_42", "revision": "sha256:..."},
  "scope": "private:family"
}
```

The actual GASP emitter uses `observation.created` plus its required paired `state.ops_applied` event for a graph observation. It may attach a content-addressed evidence artifact. A private, append-only fact record uses the core `id`, `ts_ms`, `text`, `derived_from`, and `supersedes` fields; its `derived_from` values point to the observations or runs that taught it. The card is regenerated from the latest authorized bindings and unsuperseded facts. A future Pack could add first-class subject nodes if real queries need them; this draft does not require that change.

Each observation needs a source identifier, observation time, modality, bounded coverage, the asserted detail, confidence, analyzer or human provenance, and an artifact revision when evidence bytes are retained. Text in an image or transcript remains source content, not an identity attestation or instruction. A past sighting is context for a new scene, not proof of what the new scene contains.

## Correction and learning

An identity binding and a remembered fact have different lifecycles. A person may correct a preference without changing their identity; a mistaken account or visual link may be revoked without deleting every valid conversation observation.

Corrections append a sourced replacement or retraction and supersede the earlier claim. Queries and card projections show the current result and can trace it back to both the original and the correction. The agent must not continue using a superseded claim because an old card or model checkpoint still contains it. Automatic learning may propose a candidate fact; promoting it to a durable personal fact requires a source appropriate to the claim. Sensitive inferences should not be promoted from appearance, tone, or a single ambiguous exchange.

## Privacy, portability, and restore

Person records are private by default. A public GASP repo must not contain private names, handles, conversation text, image bytes, face embeddings, personal facts, or correlatable source references. A hash or URI is not automatically safe to publish: it can reveal a known image or expose an accessible object. Family and work scopes must stay separate unless an authorized person explicitly connects them.

A deployment can keep the profile in a separate access-controlled GASP repo or private artifact store. **A pointer to one vendor's private bucket is provenance, not portability.** An implementation may claim portable person memory only if an authorized restore can obtain the scoped records and required artifacts, verify their integrity, replay corrections, and rebuild cards on another runtime. If an artifact is missing or access is denied, the new runtime must report the memory as unavailable rather than reconstruct it from a label or generated reply. A runtime that ignores this optional profile remains core-GASP conformant but cannot claim the profile's restore guarantee.

An append-only git history complicates deletion of personal information. This draft does not promise erasure from a public or replicated repo. Before storing real family or workplace data, an implementation needs an explicit retention, export, correction, revocation, and deletion design for its private storage and backups. Raw media and embeddings should be retained only when needed for the agreed use; the event log can usually keep a smaller sourced observation.

## Runtime responsibilities

The runtime decides when to inspect media, selects candidate reference images, verifies account bindings, enforces read permissions, and retrieves only relevant card details for a task. It must keep the current source in view: a confirmed past identity link does not justify inventing an action in a new video. The runtime should be able to answer “I cannot identify this confidently” when the evidence is weak. None of these operations is supplied by GASP's event reducer.

## Validation before claiming support

1. Restore a private profile and its content-addressed artifacts on a different runtime; reproduce the current card from events and facts.
2. Keep two people with similar appearances, changed handles, or shared costumes separate until an independently verified binding exists.
3. Correct a mistaken sighting and a preference; verify old claims remain traceable but no longer appear in current context.
4. Deny a work-scoped reader access to family memory and ensure a public export reveals no personal payload or correlatable reference.
5. Handle a missing artifact, partial video coverage, and an unavailable private resolver without fabricating identity or continuity.
6. Exercise export, revocation, backup retention, and the chosen deletion mechanism before using real private data.

## Open design questions

- Should private person memory live in one encrypted agent repo, separate scope-specific repos, or a portable artifact bundle referenced by a minimal private event log?
- Which identity-binding evidence is sufficient for automatic linking, and which links always require a person to confirm them?
- How should an authorized correction or deletion propagate to replicas and old backups without overstating what append-only storage can erase?
- At what scale do person cards need a Pack with first-class subject nodes rather than an indexed profile projection over existing observations and facts?
