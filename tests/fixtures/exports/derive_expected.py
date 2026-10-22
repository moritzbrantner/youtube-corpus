#!/usr/bin/env python3
"""Independent derivation of the expected downstream export fixtures (issue #42).

This script does NOT call youtube-corpus code. It re-implements the documented
export contract (see the header of `tests/corpus_exports.rs`) with the Python
standard library only, so the committed expected files follow from the contract
rather than from the implementation under test.

Usage (from the repository root):

    python3 -I tests/fixtures/exports/derive_expected.py

It reads `tests/fixtures/exports/corpus-snapshot.json` and writes
`tests/fixtures/exports/expected/{source-span.json,corpus-core.jsonl,belief-evidence.json}`.
"""

import hashlib
import math
import json
import pathlib
import struct

HERE = pathlib.Path(__file__).resolve().parent
REVISION = "git:fixture"
PRODUCER = "youtube-corpus"
CORE_PRODUCER = "youtube-corpus/corpus-export"
TEXT_MEDIA_TYPE = "text/plain; charset=utf-8"


def sha_hex(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha(text):
    return "sha256:" + sha_hex(text)


def compact(value):
    # serde_json::to_string: no whitespace, non-ASCII unescaped.
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def canonical(value):
    # serde_json::to_string(&serde_json::to_value(..)) without preserve_order: keys sorted.
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def pretty(value):
    # serde_json::to_string_pretty: two-space indent, `"key": value`.
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def sorted_obj(mapping):
    # BTreeMap / serde_json::Map without preserve_order: keys in byte order.
    return {key: mapping[key] for key in sorted(mapping)}


def derive_id(namespace, parts):
    # corpus-core `derive`: namespace ":" hex(sha256(len_be_u64 ‖ bytes … over namespace and parts)).
    hasher = hashlib.sha256()
    for component in [namespace, *parts]:
        data = component.encode("utf-8")
        hasher.update(struct.pack(">Q", len(data)))
        hasher.update(data)
    return f"{namespace}:{hasher.hexdigest()}"


def creators(video):
    result = []
    for creator in (video.get("channel"), video.get("uploader")):
        if creator is not None and creator.strip() and creator not in result:
            result.append(creator)
    return result


def track_sort_key(track):
    start = track.get("startSeconds")
    return (start is None, start if start is not None else 0.0, track["trackKey"], track["id"])


def segment_material(segment):
    # Shared per-segment revision material (same object the source-span stream revision hashes).
    return sorted_obj(
        {
            "id": segment["id"],
            "sequence": segment["segmentIndex"],
            "textHash": sha(segment["text"]),
            "startSeconds": segment.get("startSeconds"),
            "endSeconds": segment.get("endSeconds"),
            "language": segment.get("language"),
        }
    )


def load():
    snapshot = json.loads((HERE / "corpus-snapshot.json").read_text(encoding="utf-8"))
    videos = {video["id"]: video for video in snapshot["videos"]}
    streams = sorted(snapshot["transcriptStreams"], key=lambda stream: stream["id"])
    for stream in streams:
        stream["segments"] = sorted(stream["segments"], key=lambda seg: seg["segmentIndex"])
    runs = sorted(snapshot.get("visualTextRuns", []), key=lambda run: run["id"])
    for run in runs:
        run["tracks"] = sorted(run["tracks"], key=track_sort_key)
    return videos, streams, runs, snapshot.get("scenes", [])


def optional(record, key, value):
    if value is not None:
        record[key] = value


def source_span(videos, streams, runs):
    sources, spans = [], []
    for stream in streams:
        video = videos[stream["videoId"]]
        segments = stream["segments"]
        full_text = stream.get("fullText")
        stream_text = full_text if full_text else " ".join(seg["text"] for seg in segments)
        content_hash = sha(stream_text)
        material = sorted_obj(
            {
                "contentHash": content_hash,
                "sourceKind": stream["sourceKind"],
                "language": stream.get("language"),
                "spans": [segment_material(seg) for seg in segments],
            }
        )
        metadata = {
            "videoId": video["id"],
            "streamId": stream["id"],
            "transcriptSource": stream["sourceKind"],
        }
        optional(metadata, "youtubeId", video.get("youtubeId"))
        optional(metadata, "channelId", video.get("channelId"))
        optional(metadata, "uploaderId", video.get("uploaderId"))
        source = {"id": stream["id"], "kind": "youtube_transcript", "revision": sha(compact(material))}
        source["uri"] = video["sourceUrl"]
        optional(source, "title", video.get("title"))
        if creators(video):
            source["creators"] = creators(video)
        optional(source, "language", stream.get("language"))
        source["contentHash"] = content_hash
        source["metadata"] = sorted_obj(metadata)
        sources.append(source)
        for seg in segments:
            span = {
                "id": seg["id"],
                "sourceId": stream["id"],
                "sequence": seg["segmentIndex"],
                "text": seg["text"],
                "contentHash": sha(seg["text"]),
            }
            optional(span, "language", seg.get("language") or stream.get("language"))
            locator = {"kind": "timed", "segmentIndex": seg["segmentIndex"]}
            optional(locator, "startSeconds", seg.get("startSeconds"))
            optional(locator, "endSeconds", seg.get("endSeconds"))
            span["locator"] = locator
            span["metadata"] = sorted_obj(
                {"videoId": video["id"], "streamId": stream["id"], "transcriptSource": stream["sourceKind"]}
            )
            spans.append(span)
    for run in runs:
        video = videos[run["videoId"]]
        tracks = run["tracks"]
        content_hash = sha("\n".join(track["text"] for track in tracks))
        material = sorted_obj(
            {
                "contentHash": content_hash,
                "processor": run["processor"],
                "processorVersion": run["processorVersion"],
                "model": run["model"],
                "modelVersion": run["modelVersion"],
                "inputHash": run["inputHash"],
                "configHash": run["configHash"],
                "spans": [
                    sorted_obj(
                        {
                            "id": track["id"],
                            "sequence": sequence,
                            "textHash": sha(track["text"]),
                            "role": track["role"],
                            "startSeconds": track.get("startSeconds"),
                            "endSeconds": track.get("endSeconds"),
                        }
                    )
                    for sequence, track in enumerate(tracks)
                ],
            }
        )
        source_id = f"ocr:{run['id']}"
        source = {"id": source_id, "kind": "youtube_visual_text", "revision": sha(compact(material))}
        source["uri"] = video["sourceUrl"]
        optional(source, "title", video.get("title"))
        if creators(video):
            source["creators"] = creators(video)
        optional(source, "language", next((t["language"] for t in tracks if t.get("language")), None))
        source["contentHash"] = content_hash
        source["metadata"] = sorted_obj(
            {
                "videoId": video["id"],
                "runId": run["id"],
                "processor": run["processor"],
                "processorVersion": run["processorVersion"],
                "model": run["model"],
                "modelVersion": run["modelVersion"],
                "inputHash": run["inputHash"],
                "configHash": run["configHash"],
            }
        )
        sources.append(source)
    for run in runs:
        for sequence, track in enumerate(run["tracks"]):
            span = {
                "id": track["id"],
                "sourceId": f"ocr:{run['id']}",
                "sequence": sequence,
                "text": track["text"],
                "contentHash": sha(track["text"]),
            }
            optional(span, "language", track.get("language"))
            locator = {"kind": "timed", "segmentIndex": sequence}
            optional(locator, "startSeconds", track.get("startSeconds"))
            optional(locator, "endSeconds", track.get("endSeconds"))
            span["locator"] = locator
            span["metadata"] = sorted_obj(
                {
                    "videoId": run["videoId"],
                    "runId": run["id"],
                    "visualTextRole": track["role"],
                    "mediaEvidenceRef": track["id"],
                }
            )
            spans.append(span)
    return pretty(
        {
            "schema": "source_span_interchange",
            "schemaVersion": 1,
            "producer": {"name": PRODUCER, "revision": REVISION},
            "sources": sources,
            "spans": spans,
        }
    )


def ms(seconds):
    return int(round(seconds * 1000.0))


def time_range(start, end):
    if start is None or end is None:
        return None
    return {"start": ms(start), "end": ms(end)}


def core_provenance(operation, inputs):
    return {
        "operation": operation,
        "producer": CORE_PRODUCER,
        "producer_version": REVISION,
        "inputs": inputs,
        "parameters_hash": None,
    }


def text_segments(asset_id, representation_id, items, operation, metadata_for, separator="\n"):
    """Yields (segment record, representation text) for items of (key, text, start, end)."""
    records = []
    offset = 0
    for index, (key, text, start, end, extra) in enumerate(items):
        if index:
            offset += len(separator)
        length = len(text)  # Python str length == Unicode scalar count
        records.append(
            {
                "record_type": "segment",
                "record": {
                    "id": derive_id("segment", [asset_id, key]),
                    "asset_id": asset_id,
                    "locator": {
                        "byte_range": None,
                        "character_range": {"start": offset, "end": offset + length},
                        "page_index": None,
                        "time_range_ms": time_range(start, end),
                        "frame_range": None,
                        "selectors": {"character_unit": "unicode_scalar", "representation_id": representation_id},
                    },
                    "provenance": core_provenance(operation, [{"record_type": "representation", "id": representation_id}]),
                    "metadata": sorted_obj(metadata_for(extra, text)),
                },
            }
        )
        offset += length
    return records, separator.join(item[1] for item in items)


def corpus_core(videos, streams, runs):
    lines = [compact({"schema": "moenarch_corpus_core_batch", "schema_version": 1, "producer": {"name": PRODUCER, "revision": REVISION}})]
    payloads = {}
    for video_id in sorted(videos):
        video = videos[video_id]
        source_id = derive_id("source", [PRODUCER, video_id])
        url = video["sourceUrl"]
        kind = "url" if url.lower().startswith(("http://", "https://")) else "file_system"
        metadata = {"video_id": video_id}
        for key, field in (
            ("title", "title"),
            ("channel", "channel"),
            ("channel_id", "channelId"),
            ("uploader", "uploader"),
            ("uploader_id", "uploaderId"),
            ("youtube_id", "youtubeId"),
        ):
            value = video.get(field)
            if value is not None and value.strip():
                metadata[key] = value
        lines.append(compact({"record": {"record_type": "source", "record": {"id": source_id, "kind": kind, "uri": url, "metadata": sorted_obj(metadata)}}}))

        for stream in (s for s in streams if s["videoId"] == video_id):
            asset_id = derive_id("asset", [source_id, "transcript_stream", stream["id"]])
            asset_meta = {
                "processor": stream["processor"],
                "processor_version": stream["processorVersion"],
                "transcript_source": stream["sourceKind"],
                "transcript_stream_id": stream["id"],
            }
            optional(asset_meta, "language", stream.get("language"))
            lines.append(
                compact(
                    {
                        "record": {
                            "record_type": "asset",
                            "record": {
                                "id": asset_id,
                                "source_id": source_id,
                                "kind": "subtitle",
                                "content": {"uri": url, "hash": None, "media_type": None, "byte_length": None},
                                "metadata": sorted_obj(asset_meta),
                            },
                        }
                    }
                )
            )
            representation_id = derive_id("representation", [asset_id, "transcript"])

            def seg_meta(seg, text, stream=stream):
                meta = {
                    "segment_index": str(seg["segmentIndex"]),
                    "text_sha256": sha_hex(text),
                    "transcript_segment_id": seg["id"],
                }
                optional(meta, "language", seg.get("language") or stream.get("language"))
                return meta

            items = [(seg["id"], seg["text"], seg.get("startSeconds"), seg.get("endSeconds"), seg) for seg in stream["segments"]]
            segments, text = text_segments(asset_id, representation_id, items, "segment_transcript", seg_meta)
            text_hash = sha_hex(text)
            payloads.setdefault(text_hash, text)
            lines.append(
                compact(
                    {
                        "record": {
                            "record_type": "representation",
                            "record": {
                                "id": representation_id,
                                "owner": {"record_type": "asset", "id": asset_id},
                                "kind": "transcript",
                                "content_hash": text_hash,
                                "content": None,
                                "provenance": core_provenance("assemble_transcript", [{"record_type": "asset", "id": asset_id}]),
                                "metadata": {},
                            },
                        }
                    }
                )
            )
            lines.extend(compact({"record": segment}) for segment in segments)

        for run in (r for r in runs if r["videoId"] == video_id):
            asset_id = derive_id("asset", [source_id, "visual_text_run", run["id"]])
            lines.append(
                compact(
                    {
                        "record": {
                            "record_type": "asset",
                            "record": {
                                "id": asset_id,
                                "source_id": source_id,
                                "kind": "video",
                                "content": {"uri": url, "hash": None, "media_type": None, "byte_length": None},
                                "metadata": sorted_obj(
                                    {
                                        "config_hash": run["configHash"],
                                        "input_hash": run["inputHash"],
                                        "model": run["model"],
                                        "model_version": run["modelVersion"],
                                        "processor": run["processor"],
                                        "processor_version": run["processorVersion"],
                                        "visual_text_run_id": run["id"],
                                    }
                                ),
                            },
                        }
                    }
                )
            )
            representation_id = derive_id("representation", [asset_id, "ocr_text"])

            def track_meta(track, text):
                meta = {
                    "text_sha256": sha_hex(text),
                    "visual_text_role": track["role"],
                    "visual_text_track_id": track["id"],
                }
                optional(meta, "language", track.get("language"))
                return meta

            items = [(t["id"], t["text"], t.get("startSeconds"), t.get("endSeconds"), t) for t in run["tracks"]]
            segments, text = text_segments(asset_id, representation_id, items, "segment_visual_text", track_meta)
            text_hash = sha_hex(text)
            payloads.setdefault(text_hash, text)
            lines.append(
                compact(
                    {
                        "record": {
                            "record_type": "representation",
                            "record": {
                                "id": representation_id,
                                "owner": {"record_type": "asset", "id": asset_id},
                                "kind": "ocr_text",
                                "content_hash": text_hash,
                                "content": None,
                                "provenance": core_provenance("assemble_visual_text", [{"record_type": "asset", "id": asset_id}]),
                                "metadata": {},
                            },
                        }
                    }
                )
            )
            lines.extend(compact({"record": segment}) for segment in segments)

    for text_hash in sorted(payloads):
        lines.append(compact({"payload": {"hash": text_hash, "media_type": TEXT_MEDIA_TYPE, "text": payloads[text_hash]}}))
    return "\n".join(lines) + "\n"


MEDIA_TICKS = 1_000_000


def media_ticks(seconds):
    # Microsecond ticks, rounding halves away from zero like Rust's f64::round for non-negative times.
    if seconds is None or seconds < 0 or seconds != seconds or seconds in (float("inf"),):
        return None
    return math.floor(seconds * MEDIA_TICKS + 0.5)


def media_time(pts):
    return {"pts": pts, "timebase": {"num": 1, "den": MEDIA_TICKS}}


def belief_evidence(videos, streams, scenes):
    scenes = sorted(scenes, key=lambda scene: (scene["startSeconds"], scene["id"]))
    evidence = []
    for stream in streams:
        for seg in stream["segments"]:
            revision = stream.get("processorRevision", stream["processorVersion"])
            prefixed = any(revision.startswith(prefix) and len(revision) > len(prefix)
                           for prefix in ("sha256:", "git:", "commit:"))
            full_hex = len(revision) in (40, 64) and all(c in "0123456789abcdefABCDEF" for c in revision)
            if not (prefixed or full_hex):
                raise ValueError(f"stream {stream['id']} lacks immutable processor provenance")
            producer = {"name": stream["processor"], "revision": revision}
            optional(producer, "configHash", stream.get("processingConfigHash"))
            video = seg.get("videoId", stream["videoId"])
            start, end = media_ticks(seg.get("startSeconds")), media_ticks(seg.get("endSeconds"))
            media = {}
            if start is not None and end is not None and start < end:
                media["mediaSpan"] = {"start": media_time(start), "end": media_time(end)}
                covering = [
                    f"scene:{scene['id']}"
                    for scene in scenes
                    if scene["videoId"] == video
                    and media_ticks(scene["startSeconds"]) < end
                    and start < media_ticks(scene["endSeconds"])
                ]
                if covering:
                    media["sceneIds"] = covering
            evidence.append(
                {
                    "id": f"evidence:youtube-corpus:transcript-segment:{seg['id']}",
                    "class": "transcript",
                    "familyId": f"family:transcript-segment:{seg['id']}",
                    "source": {
                        "repository": "youtube-corpus",
                        "scopeId": f"video:{stream['videoId']}",
                        "recordId": f"transcript-segment:{seg['id']}",
                        "revision": sha(compact(segment_material(seg))),
                    },
                    "producer": producer,
                    **media,
                }
            )
    evidence.sort(key=lambda record: record["id"])
    return pretty(
        {
            "schema": "belief_evidence_interchange",
            "schemaVersion": 2 if any("mediaSpan" in record for record in evidence) else 1,
            "exporter": {"name": PRODUCER, "revision": REVISION},
            "revision": sha(canonical(evidence)),
            "evidence": evidence,
        }
    )


def main():
    videos, streams, runs, scenes = load()
    out = HERE / "expected"
    out.mkdir(exist_ok=True)
    (out / "source-span.json").write_text(source_span(videos, streams, runs), encoding="utf-8")
    (out / "corpus-core.jsonl").write_text(corpus_core(videos, streams, runs), encoding="utf-8")
    (out / "belief-evidence.json").write_text(belief_evidence(videos, streams, scenes), encoding="utf-8")


if __name__ == "__main__":
    main()
