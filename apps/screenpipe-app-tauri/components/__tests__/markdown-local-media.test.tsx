// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import remarkGfm from "remark-gfm";

// The native media reader is the only boundary faked here: everything between
// the markdown text and the file request runs for real.
const getMediaFileCommand = vi.hoisted(() => vi.fn());

vi.mock("@/lib/utils/tauri", () => ({
  commands: {
    getMediaFile: getMediaFileCommand,
    openViewerWindow: vi.fn(async () => ({ status: "ok" })),
  },
}));

import { MemoizedReactMarkdown, chatUrlTransform } from "@/components/markdown";

describe("MemoizedReactMarkdown local media", () => {
  beforeEach(() => {
    getMediaFileCommand.mockResolvedValue({
      status: "ok",
      data: { data: "AAAA", mimeType: "video/mp4" },
    });
    URL.createObjectURL = vi.fn(() => "blob:local-media");
    URL.revokeObjectURL = vi.fn();
  });

  afterEach(() => {
    getMediaFileCommand.mockReset();
  });

  it.each([
    ["a bare filename", "Here is the file: `demo.mp4`", ["demo.mp4"]],
    ["a bare extension", "any name ending in `.mp4`", [".mp4"]],
    ["a relative path", "saved to `clips/demo.mp4`", ["clips/demo.mp4"]],
    [
      "a filename pattern",
      "recordings are saved as `~/.screenpipe/data/monitor_*.mp4`",
      ["~/.screenpipe/data/monitor_*.mp4"],
    ],
    [
      "bare filenames in a table",
      [
        "| File | Size |",
        "|---|---|",
        "| `before-github.mp4` | 2.6 MB |",
        "| `after-github.mp4` | 2.5 MB |",
      ].join("\n"),
      ["before-github.mp4", "after-github.mp4"],
    ],
  ])("keeps %s in inline code as text", (_label, markdown, names) => {
    render(
      <MemoizedReactMarkdown remarkPlugins={[remarkGfm]}>{markdown}</MemoizedReactMarkdown>,
    );

    for (const name of names) {
      expect(screen.getByText(name).tagName).toBe("CODE");
    }
    expect(getMediaFileCommand).not.toHaveBeenCalled();
  });

  it("keeps a code block listing several recordings as text", () => {
    const { container } = render(
      <MemoizedReactMarkdown>
        {"```\n/Users/me/Movies/a.mp4\n/Users/me/Movies/b.mp4\n```"}
      </MemoizedReactMarkdown>,
    );

    expect(container.querySelector("pre code")?.textContent).toContain("/Users/me/Movies/b.mp4");
    expect(getMediaFileCommand).not.toHaveBeenCalled();
  });

  // An <img> can never show audio or video, so a media address the reader
  // can't open must not fall through to a broken image.
  it.each([
    ["a bare filename", "![after](after-github.mp4)", "after-github.mp4"],
    ["a name with spaces", "![](<after github.mp4>)", "after github.mp4"],
    ["a web address", "![clip](https://example.com/clip.webm)", "https://example.com/clip.webm"],
  ])("shows an image-syntax video with %s as text", (_label, markdown, name) => {
    const { container } = render(
      <MemoizedReactMarkdown urlTransform={chatUrlTransform}>{markdown}</MemoizedReactMarkdown>,
    );

    expect(screen.getByText(name).tagName).toBe("CODE");
    expect(container.querySelector("img")).toBeNull();
    expect(getMediaFileCommand).not.toHaveBeenCalled();
  });

  it.each([
    ["https://example.com/demo.mp4"],
    ["//cdn.example.com/demo.mp4"],
  ])("keeps a web link to %s as a link", (href) => {
    render(
      <MemoizedReactMarkdown urlTransform={chatUrlTransform}>
        {`[demo](${href})`}
      </MemoizedReactMarkdown>,
    );

    expect(screen.getByRole("link", { name: "demo" })).toHaveAttribute("href", href);
    expect(getMediaFileCommand).not.toHaveBeenCalled();
  });

  // The media player caches by path for the whole module, so every case below
  // uses a path no other test loads.
  it.each([
    [
      "an absolute recording path",
      "`/Users/me/.screenpipe/data/monitor_1_2026-09-28_10-30-00.mp4`",
      "/Users/me/.screenpipe/data/monitor_1_2026-09-28_10-30-00.mp4",
    ],
    [
      "an image-syntax video",
      "![screen recording](</Users/me/Movies/demo-run.mp4>)",
      "/Users/me/Movies/demo-run.mp4",
    ],
    ["a home-relative path", "`~/Downloads/clip.mp4`", "~/Downloads/clip.mp4"],
    [
      "a Windows path",
      "`C:\\Users\\me\\.screenpipe\\data\\monitor_1.mp4`",
      "C:\\Users\\me\\.screenpipe\\data\\monitor_1.mp4",
    ],
    [
      "a file URL link",
      "[clip](file:///Users/me/Movies/export.mp4)",
      "/Users/me/Movies/export.mp4",
    ],
    [
      // The markdown parser percent-encodes backslashes in link addresses.
      "an image-syntax Windows video",
      "![export](<C:\\Users\\me\\Downloads\\export.mp4>)",
      "C:\\Users\\me\\Downloads\\export.mp4",
    ],
  ])("plays %s", async (_label, markdown, path) => {
    const { container } = render(
      <MemoizedReactMarkdown urlTransform={chatUrlTransform}>{markdown}</MemoizedReactMarkdown>,
    );

    await waitFor(() => expect(container.querySelector("video")).not.toBeNull());
    expect(getMediaFileCommand).toHaveBeenCalledWith(path);
  });
});
