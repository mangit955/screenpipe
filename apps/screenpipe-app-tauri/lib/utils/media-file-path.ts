// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

const MEDIA_EXTENSIONS = ["mp4", "mp3", "wav", "webm", "ogg", "m4a"] as const;
const MEDIA_EXTENSION_PATTERN = MEDIA_EXTENSIONS.join("|");

/** Percent-decodes a markdown link address, keeping it as-is when malformed. */
export function decodeLinkAddress(address: string): string {
  try {
    return decodeURIComponent(address);
  } catch {
    return address;
  }
}

// Removes what chat wraps around a path (quotes, backticks, percent escapes, a
// `file:` scheme), leaving the text the player looks for a path in.
function unwrapMediaFilePath(path: string): string {
  let cleaned = decodeLinkAddress(path.trim().replace(/^["'`]|["'`]$/g, "").trim());

  if (/^file:\/\/\/[A-Z]:[\\/]/i.test(cleaned)) {
    cleaned = cleaned.replace(/^file:\/\/\//i, "");
  } else {
    cleaned = cleaned.replace(/^file:\/+/i, "/");
  }

  // Windows file URLs often become /C:/Users/... after stripping file://.
  return cleaned.replace(/^\/([A-Z]:[\\/])/i, "$1");
}

// The first media path in `text`, so a path can be read out of a sentence.
function findMediaFilePath(text: string): string {
  const windowsMatch = text.match(
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
  const tildeMatch = text.match(
    new RegExp(`(?:^|\\s)(~[\\\\/][^\\n\\r\`"<>]+?\\.(?:${MEDIA_EXTENSION_PATTERN}))`, "i"),
  );
  if (tildeMatch) return tildeMatch[1].trim();

  const unixMatch = text.match(
    new RegExp(`/[^\\n\\r\`"<>]+?\\.(${MEDIA_EXTENSION_PATTERN})`, "i"),
  );
  if (unixMatch) return unixMatch[0].trim();

  return text;
}

export function normalizeMediaFilePath(path: string): string {
  return findMediaFilePath(unwrapMediaFilePath(path));
}

export function isAudioMediaPath(path: string): boolean {
  if (/\.(mp3|wav|ogg|m4a)$/i.test(path)) return true;
  return /[\\/][^\\/]+\s+\((input|output)\)_\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}\.mp4$/i.test(path);
}

// Where the media reader can find a file: an absolute Unix path (not a `//host`
// web address), a `~/` path (the backend expands it), or a Windows drive or
// network path. A `file:` URL is unwrapped into one of these first.
const LOCAL_PATH_PREFIX = /^(?:\/(?!\/)|~[\\/]|[A-Z]:[\\/]|\\\\)/i;
const MEDIA_EXTENSION_SUFFIX = new RegExp(`\\.(${MEDIA_EXTENSION_PATTERN})$`, "i");
// A code block listing several files, a pattern like `monitor_*.mp4`, or a
// placeholder like `monitor_<id>.mp4` is not one file to play.
const NOT_ONE_FILE = /[\r\n*<>]/;

/**
 * Whether `path` names one local audio/video file the media reader can open.
 * A bare or relative name (`demo.mp4`) has no location to read from, so it
 * stays text instead of becoming a player that can only fail.
 */
export function isMediaFilePath(path: string): boolean {
  // Judge the path the way the player reads it: markdown hands link addresses
  // over percent-encoded (`C:%5CUsers`) or as `file:` URLs.
  const unwrapped = unwrapMediaFilePath(path);
  return (
    LOCAL_PATH_PREFIX.test(unwrapped) &&
    MEDIA_EXTENSION_SUFFIX.test(unwrapped) &&
    !NOT_ONE_FILE.test(unwrapped) &&
    // The player reads only the first media path it finds, so text naming
    // several (`/a.mp4, /b.mp4`) would play just part of what it says.
    findMediaFilePath(unwrapped) === unwrapped
  );
}

/**
 * Whether a link or image address points at an audio/video file, local or on
 * the web. A web address may carry a `?query` or `#fragment` after the name.
 */
export function isMediaAddress(address: string): boolean {
  const withoutQuery = address.split(/[?#]/, 1)[0] ?? address;
  return MEDIA_EXTENSION_SUFFIX.test(withoutQuery);
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
