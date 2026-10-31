# YouTube Corpus Decision Atlas

This is the repository-owned, feature-oriented source for **what YouTube Corpus should do**. Entries describe product intent, not claims that a feature is implemented or independently accepted. The initial entries consolidate the owner's direction from 2026-10-13 and a previously documented evidence boundary (identified by its Status). Existing technical authority remains in [AGENTS.md](../AGENTS.md) and [.repository.toml](../.repository.toml).

## Editing rules

- **Decision** and **Question** are the only semantic leaf-entry kinds. Section headings group features; they are not a third kind.
- A Decision records owner-approved product intent. A `[question]` entry records an unresolved consequential choice and is not automatically a blocker.
- Keep entry IDs stable. Append new entries under the relevant feature; do not renumber, silently overwrite, or delete historical intent.
- When a Question is answered, retain it and append a Decision marked `Answers <ID>`. If intent changes, preserve the old Decision and append one marked `Supersedes <ID>`.
- Repository specifications and independently authored acceptance checks may elaborate the intended behavior, but must not silently change it. GitHub issues track implementation, not product authority.
- Existing producer boundaries matter: this corpus owns ingestion, stable media identities, aligned evidence, retrieval, and subscriptions; detector algorithms remain in their owning capability repositories. The legacy [roadmap](../ROADMAP.md) adds technical and downstream context, not a competing source of product decisions.

## 1. Tracking and organizing material

- **1.1 Track YouTube videos, channels, and playlists.**  
  Status: adopted 2026-10-13.  
  Users can register individual videos, channels, and playlists and browse the resulting known-video catalog. Source relationships remain visible, including when a video is found in multiple tracked sources.

- **1.2 Monitor collections for newly published videos.**  
  Status: adopted 2026-10-13.  
  A tracked channel or playlist can be checked for new items, so users need not repeatedly add each video manually. Already-known videos are recognized instead of becoming duplicate corpus identities.

- **1.3 Organize tracked sources and videos with user-defined categories or tags.**  
  Status: adopted 2026-10-13.  
  Users can assign and browse their own groups/labels. These assignments are separate from categories and tags supplied by YouTube; both should remain usable.

- **1.4 Preserve and use YouTube metadata.**  
  Status: adopted 2026-10-13.  
  Available metadata such as channel, playlist relation, title, description, publication date, supplied categories/tags, and engagement fields enrich discovery, browsing, filtering, and search. The original source of metadata remains distinguishable from user annotations.

- **1.5** [question] **What should happen when monitoring discovers a new video?**  
  Resolved 2026-10-13 by 1.6.  
  Should all newly discovered videos immediately run the applicable detector profile, should they be cataloged until the user explicitly approves analysis, or should each tracked source allow automatic versus review-first processing (with a chosen default)? Regardless of the policy, preserve discovery without duplicate corpus identities and distinguish known-but-unprocessed videos from analyzed ones.

- **1.6 Automatically process discovered videos by default, with review-first per source.**  
  Status: adopted 2026-10-13. Answers 1.5 (option C).  
  Each monitored channel or playlist offers an **automatic** (default) or **review-first** processing policy for newly discovered videos. Discovery always records the video and its source membership without introducing duplicate corpus identities. When a source uses automatic processing, a normal monitoring check can initiate the video's effective configured detectors (2.6 and 2.8), reusing unchanged prior evidence (2.2). When a source uses review-first, its newly discovered videos are visible as known/pending review and content analysis awaits user approval **unless** another tracked source already requests automatic processing for the same video under 1.8. The source's current policy and pending/processed status must be inspectable, and merely opening the app must not trigger surprise downloads or analysis.

- **1.7** [question] **What processing policy wins when sources overlap?**  
  Resolved 2026-10-13 by 1.8.  
  If the same video is discovered by one automatic source and one review-first source, should the automatic source trigger processing, should review-first hold processing until the user approves, or should the user choose an explicit per-video policy for this conflict? Keep one video identity and do not discard its source memberships.

- **1.8 Automatic processing wins when tracked sources disagree.**  
  Status: adopted 2026-10-13. Answers 1.7 (option A).  
  When the same canonical video belongs to both automatic and review-first tracked sources, **any automatic source is sufficient to initiate analysis**, without asking for another approval. Review-first mode delays automatic processing only while no applicable source requests it. Combine the effective detector requests from applicable profiles (2.8), reuse unchanged results (2.2), and process each identical video/input/detector/configuration once rather than once per source. Preserve all source memberships and show each source's policy alongside the video's actual pending/running/completed state. A video previously awaiting review can transition into automatic processing if later discovered by an automatic source, without duplicating the video or silently erasing its review history.

