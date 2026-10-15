# #59 real-browser acceptance: user-defined labels

Independent acceptance journey for issue #59 (Decision Atlas 1.1, 1.2, 1.3, 1.4, 3.3), written by
the acceptance author before any implementation, together with `tests/user_labels_db.rs` (API) and
`tests/frontend/corpus-labels-api.test.ts` (UI client). It checks the integrated local workspace;
the backend-dependent workspace is **not** expected on the standalone Pages build (#30).

Selectors are described by visible text/accessible names. The implementer may choose the
controls, but each observation below must hold.

## Setup

```bash
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/youtube_corpus   # any migrated DB
bun run build
cargo run -- migrate
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f tests/acceptance/user-labels-seed.sql
cargo run -- --no-open          # serves http://127.0.0.1:1420
```

Open `http://127.0.0.1:1420/?corpus=5900a000-0000-4000-8000-000000000001#corpora` (corpus
**Labels demo**). The seeded sources are fixtures: do **not** press _Check now_ on them.

Seed: monitored channel **Lecture channel** and playlist **Seminar playlist** both contain
**Shared lecture** (YouTube category `Education`, tags `history`, `lecture`, one transcript
segment "grace perfects nature"); **Channel-only lecture** is only in the channel;
**Directly added talk** (tag `talk`) was added directly.

## Journey and expected observations

1. **Catalog and memberships (1.1, 1.2).** The Videos list shows each of the three videos exactly
   once. _Shared lecture_ visibly names both _Lecture channel_ and _Seminar playlist_ as its
   sources; _Channel-only lecture_ names only _Lecture channel_; _Directly added talk_ is marked
   as directly added (no tracked source).
2. **YouTube metadata stays visible and distinct (1.4).** _Shared lecture_ shows its YouTube
   category/tags (`Education`, `history`, `lecture`) presented as YouTube-provided, visually
   distinct from user labels (for example a separate group or caption such as "YouTube tags").
3. **Create labels (1.3).** Create user labels `Theology` and `Watch later`. Creating `Theology`
   a second time shows an intelligible error (it already exists) and no second `Theology`
   appears. A blank name is rejected with a message.
4. **Assign on a source.** Assign `Theology` to _Lecture channel_. The source row in Sources shows
   the `Theology` label.
5. **Assign on a video.** Assign `Watch later` to _Directly added talk_. Its row shows the label,
   distinguishable from its YouTube tag `talk`.
6. **Filter by label (3.3).** Filter the Videos catalog by `Watch later`: only _Directly added
   talk_ remains. Filter by `Theology`: _Shared lecture_ and _Channel-only lecture_ appear (videos
   of the labeled source — derived rule, see `tests/user_labels_db.rs`), _Shared lecture_ once,
   _Directly added talk_ absent. Clearing the filter restores all three videos.
7. **Empty state.** Create `Unused` and filter by it: an intelligible empty state ("no videos with
   this label" or similar), not the full catalog and not an error.
8. **Change a label.** Rename `Theology` to `Church history`: the source row and filter options
   show the new name; the source keeps the label. Move _Directly added talk_ from `Watch later` to
   `Church history` (remove one, add the other).
9. **Remove labels (1.3, 1.4).** Remove `Church history` from _Lecture channel_, then delete the
   `Watch later` label. All three videos remain; _Shared lecture_ still shows both sources, its
   YouTube category/tags, `1 streams`/`1 segments`, and the search _grace perfects nature_ in
   "Search this corpus" still finds it.
10. **Same text, different origin.** Create user label `history` and assign it to _Directly added
    talk_. Filtering by the user label `history` shows only _Directly added talk_, not _Shared
    lecture_ (whose `history` is a YouTube tag).
11. **Persistence.** Reload the page: labels, assignments and the active corpus are unchanged.
12. **Error state.** Stop the server and trigger a label action: the UI shows an intelligible
    error instead of silently dropping the change.

Record a screenshot after steps 1, 6 and 9 for the PR.

## Not automatable here

- Real deduplication of a video discovered by two **live** YouTube sources through `Check now`
  needs network/yt-dlp; the seed reproduces the resulting stored state instead.
- Visual distinctness of user labels versus YouTube tags (step 2) is a reviewer judgment; the API
  tests only prove they are separate fields.

## Baseline on main 7fe61c6 (observed by the acceptance author)

Served the existing `dist/` against the seeded database with headless Chromium. Step 1 partly
holds: each video appears once. Nothing else does yet. The video rows show no source membership
and no YouTube categories or tags, and the workspace has no label controls.
`GET /api/corpora/{id}/labels` returns `404 API endpoint not found.`
