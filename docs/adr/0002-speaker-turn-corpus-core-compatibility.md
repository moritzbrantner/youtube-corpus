# Speaker-turn annotations in corpus-core v1

Adopted for #119 (2026-10-23); consumer acceptance: belief-lab#63 and
philosophy-extractor#19.

## Decision

Keep `moenarch_corpus_core_batch@1` and the existing corpus-core record types.
Persisted `speaker_turn` annotations belong to their canonical video `Source`
record as a versioned JSON array in string-valued `source.metadata`. A separate
`speaker_turns_schema` identifies the array contract; both keys are absent
without stored turns. Include annotation and corpus IDs, original segment/stream
IDs where present, exact media seconds, local speaker label, source kind, stored
producer version and annotation row revision. Sort by persisted values only.

The existing philosophy-extractor corpus-core v1 intake maps every `Segment` to
text from a representation and rejects segments without a character range.
Representing audio diarization turns as text segments would either invent text
or break that established consumer. Adding a new top-level record kind or v2
envelope would also break the v1 reader. Source metadata preserves the full
observation without either compatibility break.

## Consequences

- The transcript corpus-core segment IDs and media locators are unchanged;
  proposition `sourceSegments` can join by segment ID and/or stored time overlap
  through the video's stable source identity.
- Source metadata is a structured JSON _string_ under the v1 contract, so
  downstream attribution adapters parse this one versioned field explicitly.
  Existing consumers may ignore it without losing transcript functionality.
- Turn IDs, corpus ownership, speaker labels and producer/model versions are
  not converted to person identities, name guesses or assertions of stance.
- The current annotation persistence has a `processor_version` but no distinct
  immutable model-revision column. Export it verbatim as `producerRevision` if
  present and keep the annotation's numeric revision separately. Missing
  provenance remains missing, never a guessed version.
- Input rows that violate the stored media-time constraints or contain duplicate
  speaker-turn IDs cause an explicit corpus-core export error. Other export
  formats remain unaffected by the additive annotation snapshot field.

The next integration step belongs to Belief Lab: choose or review which
source-local speaker labels correspond to named people and decide proposition
stance. This repository does not own those judgments.