- **1.9** [question] **How much of a channel or playlist's existing history should be processed when first tracked?**  
  Resolved 2026-10-13 by 1.10.  
  When adding a source with hundreds or thousands of existing videos, should the app automatically work through the entire back catalog, track only videos published after the source is added, or let the user choose an initial backfill scope (for example: none, recent items, or entire history)? The answer determines the first-use experience and amount of analysis triggered; the approved source processing mode from 1.6 and overlap rule 1.8 still govern any selected items.

- **1.10 Configure the initial historical backfill when tracking a source.**  
  Status: adopted 2026-10-13. Answers 1.9 (option C).  
  Built scope: add-source history choice of none, recent days, first N videos in the source's displayed order (owner answer youtube-corpus#67.Q1) or full history; the discovery snapshot and its order are recorded per backfill (#67).  
  When registering a channel or playlist, users can choose which already-published videos to backfill: **none**, a **recent-history** selection, a **limited number** of videos, or the **entire available history**. Show the requested scope before starting historical discovery/analysis and keep that initial backfill choice distinct from ongoing checks for newly published videos. Videos selected for backfill obey the source's automatic or review-first processing policy (1.6), automatic precedence for shared videos (1.8), and effective detector profiles (2.6 and 2.8); previously known videos and unchanged results must not be duplicated (2.2). The initial scope must not silently truncate the user's requested full-history selection or trigger an unselected full backfill. The initial selection defaults to full history under 1.12.

- **1.11** [question] **What initial backfill option should be preselected?**  
  Resolved 2026-10-13 by 1.12.  
  When the user first adds a channel or playlist, should the default backfill selection be **none**, **recent/limited history**, or **full history**? Users retain the explicit alternatives in 1.10. This choice determines the risk of expensive surprise processing during first-time source setup.

