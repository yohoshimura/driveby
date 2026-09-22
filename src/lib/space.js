// Room at a destination, as the backend counts it.
//
// The run refuses a destination whose volume cannot take what it would
// write (`decide_room` in src-tauri/src/backup.rs), and the preview carries
// the same two figures so the dialog can say so before the user confirms.

/// Whether a destination described by the preview lacks the room the run
/// needs there. A volume that would not say what it has free is not judged:
/// the run does not check it either, and the dialog must not block a backup
/// the run would go ahead with.
export function lacksRoom(dest) {
  return !!dest?.reachable
    && dest.availableBytes != null
    && dest.requiredBytes > dest.availableBytes;
}
