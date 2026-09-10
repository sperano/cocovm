"""Keep only the benchmark's owned macOS process in the foreground."""
import subprocess

import host


def run_script(pid, script):
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


def ensure(pid):
    script = f'''tell application "System Events"
    set benchmarkProcess to first application process whose unix id is {int(pid)}
    if frontmost of benchmarkProcess then
        return "unchanged"
    end if
    set frontmost of benchmarkProcess to true
    return "changed"
end tell'''
    return run_script(pid, script)


def ensure_window(pid, title):
    escaped_title = title.replace("\\", "\\\\").replace('"', '\\"')
    script = f'''tell application "System Events"
    set benchmarkProcess to first application process whose unix id is {int(pid)}
    set benchmarkWindow to first window of benchmarkProcess whose name is "{escaped_title}"
    set changedState to not (frontmost of benchmarkProcess) or not (value of attribute "AXMain" of benchmarkWindow) or not (value of attribute "AXFocused" of benchmarkWindow)
    if not changedState then
        return "unchanged"
    end if
    set frontmost of benchmarkProcess to true
    perform action "AXRaise" of benchmarkWindow
    set value of attribute "AXMain" of benchmarkWindow to true
    set value of attribute "AXFocused" of benchmarkWindow to true
    if not (value of attribute "AXMain" of benchmarkWindow) or not (value of attribute "AXFocused" of benchmarkWindow) then
        error "window did not become main and focused"
    end if
    return "changed"
end tell'''
    return run_script(pid, script)
