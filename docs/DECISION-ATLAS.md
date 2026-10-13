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

## 2. Parsing and detector configuration

- **2.1 Make processing status visible per video and detector stage.**  
  Status: adopted 2026-10-13.  
  Users can tell whether a known video has been analyzed, which configured stages have completed, which are missing, running, skipped, or failed, and what output is available. A single opaque "parsed" flag is insufficient for a configurable analysis pipeline.

- **2.2 Reuse extraction results when inputs and configuration have not changed.**  
  Status: adopted 2026-10-13.  
  The normal path avoids parsing the same video repeatedly without reason. Previously produced results are reused for the same relevant input, detector/model, and configuration; changed inputs, explicitly requested refreshes, or newly enabled stages permit targeted reprocessing without unnecessarily rerunning unrelated stages.

- **2.3 Let users configure which detectors enrich a video.**  
  Status: adopted 2026-10-13.  
  The product offers configurable text, audio, and visual analysis capabilities rather than one fixed parse. Detector selection and its produced evidence are inspectable. The default scope and inheritance of detector settings remain open in 2.5.

- **2.4 Align spoken text, visible text, and scene evidence on the video timeline.**  
  Status: adopted 2026-10-13.  
  Transcripts, OCR of text displayed in the video, and useful scene boundaries can all contribute time-linked searchable evidence. Keep spoken words, on-screen text, raw observations, and derived evidence distinguishable rather than silently replacing one another. Reuse existing scene-detection capability where it helps; the corpus does not implement a second scene detector.

- **2.5** [question] **Where should detector settings apply by default?**  
  Should a processing profile primarily be selected for a corpus/group, for each tracked source, or for individual videos? Inheritance and override behavior are not yet approved.

- **2.6 Detector profiles default to the corpus or group, with scoped overrides.**  
  Status: adopted 2026-10-13. Answers 2.5 (option A).  
  A corpus or research group supplies the common detector profile by default. Users can optionally override that selection for a tracked channel/playlist or an individual video. The effective detector selection for a video must be inspectable so users understand which analyses will run. Existing results should be reused, and a changed profile should not needlessly rerun unrelated unchanged stages (see 2.2). Resolving conflicting inherited profiles for material belonging to more than one group/source is not decided here; see 2.7.

- **2.7** [question] **How should overlapping source or group detector profiles be resolved?**  
  When the same video belongs to multiple tracked sources or research groups whose detector configurations differ, should the applicable detectors be combined, should the user choose one effective profile, or should the corpus default prevail until a video override is set? An explicit per-video override remains supported under 2.6.

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
  Cross-video face/voice similarity clusters and textual mentions are evidence, not automatic proof that the system identified the same real-world person. Preserve provenance and uncertainty; the owner has not yet selected the cross-video identity-linking policy in 4.4.

- **4.4** [question] **How should possible person matches become one cross-video identity?**  
  Should the corpus only link explicitly confirmed people, suggest uncertain links for confirmation, or automatically combine high-confidence matches while exposing uncertainty? Until decided, keep candidate matches separate from verified person identities.
