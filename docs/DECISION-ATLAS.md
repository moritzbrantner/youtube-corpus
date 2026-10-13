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
  When registering a channel or playlist, users can choose which already-published videos to backfill: **none**, a **recent-history** selection, a **limited number** of videos, or the **entire available history**. Show the requested scope before starting historical discovery/analysis and keep that initial backfill choice distinct from ongoing checks for newly published videos. Videos selected for backfill obey the source's automatic or review-first processing policy (1.6), automatic precedence for shared videos (1.8), and effective detector profiles (2.6 and 2.8); previously known videos and unchanged results must not be duplicated (2.2). The initial scope must not silently truncate the user's requested full-history selection or trigger an unselected full backfill. The preselected/default initial scope remains unresolved in 1.11.

- **1.11** [question] **What initial backfill option should be preselected?**  
  When the user first adds a channel or playlist, should the default backfill selection be **none**, **recent/limited history**, or **full history**? Users retain the explicit alternatives in 1.10. This choice determines the risk of expensive surprise processing during first-time source setup.

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
  The corpus may automatically associate high-confidence matching face/voice observations or clusters across videos into a shared **inferred, corpus-local person identity**, without requiring user confirmation for every match. These matches are visibly identified as automatic inferences rather than verified real-world identities; retain confidence, source observations, timestamps, and producing algorithm/model/configuration provenance. Users can inspect the supporting evidence, correct, unlink or split incorrect associations, and see updated cross-video results without destroying the original evidence. Reprocessing must not silently reinstate a user-rejected association. Low-confidence or contradictory evidence remains unresolved rather than forcing a match. Similarity alone does not justify assigning a real-world person's name; evidence-grounded name attribution is governed by 4.7 (Answers 4.6).

- **4.6** [question] **When may text mentions give an inferred person a real-world name?**  
  Resolved 2026-10-13 by 4.7.  
  Should the corpus avoid automatically attaching a spoken/written person's name to a face/voice identity, attach one only when additional time-aligned evidence supports that link, or infer a name from simple co-occurrence in the same video? Such an attribution is separate from recognizing repeated face/voice observations as an inferred individual (4.5).

- **4.7 Automatically infer person names only from strong contextual evidence.**  
  Status: adopted 2026-10-13. Answers 4.6 (option B).  
  An inferred corpus-local face/voice identity may be associated with a real-world person's name automatically **only when specific time-aligned, source-grounded contextual evidence supports that association**, such as an on-screen speaker name linked to the visible person, or a clear self-introduction attributable to the detected speaker. Merely mentioning a name somewhere in the same video or near a face is insufficient. Preserve the exact supporting transcript/OCR/audio/visual spans, modality, confidence, producer and revision; present the name as an **evidence-supported inference**, never silently as a verified real-world identity. Weak, ambiguous, homonymous or conflicting evidence must leave name attribution unresolved. Users may confirm, correct or remove an inferred name, and later processing must not silently reinstate a user-rejected attribution. Keep naming evidence and associations distinct from the raw observations and cross-video similarity links of 4.5.
