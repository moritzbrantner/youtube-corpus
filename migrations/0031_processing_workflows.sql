-- Shared processing workflow definitions (#108, Atlas 6.1-6.2).
--
-- A definition is authored in the shared Workflow Editor and stored as immutable versions: the
-- editor document (to reopen it) and its compiled `@moritzbrantner/workflow/compiled` v1 form
-- (what corpus processing translates into a detector plan). Definitions are reusable across
-- corpora; enabled stages, their parameters and run histories belong to one corpus.
CREATE TABLE processing_workflows (
    id uuid PRIMARY KEY,
    name text NOT NULL CHECK (btrim(name) <> ''),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE processing_workflow_versions (
    workflow_id uuid NOT NULL REFERENCES processing_workflows(id) ON DELETE CASCADE,
    version integer NOT NULL CHECK (version > 0),
    document jsonb NOT NULL,
    compiled jsonb NOT NULL,
    fingerprint text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workflow_id, version)
);

-- One corpus's overrides of a definition's stages, keyed by compiled node id:
-- `[{"nodeId": "asr", "enabled": false}, {"nodeId": "translate", "targetLanguages": ["de"]}]`.
CREATE TABLE corpus_workflow_settings (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    workflow_id uuid NOT NULL REFERENCES processing_workflows(id) ON DELETE CASCADE,
    stages jsonb NOT NULL DEFAULT '[]'::jsonb,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (corpus_id, workflow_id)
);

-- One run of a definition version over a corpus. `stages` is the effective stage list the run
-- applied; `report` holds every video's stage outcomes once the run ends. Stage attempts stay in
-- `detector_attempts`.
CREATE TABLE corpus_workflow_runs (
    id uuid PRIMARY KEY,
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    workflow_id uuid NOT NULL,
    workflow_version integer NOT NULL,
    fingerprint text NOT NULL,
    stages jsonb NOT NULL,
    refresh boolean NOT NULL DEFAULT false,
    state text NOT NULL CHECK (state IN ('running', 'completed', 'failed', 'interrupted')),
    report jsonb,
    message text,
    started_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    FOREIGN KEY (workflow_id, workflow_version)
        REFERENCES processing_workflow_versions(workflow_id, version) ON DELETE CASCADE
);

CREATE INDEX corpus_workflow_runs_by_corpus
    ON corpus_workflow_runs (corpus_id, workflow_id, started_at DESC);
