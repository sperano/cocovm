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
    if state not in ("changed", "unchanged", "missing"):
        raise RuntimeError(f"unexpected foreground response for benchmark PID {pid}: {state!r}")
    if state == "missing":
        return None
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


def ensure_window(pid, title, position=None):
    escaped_title = title.replace("\\", "\\\\").replace('"', '\\"')
    position_check = "false"
    position_set = ""
    if position:
        x, y = (int(value) for value in position)
        position_check = f"(position of benchmarkWindow is not {{{x}, {y}}})"
        position_set = f"    set position of benchmarkWindow to {{{x}, {y}}}\n"
    script = f'''tell application "System Events"
    set benchmarkProcess to first application process whose unix id is {int(pid)}
    if not (exists window "{escaped_title}" of benchmarkProcess) then
        return "missing"
    end if
    set benchmarkWindow to window "{escaped_title}" of benchmarkProcess
    set changedState to not (frontmost of benchmarkProcess) or (value of attribute "AXMinimized" of benchmarkWindow) or not (value of attribute "AXMain" of benchmarkWindow) or not (value of attribute "AXFocused" of benchmarkWindow) or {position_check}
    if not changedState then
        return "unchanged"
    end if
    set frontmost of benchmarkProcess to true
    set value of attribute "AXMinimized" of benchmarkWindow to false
    perform action "AXRaise" of benchmarkWindow
    set value of attribute "AXMain" of benchmarkWindow to true
    set value of attribute "AXFocused" of benchmarkWindow to true
{position_set}
    if (value of attribute "AXMinimized" of benchmarkWindow) or not (value of attribute "AXMain" of benchmarkWindow) or not (value of attribute "AXFocused" of benchmarkWindow) then
        error "window did not become restored, main, and focused"
    end if
    return "changed"
end tell'''
    return run_script(pid, script)
