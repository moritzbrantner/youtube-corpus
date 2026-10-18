# Detector attempt observations

Adopted with #60. The corpus owns per-video execution state; producer repositories
continue to own their analysis algorithms.

Store append-only attempt observations in Postgres, separately from retained evidence.
The latest attempt can fail while an earlier transcript remains searchable. Effective
profile selection controls whether a stage is disabled; successful metadata discovery
alone never establishes successful analysis. Translation states are exposed per source
modality and requested target language even while their producers are unavailable.

These records are not a scheduler or queue. A transaction-scoped advisory lock owns a
running detector attempt. The independently committed observation records that lock's
backend PID. A reader checks the exact lock, so cancellation, process exit and connection
loss become `interrupted` instead of permanently `running`. A retry takes the same lock;
identical active attempts cannot execute concurrently. Historical observations remain.

Reuse checks compare the relevant producer/configuration and actual retained input
bytes. Caption and ASR settings are compared independently. ASR compatibility commands
record executable content identity and arguments; this does not establish the identity
or quality of models hidden inside an arbitrary external launcher. The canonical audio
provider integration remains #73. Conflicting source compatibility configurations
fail explicitly rather than arbitrarily choosing one provider. Explicit refresh bypasses
reuse, and a failed refresh does not erase previously retained evidence.

The schema change is additive. Older binaries can continue ingesting, but they cannot
emit these attempt observations; pre-upgrade failures cannot be reconstructed from
successful transcript rows alone.
