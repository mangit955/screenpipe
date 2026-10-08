// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { emit } from "@tauri-apps/api/event";
import { clearRetainedState } from "@/lib/hooks/use-retained-state";
import { useTauriEvent } from "@/lib/hooks/use-tauri-event";

// Sent to every window when recorded data is deleted. Each window keeps its
// own retained values, so each must forget them.
const RECORDED_DATA_DELETED = "recorded-data-deleted";

/**
 * After recorded data is deleted: forget retained values in every window, so
 * each section's next visit loads from scratch instead of showing the
 * deleted data again.
 */
export async function forgetRetainedStateEverywhere(): Promise<void> {
  clearRetainedState();
  try {
    await emit(RECORDED_DATA_DELETED);
  } catch (error) {
    console.warn("failed to tell other windows that data was deleted", error);
  }
}

/** Mount once per window: forgets its retained values on any deletion. */
export function useForgetRetainedStateOnDeletion() {
  useTauriEvent(RECORDED_DATA_DELETED, clearRetainedState);
}
