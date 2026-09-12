# Per-run Context Provenance

Status: DRAFT, optional profile `yoyo.context/v1` for task execution recovery metadata. Core event kinds, reducers, and evolution state are unchanged. This profile describes the Cloudflare Yoyo harness; other implementations may ignore these optional fields without claiming support for their guarantees.

## Separate recovery from evidence refresh

A task checkpoint preserves execution lineage, artifacts and action outcomes. It is not an assertion that every instruction, retrieved fact or transcript in that checkpoint remains current. Each resumed run must identify whether it restored the serialized session or reconstructed semantic context. A fresh-context semantic run must not be reported as full session continuation.

For a provided-harness semantic run, the host supplies current versioned instructions and selects relevant task records. A conversation may retain one stable task identity while each reply receives only its actual parent chain and the addressed user's scoped memory. Sibling branches and another participant's private memory must not enter the new prompt merely because they share an executor checkpoint.

## Optional metadata

The private recovery manifest's `context` may include:

- `provided_system_hash`: SHA-256 of the current supplied instructions and context.
- `refresh_policy`: `fresh-harness-per-run` when a new supplied prompt replaces the prior system text.
- `parent_selection`: `explicit` or `serialized_latest`. The latter means the host selected a concrete committed parent under exclusive task ownership. It does not relax the canonical writer's parent comparison or fencing requirements.
- `retrieval`: a `yoyo.context/v1` record containing the verified content bundle ID, source commit, and retrieval diagnostics. Individual evidence references include source, reference, revision and observation/export time. Missing sources and incomplete ancestry are explicit.

The checkpoint manifest still records its exact parent checkpoint. A recovery lineage's state boundary and newly retrieved per-run source versions have distinct meanings; neither should be silently substituted for the other.

## Trust and persistence

Retrieved documents and conversations are evidence, never execution permissions. Historical generated answers are records of what an agent said, not independent primary evidence for those claims. Current balances and totals require fresh authoritative data; missing live evidence cannot be silently replaced by an old generated answer.

Personal transcripts, private source content and credentials must not be copied into public graph metadata. Credentials must not enter model context or checkpoints. Protected external artifacts require the existing authenticated resolver, retention and export/import mechanisms. Knowledge admission and reusable facts remain separate from task checkpoints and evolution promotion.

## Validation

Verify two runs under one stable task, replacement of stale supplied instructions, private sibling isolation, explicit incomplete-parent handling, refreshed dynamic facts, unavailable-source behavior, unchanged explicit-parent rejection, and separate idempotency/reconciliation of external actions. Structural citation or hash validation alone does not establish the semantic truth of a compiled knowledge page.
