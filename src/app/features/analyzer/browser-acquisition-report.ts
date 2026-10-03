import type { VideoAnalysisReport } from "../../api";
import type { BrowserCaptionAcquisition } from "./youtube-browser-extractor";

export function applyBrowserCaptionAcquisition(
  report: VideoAnalysisReport,
  acquisition: BrowserCaptionAcquisition,
): VideoAnalysisReport {
  const sourceKind = acquisition.track.sourceKind;
  const stream = report.streams[0];
  const language = acquisition.track.languageCode || null;
  const localYtDlp = acquisition.transport === "local-yt-dlp";

  return {
    ...report,
    video: {
      ...report.video,
      title: acquisition.player.title ?? report.video.title,
      channel: acquisition.player.author ?? report.video.channel,
      uploader: acquisition.player.author ?? report.video.uploader,
      channelId: acquisition.player.channelId ?? report.video.channelId,
      durationSeconds: acquisition.player.durationSeconds ?? report.video.durationSeconds,
      description: acquisition.player.description ?? report.video.description,
      thumbnailUrl: acquisition.player.thumbnailUrl ?? report.video.thumbnailUrl,
      viewCount: acquisition.player.viewCount ?? report.video.viewCount,
      transcriptSourceKinds: [sourceKind],
      metadata: {
        ...report.video.metadata,
        browserAnalysis: {
          schemaVersion: 4,
          engine: "youtube-corpus-browser-lexical",
          persistence: "none",
          transcriptSource: localYtDlp ? "local-yt-dlp" : "youtube-direct",
          extractionEngine: localYtDlp ? "yt-dlp" : "youtube-browser-wasm",
          playerEndpoint: acquisition.endpoint,
          innertubeClient: acquisition.client,
          captionTrack: {
            languageCode: acquisition.track.languageCode,
            name: acquisition.track.name,
            sourceKind,
            vssId: acquisition.track.vssId ?? null,
          },
        },
      },
    },
    streams: stream
      ? [
          {
            ...stream,
            sourceKind,
            language,
            message: localYtDlp
              ? "Caption track fetched by the local yt-dlp bridge; no Postgres persistence was used."
              : `Caption track fetched directly from YouTube in the browser via ${acquisition.endpoint}/${acquisition.client}.`,
          },
        ]
      : report.streams,
    segments: report.segments.map((segment) => ({ ...segment, language })),
    coverage: {
      ...report.coverage,
      metadata: Boolean(
        acquisition.player.title ||
        acquisition.player.author ||
        acquisition.player.durationSeconds ||
        acquisition.player.viewCount,
      ),
    },
  };
}
