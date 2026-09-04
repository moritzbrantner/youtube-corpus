# Discovery frontier and workflow orchestration

YouTube Corpus owns the open-ended discovery graph. The workflow stack owns the bounded execution of one frontier item at a time.

## Boundary

`youtube-corpus` owns:

- canonical YouTube video/channel/playlist targets
- discovery evidence and provenance
- admission scoring and maximum crawl depth
- deduplication and frontier state
- claiming, completing, and failing frontier items
- extracting new candidates from ingested video metadata, descriptions, and transcripts

`workflow-engine` owns workflow registration/versioning, triggers, run history, scheduling, and dispatch. `workflow-runner` owns the execution of one compiled workflow and its node retries/cancellation. Neither workflow repository should contain YouTube-specific domain records.

## Frontier lifecycle

```text
seed / ingested video
        |
        v
extract candidates
        |
        v
admission policy -- rejected/too deep --> deferred
        |
        v
      pending
        |
        | claim_next_discovery
        v
      claimed
        |
        | workflow run
        v
 ingest candidate ----> discovers more candidates ----> pending
        |
        +--> success --> completed
        |
        +--> terminal failure --> failed
```

Claims use a 15-minute lease by default. Expired claims are eligible for atomic re-claiming, so a worker crash does not permanently strand a target. Every successful claim increments `attempt_count`; the claim attempt is a fencing token that must be presented to `complete_discovery` or `fail_discovery`. Once another worker reclaims an expired target, terminal writes from the older attempt are rejected. Repeating the same terminal write for the same attempt is idempotent.

The `(kind, canonical_key)` uniqueness constraint makes discovery idempotent. `discovery_evidence` is append-only and records why a target exists, including the source video, parent discovery target, method, scores, depth, and evidence payload.

Failed or deferred targets can become pending again when later evidence satisfies the admission policy. Completed and currently claimed targets are not reopened by duplicate discoveries.

When a claimed target is ingested by a workflow run, ingest reuses that claimed target rather than treating its URL as a new depth-0 seed. A retryable ingest report does not transition the target to `failed`, and successful discovery expansion does not complete a workflow-owned claim. This preserves the actual crawl depth and leaves retry and terminal-transition timing with the workflow host.

Direct ingestion outside the discovery workflow may mark an unclaimed target completed after successful expansion. That update is guarded so it cannot overwrite a concurrently claimed target.

## Admission policy

The first policy is deliberately deterministic and local:

- maximum depth: 3
- minimum confidence: 0.5
- minimum priority: 0.35
- priority = `0.30 * confidence + 0.50 * relevance + 0.20 * novelty - depth penalty`
- depth penalty = `min(depth * 0.05, 0.25)`

The scores are corpus policy, not workflow-engine semantics. They can later be replaced by richer semantic relevance, source trust, cost, quota, or novelty models without changing the workflow contract.

## Workflow contract

`workflows/discovery-frontier.compiled.json` is a bounded pump workflow:

1. `youtube_corpus.frontier.claim`
2. branch on whether a candidate exists
3. `youtube_corpus.ingest`
4. `youtube_corpus.frontier.complete`
5. finish

A composition host registers these three YouTube-specific node executors with `workflow-runner` and registers the compiled document as `youtube-corpus.discovery.process` with `workflow-engine`.

Executor contracts:

- `youtube_corpus.frontier.claim`: claim the highest-priority pending target using `FOR UPDATE SKIP LOCKED`. Convert it with `workflow_input`; output `{ candidate, found }`, where `candidate` has `{ discoveryId, sourceKind, sourceUrl, depth, claimAttempt }`.
- `youtube_corpus.ingest`: convert `candidate.sourceKind` and `candidate.sourceUrl` into a `CorpusSource`, run the existing ingest pipeline, and carry `discoveryId` plus `claimAttempt` into its result. Ingest automatically records newly found targets while preserving the claimed target's depth. If every resolved item fails, the executor returns an execution error so `workflow-runner` can apply its retry policy; the corpus ingest hook itself does not mark the frontier item terminally failed.
- `youtube_corpus.frontier.complete`: call `complete_discovery` with the `discoveryId` and `claimAttempt` from the ingest result, then preserve the engine run id in `workflow_run_id`. A stale attempt cannot complete a target after it has been reclaimed.

If ingest reaches a terminal error after runner retries, the host calls `fail_discovery` with the same `discoveryId` and `claimAttempt` before returning the final failure. This keeps failure state corpus-owned while retry policy stays runner-owned.

The engine can trigger this workflow manually, on a cron schedule, or from a later queue-backed trigger. The DAG never recursively creates child nodes; recursion exists only as new durable frontier rows.

## Progressive processing

This slice reuses the existing ingest path for metadata, captions, optional ASR, persistence, and text indexing. Face/voice and later semantic analysis remain separate derived processing stages. Future workflows can add them after ingest without changing discovery identity or provenance.

## Compatibility

Ingest checks whether migration `0009_discovery_frontier.sql` is present before recording discoveries. Existing databases that have not migrated therefore keep the previous ingest behavior instead of failing mid-ingest.
