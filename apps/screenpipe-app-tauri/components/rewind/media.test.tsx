// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import React from "react";
import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getMediaFileCommand = vi.hoisted(() => vi.fn());

vi.mock("@/lib/utils/tauri", () => ({
  commands: { getMediaFile: getMediaFileCommand },
}));

import { MediaComponent } from "./media";

// Long enough for the player to use up all of its retries.
const RETRIES_DONE_MS = 4_000;

describe("MediaComponent", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    getMediaFileCommand.mockResolvedValue({ status: "error", error: "File does not exist" });
  });

  afterEach(() => {
    vi.useRealTimers();
    getMediaFileCommand.mockReset();
  });

  it("keeps the error box for a recording that can't be read, and shows it at once when mounted again", async () => {
    const path = "/Users/me/.screenpipe/data/Mic (input)_2026-09-28_10-30-00.mp4";
    const first = render(<MediaComponent filePath={path} />);
    await act(() => vi.advanceTimersByTimeAsync(RETRIES_DONE_MS));

    expect(screen.getByText(/Failed to load media/)).toBeInTheDocument();
    expect(getMediaFileCommand).toHaveBeenCalledTimes(4);
    first.unmount();

    render(<MediaComponent filePath={path} />);
    expect(screen.getByText(/Failed to load media/)).toBeInTheDocument();
    expect(screen.queryByText("Loading media...")).toBeNull();
  });
});