- **1.12 Full available history is the preselected initial backfill.**  
  Status: adopted 2026-10-13. Answers 1.11 (option C).  
  Built scope: the corpus source form preselects full history; nothing is listed or processed until the source is added (#67).  
  The channel/playlist setup initially selects **all available historical videos** rather than none or a limited recent selection. The full-history scope is plainly visible and can be changed to any alternative from 1.10 **before the user adds the source**. Adding the source explicitly with this selected scope starts the requested historical discovery/backfill; merely opening the app must not trigger it. Do not silently impose the ordinary per-check item limit as a cap on full history. Respect the selected source's automatic or review-first processing policy (1.6), shared-video precedence (1.8), deduplication and per-stage reuse (2.2). Show the scope and progress truthfully, including provider restrictions or partially available history; never silently report an incomplete backfill as complete. Background execution and controls for extensive backfills follow 1.14 (Answers 1.13).

- **1.13** [question] **How should extensive initial backfills execute and be controlled?**  
  Resolved 2026-10-13 by 1.14.  
  When a newly added channel has thousands of videos and full history is selected, should the backfill run progressively in the background with visible progress and pause/resume/cancel controls, run as a blocking operation until complete, or process only user-launched batches? The decision should preserve the full-history selection (1.12), review-first policy (1.6), and idempotent reuse (2.2), without inventing a hidden item cap.

- **1.14 Run initial backfills progressively in the background, with user controls.**  
  Status: adopted 2026-10-13. Answers 1.13 (option A).  
  Built scope: server-owned background listing and processing with persisted progress, pause/resume/cancel and requeue of interrupted work on restart (#67).  
  Once a user adds a source with historical backfill selected (1.10 and 1.12), work advances **progressively in the background** rather than blocking the current page or requiring manually launched batches. Users can inspect honest per-backfill progress, including how much was discovered, queued, analyzed, completed, or failed, and can **pause, resume, or cancel** the backfill. Pausing prevents further backfill work until resumed; canceling stops the remaining requested work rather than deleting already-produced evidence or source memberships. Completed work and persistent progress remain available after leaving and reopening the page, and a resumed run reuses existing outputs without duplicate processing (2.2). The implementation should remain bounded and should report a partial or interrupted backfill rather than falsely labeling it complete. A full-history request must not be silently converted into a fixed-size sample. This preserves automatic versus review-first processing (1.6), automatic precedence (1.8), and producer boundaries.

- **1.15** [question] **How should new uploads compete with a long-running historical backfill?**  
  Resolved 2026-10-13 by 1.16.  
  If a tracked channel publishes a new video while its full-history backfill still has hundreds of videos remaining, should the new upload be prioritized, should the system interleave new and historical work fairly, or should the backfill retain precedence until it completes? The source's automatic or review-first policy remains in force for either kind of video.

- **1.16 Fairly interleave new uploads and historical backfill.**  
  Status: adopted 2026-10-13. Answers 1.15 (option B).  
  Built scope: one dispatcher alternates queued new uploads and running backfill videos; paused or canceled backfills contribute no work (#67).  
  When newly published videos arrive while a historical backfill is still running, the corpus **interleaves both classes of eligible work** so new uploads are handled promptly without waiting for the whole backfill, and the backfill continues to advance even while more new videos arrive. Neither category may be permanently starved by the other. Display enough per-source/backfill status to distinguish newly discovered, pending, running, and completed items. Respect each source's automatic/review-first policy (1.6), shared-video automatic precedence (1.8), pause/resume/cancel for backfills (1.14), and reuse/deduplication of unchanged evidence (2.2). A paused or canceled historical backfill does not itself pause checking or processing new uploads. This is a product-level fairness choice; no particular ratio, deadline, or extra scheduling service is approved here.

- **1.17** [question] **How should monitored channels and playlists be checked for new uploads?**  
  Resolved 2026-10-13 by 1.18.  
  Should monitoring use a shared periodic checking schedule for all enabled sources, offer a configurable checking interval per source (with a default), or be manual/on-demand unless a user separately runs a watch command? An explicit Check now action should remain available. This determines whether keeping the local corpus server running is sufficient to discover new uploads automatically.

- **1.18 Automatically check monitored sources on independently configurable intervals.**  
  Status: adopted 2026-10-13. Answers 1.17 (option B).  
  Every enabled, monitored channel or playlist has its **own configurable checking interval**, initially populated with a sensible default. With the local corpus server running, due checks happen automatically without a separate CLI watch process. Users can inspect and change each source's interval and see its last-check result and upcoming or overdue check, while retaining a **Check now** action. Disabled or unmonitored sources are not checked periodically. Repeated, overlapping, or retried checks must not create duplicate canonical videos or duplicate unchanged detector work (1.1, 1.2, 2.2). Newly discovered videos follow automatic/review-first processing (1.6 and 1.8) and fair interleaving with historical backfill (1.16); routine new-upload checks must not silently turn a selected limited/none initial backfill into full historical analysis (1.10). Source-check scheduling belongs to existing corpus-owned subscriptions and runtime, not a second monitoring service. The precise default interval is a routine configuration value, not an owner-approved numeric SLA.

- **1.19** [question] **What should happen to missed checks after the local server restarts?**  
  Resolved 2026-10-13 by 1.20.  
  If the server has been off for several days and a channel's next scheduled check was missed, should it check the source promptly on restart and then resume its normal schedule, simply start a new interval from the restart time, or replay every missed scheduled check? Preserve discovery deduplication and avoid redundant repeated checks.

- **1.20 Catch up once per overdue monitored source after restart.**  
  Status: adopted 2026-10-13. Answers 1.19 (option A).  
  When the local corpus server resumes after downtime, each **enabled, monitored source with a missed due check** is checked promptly **once**, regardless of how many scheduled intervals were missed. After that catch-up, the source returns to its independently configured monitoring interval (1.18). Do not replay every missed polling occurrence or defer the first overdue check until a full new interval elapses. Show accurate last-check outcome and subsequent due time; preserve source memberships and deduplicate overlapping/manual/retried checks, videos, and unchanged analysis (1.1, 1.2, 2.2). Disabled or unmonitored sources remain unscheduled. Newly found videos still follow their automatic/review-first policy (1.6 and 1.8) and fairness with historical backfill (1.16). The checks are server-owned; merely opening the browser UI is not a separate request to run a check.

## 2. Parsing and detector configuration

- **2.1 Make processing status visible per video and detector stage.**  
  Status: adopted 2026-10-13.  
  Users can tell whether a known video has been analyzed, which configured stages have completed, which are missing, running, skipped, or failed, and what output is available. A single opaque "parsed" flag is insufficient for a configurable analysis pipeline.

- **2.2 Reuse extraction results when inputs and configuration have not changed.**  
  Status: adopted 2026-10-13.  
  The normal path avoids parsing the same video repeatedly without reason. Previously produced results are reused for the same relevant input, detector/model, and configuration; changed inputs, explicitly requested refreshes, or newly enabled stages permit targeted reprocessing without unnecessarily rerunning unrelated stages.

- **2.3 Let users configure which detectors enrich a video.**  
  Status: adopted 2026-10-13.  
  The product offers configurable text, audio, and visual analysis capabilities rather than one fixed parse. Detector selection and its produced evidence are inspectable. The default scope and inheritance are defined by 2.6 (Answers 2.5).

- **2.4 Align spoken text, visible text, and scene evidence on the video timeline.**  
  Status: adopted 2026-10-13.  
  Transcripts, OCR of text displayed in the video, and useful scene boundaries can all contribute time-linked searchable evidence. Keep spoken words, on-screen text, raw observations, and derived evidence distinguishable rather than silently replacing one another. Reuse existing scene-detection capability where it helps; the corpus does not implement a second scene detector.

- **2.5** [question] **Where should detector settings apply by default?**  
  Resolved 2026-10-13 by 2.6.  
  Should a processing profile primarily be selected for a corpus/group, for each tracked source, or for individual videos? Inheritance and override behavior are not yet approved.

- **2.6 Detector profiles default to the corpus or group, with scoped overrides.**  
  Status: adopted 2026-10-13. Answers 2.5 (option A).  
  A corpus or research group supplies the common detector profile by default. Users can optionally override that selection for a tracked channel/playlist or an individual video. The effective detector selection for a video must be inspectable so users understand which analyses will run. Existing results should be reused, and a changed profile should not needlessly rerun unrelated unchanged stages (see 2.2). When multiple tracked sources/groups apply, combine their detector requests under 2.8.

- **2.7** [question] **How should overlapping source or group detector profiles be resolved?**  
  Resolved 2026-10-13 by 2.8.  
  When the same video belongs to multiple tracked sources or research groups whose detector configurations differ, should the applicable detectors be combined, should the user choose one effective profile, or should the corpus default prevail until a video override is set? An explicit per-video override remains supported under 2.6.

- **2.8 Combine detector requests from overlapping sources and groups.**  
  Status: adopted 2026-10-13. Answers 2.7 (option A).  
  When one video belongs to multiple tracked sources or research groups, its effective inherited detector plan includes all detectors requested by any applicable profile, instead of arbitrarily selecting one profile or asking the user to choose. Identical work for the same video/input, detector, producer/model revision, and effective configuration is reused rather than executed separately for each requesting source/group. Different requested configurations and their results remain distinguishable by provenance, not silently conflated. Users can inspect which detectors will run and which profiles requested them. Explicit per-video overrides from 2.6 remain available to customize the effective selection.

- **2.9** [question] **Where does an explicit per-video detector override apply when the video belongs to several corpora?**  
  Resolved 2026-10-15 by 2.10.  
  When one canonical video is a member of multiple corpora, a user-set per-video override (2.6) could apply only within the corpus where it was set, or to that video in every corpus.

- **2.10 Per-video detector overrides are scoped to the corpus where they were set.**  
  Status: adopted 2026-10-15. Answers 2.9.  
  An explicit per-video detector override applies **only within the corpus in which the user set it**. It does not change that video's effective detector plan in other corpora that also contain it; those corpora keep their own profiles and their own overrides. Identical work requested from several corpora is still reused under 2.2, and the effective plan shown in each corpus identifies the overrides that apply there.

## 3. Finding material inside videos

- **3.1 Search transcript words, phrases, and passages across the corpus.**  
  Status: adopted 2026-10-13.  
  Search results identify the matching video and relevant spoken passage and let the user return to the corresponding timestamp, rather than just locating a whole video.

- **3.2 Search text detected visually in videos.**  
  Status: adopted 2026-10-13.  
  OCR text from slides, signs, captions displayed in the picture, or other visible text becomes searchable with a jump to the relevant time/scene. The result identifies its visual source and remains distinguishable from a spoken transcript.

- **3.3 Filter evidence using both corpus organization and source metadata.**  
  Status: adopted 2026-10-13.  
  Search and browsing can narrow results by tracked sources, user-defined categories/groups, and relevant available YouTube metadata, while retaining source and timestamp context.

- **3.4** [question] **How should transcript and on-screen OCR search appear in the interface?**  
  Resolved 2026-10-13 by 3.5.  
  When users search text across their corpus, should one unified results list include both spoken passages and visible on-screen text with modality labels/filters, should transcripts and OCR have separate result views, or should transcript results appear by default with OCR only when explicitly enabled? All options retain timestamp links and source/provenance distinctions from 3.1–3.3.

- **3.5 Search spoken passages and visible OCR text together by default.**  
  Status: adopted 2026-10-13. Answers 3.4 (option A).  
  A single corpus text query searches **both transcript passages and on-screen OCR text by default** and presents their matches in **one unified results list**, not separate mandatory tabs or an opt-in visual-only mode. Each result clearly identifies whether its matching evidence was **spoken** or **visible in the video**, retains original video identity, correct timestamp/interval and corresponding context, and offers source-grounded navigation to that moment (3.1 and 3.2). Users can filter the unified results by modality and by the corpus/source metadata and user labels supported under 3.3, without losing the single-search experience. Preserve source evidence and producer provenance; an OCR observation is not silently treated as speech, and missing or unprocessed OCR must not be represented as evidence that no visible text exists. The default relevance-first ordering of individual matching moments is established in Decision 3.7 (Answers 3.6).

- **3.6** [question] **How should matching moments be ordered in unified text search?**  
  Resolved 2026-10-13 by 3.7.  
  Should the combined transcript/OCR results be ranked by cross-modality relevance in one sequence, grouped primarily by video with its matching moments beneath, or prioritize one modality when its relevance competes with the other? All approaches preserve the modality labels, timestamp links and filters from 3.5.

- **3.7 Rank matching moments from speech and OCR together by relevance.**  
  Status: adopted 2026-10-13. Answers 3.6 (option A).  
  The default unified search presents **individual transcript and on-screen OCR matches in a single cross-modality relevance order**. A relevant visual-text hit can rank above a less relevant spoken hit, and vice versa; neither modality has unconditional priority. Do not group results by video as the primary/default ordering. Retain each match's video, modality, source-grounded snippet, timestamp, and evidence/provenance context (3.5). Scores from different modalities must be meaningfully comparable rather than naively sorting incompatible producer-specific score scales. **Grouping by video may be considered later as an additional way to view results**, but is deferred and is not part of this Decision's initial acceptance or a separate implementation commitment.

- **3.8** [question] **Should a search match spoken and visible text in other languages?**  
  Answered 2026-10-13 by 3.9 with an owner-proposed processing-time translation strategy (not option A, B, or C). Other query-time cross-language mechanisms were not chosen.  
  When the user searches in one language but some tracked videos have transcripts or OCR in another, should search initially stay within the query language, use translated query variants to search other available languages while showing original evidence, or support cross-language semantic retrieval by default? Preserve the original source language, evidence and timestamps rather than silently replacing source text with translations.

- **3.9 Optionally generate and index translated text when parsing corpus videos.**  
  Status: adopted 2026-10-13. Answers 3.8 (owner's own alternative; no A/B/C selected).  
  Built scope: opt-in transcript/OCR translation stages through the native-whisperx OPUS-MT provider, derived translated records with provenance, and indexed translated hits in corpus search (#69).  
  Users can **enable translation generation as part of processing selected videos in a corpus**. For each enabled video, the configured translation stage generates **derived translated text** from supported original-language textual evidence, and the translations are **indexed for corpus search**. A user can then query indexed translations in their language, with results linked back to the corresponding original video, timestamp/interval, and original-language evidence. Preserve the original text and distinguish it from generated translations; keep source/target language and producing translation provider/model/configuration provenance inspectable. Translation remains optional rather than automatically required for every video, and unchanged translated outputs should be reusable under 2.2. The video processing/detector plan remains inspectable under 2.6/2.8. This decision does **not** assume that untranslated videos magically become cross-language searchable through query-time translation or multilingual semantic search. Translation of spoken transcript text and visible OCR text is independently configurable under 3.11; chosen target-language sets follow 3.13.

- **3.10** [question] **What video text should translation generation cover?**  
  Resolved 2026-10-13 by 3.11.  
  When translation is enabled for a corpus video, should it translate **spoken transcripts only**, **both spoken transcripts and on-screen OCR text** wherever available, or let users **configure the two separately**? All options preserve aligned original text and index generated translated evidence as required by 3.9.

- **3.11 Configure transcript and OCR translation independently.**  
  Status: adopted 2026-10-13. Answers 3.10 (option C).  
  Built scope: independent `translate_spoken`/`translate_ocr` profile settings with per-modality, per-target status (#69).  
  For each video, the effective translation processing plan allows users to **enable or disable translation of spoken transcript text and on-screen OCR text independently**. Users may request either, both, or neither; choosing one modality does not silently enable translation of the other. Reuse corpus/group defaults and optional source/video profile overrides from 2.6 and effective detector union from 2.8, rather than creating a second independent processing-policy hierarchy. Translate only the selected modality's actually available source-language evidence, retaining alignment to its original video intervals and distinct modality, original text, source/target language and producer/model/configuration provenance as in 3.9. Index successfully generated translations with their derived-from-speech or derived-from-OCR origin visible in unified search (3.5 and 3.7); missing, unsupported or failed source evidence is not a completed translation. Reuse unchanged work and do not rerun unrelated modality stages (2.2). This settles **which text modalities may be translated**; the independently chosen target languages follow 3.13.

- **3.12** [question] **How should target languages be selected for translated video evidence?**  
  Resolved 2026-10-13 by 3.13.  
  When translation is enabled, should a corpus/group define **one default target language** (with optional source/video overrides), allow **multiple chosen target languages** in inherited profiles and overrides, or automatically generate translations into **every supported target language**? The result should remain configurable and inspectable without changing original-language evidence, and should not assume every provider supports every pair.

- **3.13 Choose multiple translation target languages in inherited processing profiles.**  
  Status: adopted 2026-10-13. Answers 3.12 (option B).  
  Built scope: inherited target-language sets with deduplicated per-span translations; supported languages are en, de, fr, es, it, pt, nl and pl (#69).  
  Users can select **one or more target languages** for generated translated evidence, with the **corpus/group processing profile** providing the normal set and optional **channel/playlist and per-video overrides** under 2.6. Each video's effective translation plan must show the target languages actually requested and which applicable profiles requested them; overlapping sources/groups combine applicable translation requests under 2.8 without duplicating identical video/input/modality/target-language/model/configuration work (2.2). Transcript and OCR translation remain independently selectable under 3.11; enabling one does not force the other. Generate and index only the **chosen, supported** language outputs, preserving original-language text, modality, timestamps, and translation provenance under 3.9. Do not silently translate into every supported language, invent a universal default target language, or treat an unavailable language pair as successfully translated. The specific initially selected languages and provider availability are not owner-approved here.

- **3.14 Promote an optional Tantivy search provider only after demonstrated superiority.**  
  Status: adopted 2026-10-20 (owner choice B, [#76](https://github.com/moritzbrantner/youtube-corpus/issues/76)).  
  Start Tantivy through Search Engine as an opt-in local lexical provider. Keep the existing search backend as default until representative real-corpus comparisons show meaningful improvements in retrieval quality, correctness and performance while preserving exact source provenance, filters and navigation. Do not claim hybrid semantic/OCR fusion parity from lexical evidence; promote only after demonstrated non-regression in the supported scope. Preserve a documented fallback during evaluation.

## 4. People, places, and other entities

- **4.1 Record source-grounded entity occurrences.**  
  Status: adopted 2026-10-13.  
  Detect references to people, places, and other entities in speech, on-screen text, and other supported producer evidence. Every occurrence retains its video, timestamp or interval, evidence modality, and producer context. A name being mentioned is not the same event as a person being visually shown.

- **4.2 Explore entity evidence across videos.**  
  Status: adopted 2026-10-13.  
  Users can search for an entity and inspect which videos contain mentions, appearances, or other observations, with counts or occurrence summaries and direct links to the supporting moments. The modalities remain distinguishable in the results.

- **4.3 Do not silently turn uncertain matches into verified identities.**  
  Status: existing repository evidence boundary (see `docs/architecture.md`, Multimodal Analysis Boundary).  
  Cross-video face/voice similarity clusters and textual mentions are evidence, not automatic proof that the system identified the same real-world person. Preserve provenance and uncertainty. Decision 4.5 permits explicitly marked inferred identity links, not a silent promotion to verified real-world identity.

- **4.4** [question] **How should possible person matches become one cross-video identity?**  
  Resolved 2026-10-13 by 4.5.  
  Should the corpus only link explicitly confirmed people, suggest uncertain links for confirmation, or automatically combine high-confidence matches while exposing uncertainty? Until decided, keep candidate matches separate from verified person identities.

- **4.5 Automatically link high-confidence matches, with inspectable uncertainty and correction.**  
  Status: adopted 2026-10-13. Answers 4.4 (option C).  
  Built scope: automatic face matching with reversible researcher corrections (#64); cross-video voice matching into the same corpus people, with the same corrections, and co-occurrence-based face-to-voice link suggestions that merge two people only when a researcher confirms them (#134). Name attribution is separate work (4.7).

  The corpus may automatically associate high-confidence matching face/voice observations or clusters across videos into a shared **inferred, corpus-local person identity**, without requiring user confirmation for every match. These matches are visibly identified as automatic inferences rather than verified real-world identities; retain confidence, source observations, timestamps, and producing algorithm/model/configuration provenance. Users can inspect the supporting evidence, correct, unlink or split incorrect associations, and see updated cross-video results without destroying the original evidence. Reprocessing must not silently reinstate a user-rejected association. Low-confidence or contradictory evidence remains unresolved rather than forcing a match. Similarity alone does not justify assigning a real-world person's name; evidence-grounded name attribution is governed by 4.7 (Answers 4.6).

- **4.6** [question] **When may text mentions give an inferred person a real-world name?**  
  Resolved 2026-10-13 by 4.7.  
  Should the corpus avoid automatically attaching a spoken/written person's name to a face/voice identity, attach one only when additional time-aligned evidence supports that link, or infer a name from simple co-occurrence in the same video? Such an attribution is separate from recognizing repeated face/voice observations as an inferred individual (4.5).

- **4.7 Automatically infer person names only from strong contextual evidence.**  
  Status: adopted 2026-10-13. Answers 4.6 (option B).  
  Built scope: on-screen name tags aligned with one visible face (#65); spoken self-introductions ("I'm …", "my name is …", "Hi, this is …", German equivalents) attributed to one voice track of a matched voice person, where a host's guest introduction never names the speaker (#135); a clear self-introduction on an unmatched single-video voice track forms a suggested single-video person for that track (#164). Both stay inferred until a researcher confirms them.  
  An inferred corpus-local face/voice identity may be associated with a real-world person's name automatically **only when specific time-aligned, source-grounded contextual evidence supports that association**, such as an on-screen speaker name linked to the visible person, or a clear self-introduction attributable to the detected speaker. Merely mentioning a name somewhere in the same video or near a face is insufficient. Preserve the exact supporting transcript/OCR/audio/visual spans, modality, confidence, producer and revision; present the name as an **evidence-supported inference**, never silently as a verified real-world identity. Weak, ambiguous, homonymous or conflicting evidence must leave name attribution unresolved. Users may confirm, correct or remove an inferred name, and later processing must not silently reinstate a user-rejected attribution. Keep naming evidence and associations distinct from the raw observations and cross-video similarity links of 4.5.

## 5. Research workspaces and global identities

- **5.1 Navigate by object type with an active corpus scope.**  
  Status: adopted 2026-10-18 (owner's multi-page UI direction).  
  Provide separate usable index/detail pages for corpora, channels and other sources, videos, people, entities, historical events, and processing, plus a dedicated search page and deep-linked video moments. A corpus selection page chooses the default research scope; the selected corpus remains visible and switchable when moving among pages. Search filters, corpus scope, sorting, pagination and meaningful selections survive browser reloads/back/forward via shareable URLs. A selected result resolves to original timestamped evidence, not an independently duplicated search-result record. A global/all-corpora scope permits starting from a person or entity rather than from a corpus. Preserve the existing standalone browser analyzer as a distinct capability; do not show local database-only workspaces as functional on static Pages without their backend.

- **5.2 Use shared canonical identities across corpora, with local evidence retained.**  
  Status: adopted 2026-10-18 (owner choice A). Clarifies the **corpus-local scope** of 4.5, not its approved within-corpus inferred face/voice matching behavior.  
  A recognized real-world referent (person or other named entity) may have a stable identity discoverable across corpora; its individual mentions, detections, annotations, inferred person clusters and processing evidence remain attributable to the originating videos/corpora. The active corpus is a default filter, not a boundary on canonical identity. Model People as a dedicated research perspective over person entities, not another competing identity authority. Preserve existing corpus-local inferred identities from 4.5 as evidence or candidate associations; do not automatically promote similarity clusters, same-name mentions, or unverified name inferences into a globally confirmed person. A migration must preserve existing corpus memberships, raw observations and corrections.

- **5.3 Resolve global identity links conservatively and reversibly.**  
  Status: adopted 2026-10-18 (owner choice A); applies to the shared-identity links of 5.2.  
  Automatically link clear matches supported by reliable identifiers or comparably decisive, source-grounded evidence. Show uncertain associations as suggestions to confirm or reject rather than silently merging canonical identities. Support correction, merging and splitting; preserve original observation/claim provenance and negative user decisions across repeated processing. Matching names alone is insufficient. Corpus-local inferred similarity associations remain visibly inferred as in 4.3–4.5.

- **5.4 Show unified, filterable person/entity evidence and a timeline.**  
  Status: adopted 2026-10-18 (owner choice A, explicitly augmented with the timeline).  
  Person and entity detail pages combine source-grounded occurrences in a filterable evidence view and a timeline visualization over the same evidence. Distinguish mentioned in speech, shown on screen, identified speaker, OCR mention, and metadata reference when produced; never equate a mention with an appearance or a speaker identification. Each occurrence opens its video and exact moment, with producer, uncertainty and corroboration visible as applicable. Use the same corpus/modality filters in the list and timeline, with a structured accessible alternative to charts.

- **5.5 Offer separate publication and historical timelines.**  
  Status: adopted 2026-10-18 (owner choice C).  
  The publication timeline organizes evidence by the videos' publication dates; it does not equate publication with the time of described events. The historical timeline organizes source-backed claims about event times, with approximate dates, ranges, BC/AD and uncertainty where available. Never silently combine the two chronological axes. Both modes link to original video moments and retain the selected corpus filter. Missing or unresolved event dates remain visibly unknown rather than being assigned an invented timestamp.

## 6. Corpus processing workspaces

- **6.1 Share reusable pipeline definitions with corpus-specific overrides.**  
  Status: adopted 2026-10-18 (owner choice A).  
  Reuse pipeline definitions across corpora while permitting independently configured per-corpus parameters and stage enable/disable overrides, with independent execution/run histories. Keep the existing corpus-owned detector plan, processing and provenance authoritative until an actual integration proves parity. Use the existing Workflow Editor for authoring and Workflow Engine/Runner capabilities for general workflow orchestration/execution where their contracts fit; do not create a competing workflow authoring or generic scheduling/runtime implementation in this repository.

- **6.2 Open Processing on an operational pipeline overview.**  
  Status: adopted 2026-10-18 (owner choice A).  
  The selected corpus's Processing page initially shows real pipeline stages, enabled/overridden settings, running/failed/completed work, inspectable attempts and actionable retry or refresh controls. The full Workflow Editor opens only when authoring/editing is needed. Keep operational details and links to the affected video/source next to the relevant stage or run; avoid an always-visible graph editor, fabricated state, duplicate settings controls, and decorative dashboard counters.

## 7. Historical events and source-backed claims

- **7.1 Extract dated event claims automatically, preserving review and uncertainty.**  
  Status: adopted 2026-10-18 (owner choice A).  
  When supported transcript and OCR evidence is processed, extract dated historical-event claims using a suitable established producer capability, without reimplementing general linguistic inference locally. Show extracted claims on historical timelines immediately as **unverified claims**, not established facts. Each claim retains its source video, exact speech/OCR span and timestamp, producer/context and stated or inferred date (including ambiguous/approximate ranges). Allow confirming, correcting, rejecting and merging claims without destroying original evidence. Processing failures or missing OCR do not assert that no historical events exist.

- **7.2 Group claims by event while preserving disagreements.**  
  Status: adopted 2026-10-18 (owner choice A).  
  Represent a potentially shared historical event separately from the distinct assertions made about it by different sources. Keep each dated assertion's provenance, qualifications and contradictions inspectable; support reviewing event grouping, splitting mistaken groups, and choosing a reviewed preferred interpretation without deleting alternative accounts. Never synthesize a single supposedly authoritative historical fact merely by averaging or merging contradictory claims. Coordinate historical claim semantics with the existing evidence-first historical-source-atlas model without transferring ownership of video evidence or making a new cross-repository runtime dependency mandatory.

- **7.3 Provide a dedicated historical Events workspace.**  
  Status: adopted 2026-10-18 (owner choice A).  
  Offer an Events directory and a stable event detail URL, navigable independently of a person/entity profile. The detail view links proposed dates or ranges, source-specific claims/disagreements, associated people, organizations, places, related videos and exact evidence moments. Filtering by current corpus or all corpora must not produce separate canonical event identities. The Events workspace reuses event/claim evidence and timeline projections rather than building another ontology or duplicating stored source material.
