# Yoyo activity page

GASP events supply evolution history, task recovery checkpoints, and canonical
burn receipts. The page refreshes this log every five minutes.

The read-only `/api/activity` projection refreshes every minute in visible tabs
and caches successful responses for 30 seconds. Private Cloudflare service
bindings expose confirmed X publications and treasury receipts through an
explicit public-field whitelist. No drafts, session memory, credentials,
policy, or token mint addresses are returned.

Twitter counters count unique confirmed publications in the current service,
not incoming mentions or checkpoint sessions. Treasury announcements are shown
under Treasury. The pending count covers unconsumed messages from the last
24 hours. Publication lists contain at most 500 recent items; counters cover
all confirmed publications retained by the service.

Treasury shows confirmed burns, spending including costs, and creator-fee
receipts. Transaction signatures deduplicate live and GASP receipts. Headline
amounts are rounded for readability; hovering shows full precision, and event
text retains exact amounts. Unavailable live sources are labeled explicitly.

Validation: `node --test site-yoyo/tests/*.cjs` from the repository root.
Deploy from this directory with Wrangler; service bindings are in
`wrangler.jsonc`.
