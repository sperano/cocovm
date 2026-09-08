"""Keep only the benchmark's owned macOS process in the foreground."""
import subprocess

import host


def ensure(pid):
    script = f'''tell application "System Events"
    set benchmarkProcess to first application process whose unix id is {int(pid)}
    if frontmost of benchmarkProcess then
        return "unchanged"
    end if
    set frontmost of benchmarkProcess to true
    return "changed"
end tell'''
    try:
        result = subprocess.run(["osascript", "-e", script], capture_output=True, text=True,
                                check=True, timeout=host.COMMAND_TIMEOUT)
    except (OSError, subprocess.SubprocessError) as error:
        detail = getattr(error, "stderr", None) or str(error)
        raise RuntimeError(f"could not foreground benchmark PID {pid}: {detail}") from error
    state = result.stdout.strip()
    if state not in ("changed", "unchanged"):
        raise RuntimeError(f"unexpected foreground response for benchmark PID {pid}: {state!r}")
    return state == "changed"
