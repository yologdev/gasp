# GASP Log Segments Extension — `log-segments/1`

**Status:** DRAFT. An optional extension to GASP core v1. A repository that does not use it is unaffected; core conformance does not change. The conformance checker supports it from 0.3.0.

## 1. Problem

The event log `state/events.jsonl` is one append-only file that grows for the agent's whole life. Git hosts and runtimes bound a single file:
- Cloudflare Artifacts allows at most 32 MiB per file.
- A Worker isolate has 128 MB of memory.
- A host that stores each version whole stores the log once per commit.

A long-lived agent outgrows one file. Rotating the log is not allowed: moving lines out of `state/events.jsonl` deletes them, and check 4 fails that.

## 2. Layout

The log is a sequence of **segments**, read in order:

```
state/
├── events.jsonl        # segment 1 (the core log file)
└── events/
    ├── 000002.jsonl    # segment 2
    ├── 000003.jsonl    # segment 3
    └── ...
```

- **Names.** Segment `n` (for 2 ≤ `n` ≤ 999999) is `state/events/` followed by `n` as six zero-padded digits and `.jsonl`. Numbers are contiguous from 2. In a declared repository (§3), `state/events/` holds nothing else.
- **Format.** Every segment has the core log format: one event envelope per line, and each line ends with a newline, including the last line of the last segment. Every segment after the first holds at least one event.
- **The logical log.** The log is the concatenation of the segments in number order. All of core GASP is defined over the logical log:
  - the total order of events (Part I, "Ordering");
  - causation, which may name an event in an earlier segment;
  - the pairing rule, where an entity-creating event and its `state.ops_applied` may sit in different segments;
  - the replay fold;
  - snapshot integrity records (`line_count` counts logical lines; `sha256` hashes the logical log's first `line_count` lines).
- **The active segment.** Only the last segment is written to.
- **Sealing.** A segment is sealed by the first commit that adds the next segment. From then on its bytes never change.

When to start a new segment is the runtime's choice. It SHOULD start one before the active segment reaches its host's per-file limit. 8 MiB is a reasonable default. A new segment is started by committing the first events into the next file; nothing is moved.

## 3. Declaration

A repository with more than one segment MUST declare the extension in `AGENT.md`'s manifest block (its first fenced block), with a line of its own:

```yaml
spec_version: 1
log_segments: 1
```

A reader that does not support the extension would see only segment 1 and fold an incomplete history. The declaration lets a reader that does support it refuse such a repository rather than fold it silently. A checker or runtime that supports `log-segments/1` MUST treat segment files without the declaration as non-conformant. Declaring the extension with only one segment is allowed. Without the declaration, other files under `state/events/` are not this extension's concern, so an existing core repository checks as before.

## 4. Conformance (Part VI deltas)

With the extension, the checks change as follows. Every other check is unchanged.

- **Check 1 (envelope round-trip):** runs on the logical log, read from the working tree, which must be readable. Each of these fails:
  - segment files without the declaration;
  - a gap in segment numbers;
  - a stray file in a declared `state/events/`;
  - a segment after the first with no event;
  - a segment of a segmented log not ending with a newline.
- **Checks 2, 3, 5, 6 and 7:** run on the logical log.
- **Check 4 (append-only in git):** every segment that is tracked, or appears in any commit reachable from `HEAD`, is an append-only path, so deleting a segment fails as a deletion. In addition, for each segment `n` ≥ 2:
  - the first commit that adds segment `n` must already contain segment `n − 1`;
  - segment `n − 1`'s bytes at `HEAD`, and in the working tree, must equal its bytes at that commit. The sealing commit itself may still append to segment `n − 1`.

## 5. Runtimes and hosts

- **Restore** concatenates the segments, or folds them one after another; the result is the same. Restoring from a snapshot works unchanged, because its integrity record is defined over the logical log.
- **A host that validates each publication** and keeps an index of earlier event ids and kinds only has to read the active segment and any new segments. Sealed segments are compared by blob id.
- **Moving an unsegmented repository** to segments needs no rewrite: its `state/events.jsonl` becomes segment 1 and is sealed when segment 2 is first committed.
