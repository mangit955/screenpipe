// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

const MEDIA_EXTENSIONS = ["mp4", "mp3", "wav", "webm", "ogg", "m4a"] as const;
const MEDIA_EXTENSION_PATTERN = MEDIA_EXTENSIONS.join("|");

export function normalizeMediaFilePath(path: string): string {
  let cleaned = path.trim().replace(/^["'`]|["'`]$/g, "").trim();

  try {
    cleaned = decodeURIComponent(cleaned);
  } catch {
    // Keep the original string if it contains malformed percent escapes.
  }

  if (/^file:\/\/\/[A-Z]:[\\/]/i.test(cleaned)) {
    cleaned = cleaned.replace(/^file:\/\/\//i, "");
  } else {
    cleaned = cleaned.replace(/^file:\/+/i, "/");
  }

  // Windows file URLs often become /C:/Users/... after stripping file://.
  cleaned = cleaned.replace(/^\/([A-Z]:[\\/])/i, "$1");

  const windowsMatch = cleaned.match(
    new RegExp(`[A-Z]:[\\\\/][^\\n\\r\`"<>]+?\\.(${MEDIA_EXTENSION_PATTERN})`, "i"),
  );
  if (windowsMatch) return windowsMatch[0].trim();

  // Home-relative paths (e.g. `~/Downloads/clip.mp4`, or `~\Downloads\clip.mp4`
  // on Windows). The Unix matcher below anchors on the first `/`, which would
  // silently drop the leading `~` and turn `~/Downloads/clip.mp4` into
  // `/Downloads/clip.mp4` — a path that never exists. Match the tilde explicitly
  // first so the home prefix survives; the backend expands `~` to the real home
  // directory when reading the file. Require the `~` to sit at a path boundary
  // (start, or after whitespace) so a directory that merely ends in `~`
  // (e.g. `/Users/me~/clip.mp4`) isn't mistaken for a home reference.
  const tildeMatch = cleaned.match(
    new RegExp(`(?:^|\\s)(~[\\\\/][^\\n\\r\`"<>]+?\\.(?:${MEDIA_EXTENSION_PATTERN}))`, "i"),
  );
  if (tildeMatch) return tildeMatch[1].trim();

  const unixMatch = cleaned.match(
    new RegExp(`/[^\\n\\r\`"<>]+?\\.(${MEDIA_EXTENSION_PATTERN})`, "i"),
  );
  if (unixMatch) return unixMatch[0].trim();

  return cleaned;
}

export function isAudioMediaPath(path: string): boolean {
  if (/\.(mp3|wav|ogg|m4a)$/i.test(path)) return true;
  return /[\\/][^\\/]+\s+\((input|output)\)_\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}\.mp4$/i.test(path);
}

// Where the media reader can find a file: an absolute Unix path (not a `//host`
// web address), a `~/` path (the backend expands it), a Windows drive or
// network path, or a `file:` URL.
const LOCAL_PATH_PREFIX = /^(?:\/(?!\/)|~[\\/]|[A-Z]:[\\/]|\\\\|file:\/)/i;
const MEDIA_EXTENSION_SUFFIX = new RegExp(`\\.(${MEDIA_EXTENSION_PATTERN})$`, "i");
// A code block listing several files, or a pattern like `monitor_*.mp4`, is
// not one file to play.
const NOT_ONE_FILE = /[\r\n*]/;

/** Percent-decodes a markdown link address, keeping it as-is when malformed. */
export function decodeLinkAddress(address: string): string {
  try {
    return decodeURIComponent(address);
  } catch {
    return address;
  }
}

/** Whether `path` ends in an audio/video extension, wherever it points. */
export function hasMediaExtension(path: string): boolean {
  return MEDIA_EXTENSION_SUFFIX.test(path);
}

/**
 * Whether `path` names a local audio/video file the media reader can open.
 * A bare or relative name (`demo.mp4`) has no location to read from, so it
 * stays text instead of becoming a player that can only fail.
 */
export function isMediaFilePath(path: string): boolean {
  // Markdown hands link addresses over percent-encoded (`C:%5CUsers`) and the
  // player decodes them before reading, so judge the decoded form.
  const decoded = decodeLinkAddress(path);
  return (
    LOCAL_PATH_PREFIX.test(decoded) &&
    hasMediaExtension(decoded) &&
    !NOT_ONE_FILE.test(decoded)
  );
}

export function normalizeLocalMediaMarkdown(text: string): string {
  return text.replace(
    new RegExp(`(!?)\\[([^\\]]*)\\]\\(((?:/[^\n\r]+?|[A-Z]:[\\\\/][^\n\r]+?)\\.(${MEDIA_EXTENSION_PATTERN}))\\)`, "gi"),
    (_match, sigil: string, alt: string, path: string) => {
      const trimmedPath = path.trim();
      if (trimmedPath.startsWith("<") && trimmedPath.endsWith(">")) {
        return `${sigil}[${alt}](${trimmedPath})`;
      }
      return `${sigil}[${alt}](<${trimmedPath.replace(/>/g, "%3E")}>)`;
    },
  );
}
