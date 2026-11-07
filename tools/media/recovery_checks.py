"""Assess the injected outage before later policy hints or intentional stream stops."""
def native_recovery(timeline, not_before_ms):
    first = next((i for i, row in enumerate(timeline)
                  if row.get("ms", 0) >= not_before_ms and row.get("phase") == "Reconnecting"), None)
    checks = {"reconnecting_observed": first is not None,
              "media_resumed": False, "screen_flow_after_recovery": False}
    if first is None:
        return checks
    interrupted = timeline[first]
    for row in timeline[first + 1:]:
        if row.get("phase") != "Active":
            continue
        same_generation = row.get("generation") == interrupted.get("generation")
        audio = interrupted.get("audio_received", 0) if same_generation else 0
        video = interrupted.get("video_received", 0) if same_generation else 0
        checks["media_resumed"] |= (row.get("audio_received", 0) > audio + 100
                                    and row.get("video_received", 0) > video + 10)
        checks["screen_flow_after_recovery"] |= (bool(row.get("screen_flowing"))
            and row.get("ms", 0) > interrupted.get("ms", 0) + 2000)
    return checks
