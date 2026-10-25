# #64 real-browser acceptance: cross-video face matching

Retrospective independent acceptance for issue #64 (face-only split; Decision Atlas 4.3, 4.5),
written from the issue's approved intent after PR #97 merged, together with
`tests/face_matching_acceptance_db.rs` (Postgres/API). It checks the integrated local workspace;
the biometric corpus store is **not** available on the static Pages build.

Selectors are visible text and accessible names.

## Setup

```bash
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus   # any migrated DB
bun run build
cargo run -- migrate
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f tests/acceptance/face-matching-seed.sql
cargo run -- --no-open          # serves http://127.0.0.1:1420
```

Open `http://127.0.0.1:1420/?corpus=6400a000-0000-4000-8000-000000000001#corpora` (corpus
**Face matching demo**). The seed stores completed faces-detector runs in the pinned YuNet/SFace
producer shape with synthetic, non-biometric 128-dimensional embeddings (see the seed header).
The matcher normally runs after face processing; here _Recheck face matches_ runs that same step
over the seeded output.

## Journey and expected observations

1. **Automatic grouping.** Press _Recheck face matches_. Without any approval dialog, the _Face
   matches_ panel shows two people, each badged _Inferred · not verified_: one with _Lecture one_,
   _Lecture two_ and _Interview_ ("3 videos · 3 face tracks"), one with _Panel_ and _Debate_.
2. **Uncertain evidence stays apart.** _Unresolved face tracks_ lists _Q&A_ ("No unique
   high-confidence match in another video": it is equally similar to both people) and _Short
   clip_ ("Insufficient sampled face evidence…"). Neither appears in a person. The ambiguous
   _Q&A_ face does not prevent the two clear people from forming.
3. **Evidence and timestamps.** Each track shows its minimum cosine similarity, sampled
   observation count and detector score. Its link opens the video at the exact second
   (`t=12`, `40`, `95`, `5`, `7`). _Evidence and provenance_ shows producer
   `youtube-corpus/faces`, model `opencv-yunet+opencv-sface@yunet:…;sface:…`, the input identity
   and the matching algorithm/configuration `youtube-corpus/face-complete-link-v1`.
4. **Occurrences.** _Open person occurrences_ for the _Lecture one_ person lists three videos.
5. **Split.** Select _Interview at 1:35_ and press _Split selected into new person_. The person
   immediately shows "2 videos · 2 face tracks"; its occurrences drop to two videos. The
   _Interview_ track is still shown, marked _Researcher correction preserved_.
6. **Unchanged rerun.** Reload, press _Recheck face matches_ again. _Interview_ does not rejoin
   the _Lecture one_ person, no duplicate people appear, and _Q&A_ is still unresolved.
7. No page errors and no requests leave `127.0.0.1`.

## Result (2026-10-25, branch `agent/face-matching-acceptance`)

Headless Chromium (Playwright 1.63) passed all 23 observations above against the seeded
database. Screenshots after steps 3, 5 and 6 are kept locally, not committed.

On `main` a025f84 the Postgres acceptance test
`high_confidence_repeats_group_automatically_while_weak_ambiguous_and_foreign_evidence_stays_apart`
failed: the single ambiguous _Q&A_-style face linked both people into one similarity component,
and the matcher left **every** track of that component unresolved, so no person formed. The same
branch repairs the planner (margin-guarded complete-link agglomeration); the other four
acceptance cases already passed on `main`.

## Not covered here

- The real YuNet/SFace detector pipeline: the opt-in native check from PR #97 covers it.
- External YouTube playback of the timestamp links and real-world identity accuracy.
- Voice matching and face-to-voice bridges (outside the approved face-only split of #64).
