export interface ParsedYouTubeUrl {
  videoId: string;
  canonicalUrl: string;
  embedUrl: string;
  thumbnailUrl: string;
}

const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;

export function parseYouTubeVideoUrl(input: string): ParsedYouTubeUrl | null {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    return null;
  }

  if (url.protocol !== "https:" && url.protocol !== "http:") {
    return null;
  }

  const host = url.hostname.toLowerCase().replace(/^www\./, "");
  let videoId: string | null = null;

  if (host === "youtu.be") {
    videoId = firstPathSegment(url.pathname);
  } else if (host === "youtube.com" || host.endsWith(".youtube.com")) {
    if (url.pathname === "/watch") {
      videoId = url.searchParams.get("v");
    } else {
      const [kind, id] = pathSegments(url.pathname);
      if (["embed", "live", "shorts"].includes(kind ?? "")) {
        videoId = id ?? null;
      }
    }
  } else if (host === "youtube-nocookie.com" || host.endsWith(".youtube-nocookie.com")) {
    const [kind, id] = pathSegments(url.pathname);
    if (kind === "embed") {
      videoId = id ?? null;
    }
  }

  if (!videoId || !VIDEO_ID.test(videoId)) {
    return null;
  }

  return {
    videoId,
    canonicalUrl: `https://www.youtube.com/watch?v=${videoId}`,
    embedUrl: `https://www.youtube-nocookie.com/embed/${videoId}`,
    thumbnailUrl: `https://i.ytimg.com/vi/${videoId}/hqdefault.jpg`,
  };
}

function firstPathSegment(pathname: string) {
  return pathSegments(pathname)[0] ?? null;
}

function pathSegments(pathname: string) {
  return pathname.split("/").filter(Boolean);
}
