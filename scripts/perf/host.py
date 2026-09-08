"""Host metadata and externally sampled process resources (standard library only)."""
import ctypes
import functools
import json
import platform
import subprocess
import time

SAMPLE_INTERVAL = 1.0
COMMAND_TIMEOUT = 5.0
METADATA_TIMEOUT = 60.0
BYTES_PER_KIB = 1024
RUSAGE_INFO_V0 = 0


class RusageInfoV0(ctypes.Structure):
    """ABI from macOS SDK sys/resource.h, rusage_info_v0 (stable since 10.9)."""
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64) for name in (
            "user_time", "system_time", "package_idle_wakeups", "interrupt_wakeups",
            "pageins", "wired_size", "resident_size", "physical_footprint",
            "process_start_abstime", "process_exit_abstime")]


@functools.cache
def darwin_library():
    try:
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        library.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        library.proc_pid_rusage.restype = ctypes.c_int
        return library
    except OSError:
        return None


def darwin_usage(pid):
    library = darwin_library() if platform.system() == "Darwin" else None
    if library is None:
        return {}
    usage = RusageInfoV0()
    if library.proc_pid_rusage(pid, RUSAGE_INFO_V0, ctypes.byref(usage)) != 0:
        return {}
    return {"package_idle_wakeups": usage.package_idle_wakeups,
            "interrupt_wakeups": usage.interrupt_wakeups,
            "physical_footprint_bytes": usage.physical_footprint}


def command(args, timeout=COMMAND_TIMEOUT):
    try:
        result = subprocess.run(args, capture_output=True, text=True, check=False, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def strip_device_identifiers(value):
    if isinstance(value, dict):
        return {key: strip_device_identifiers(item) for key, item in value.items()
                if not any(identifier in key.lower() for identifier in ("serial", "uuid"))}
    if isinstance(value, list):
        return [strip_device_identifiers(item) for item in value]
    return value


def metadata(root):
    data = {"os": platform.platform(), "architecture": platform.machine(),
            "commit": command(["git", "-C", str(root), "rev-parse", "HEAD"]),
            "dirty": bool(command(["git", "-C", str(root), "status", "--porcelain"])),
            "rustc": command(["rustc", "-Vv"])}
    if platform.system() == "Darwin":
        raw = command(["system_profiler", "SPHardwareDataType", "SPDisplaysDataType",
                       "SPAudioDataType", "-json"], METADATA_TIMEOUT)
        info = json.loads(raw) if raw else {}
        hardware = info.get("SPHardwareDataType", [{}])[0]
        data["hardware"] = {key: hardware.get(key) for key in
                            ("chip_type", "machine_model", "physical_memory", "number_processors")}
        data["displays"] = strip_device_identifiers(info.get("SPDisplaysDataType"))
        data["audio_devices"] = strip_device_identifiers(info.get("SPAudioDataType"))
    else:
        data["cpu"] = command(["lscpu"])
        data["displays"] = None
        data["audio_devices"] = None
    return data


def cpu_seconds(value):
    fields = value.replace("-", ":").split(":")
    if not 1 <= len(fields) <= 4:
        raise ValueError(f"invalid ps CPU time: {value}")
    seconds = float(fields.pop())
    for weight in (60, 3600, 86400):
        if fields:
            seconds += weight * float(fields.pop())
    return seconds


def sample(pid):
    started_unix = time.time()
    raw = command(["ps", "-p", str(pid), "-o", "time=,rss="])
    if not raw:
        return None
    try:
        cpu, rss = raw.split()
        result = {"unix_seconds": started_unix, "monotonic": time.monotonic(),
                  "cpu_seconds": cpu_seconds(cpu), "rss_bytes": int(rss) * BYTES_PER_KIB}
    except ValueError:
        return None
    result.update(darwin_usage(pid))
    threads = command(["ps", "-M", "-p", str(pid)]) if platform.system() == "Darwin" else None
    handles = command(["lsof", "-nP", "-p", str(pid), "-F", "f"])
    result.update(threads=max(0, len(threads.splitlines()) - 1) if threads else None,
                  file_descriptors=sum(line[1:].isdigit() for line in handles.splitlines()
                                       if line.startswith("f")) if handles else None,
                  completed_unix_seconds=time.time())
    return result


def interval_samples(samples, started, finished):
    """Keep observations wholly inside the application's measurement window."""
    if started is None or finished is None:
        return []
    return [sample for sample in samples if sample["unix_seconds"] >= started
            and sample["completed_unix_seconds"] <= finished]


def extrema(samples, key, operation):
    return operation((s[key] for s in samples if s.get(key) is not None), default=None)


def counter_delta(samples, key):
    if samples[0].get(key) is None or samples[-1].get(key) is None:
        return None
    return samples[-1][key] - samples[0][key]


def summarize(samples):
    if not samples:
        return {"unavailable": "no complete resource sample inside the measurement window"}
    elapsed = samples[-1]["monotonic"] - samples[0]["monotonic"]
    cpu = samples[-1]["cpu_seconds"] - samples[0]["cpu_seconds"]
    return {"sample_count": len(samples), "sample_span_seconds": elapsed, "cpu_seconds_delta": cpu,
            "cpu_percent_one_core": 100 * cpu / elapsed if elapsed else None,
            "peak_sampled_rss_bytes": extrema(samples, "rss_bytes", max),
            "peak_sampled_physical_footprint_bytes": extrema(samples, "physical_footprint_bytes", max),
            "threads_min": extrema(samples, "threads", min), "threads_max": extrema(samples, "threads", max),
            "fds_min": extrema(samples, "file_descriptors", min), "fds_max": extrema(samples, "file_descriptors", max),
            "package_idle_wakeups_delta": counter_delta(samples, "package_idle_wakeups"),
            "interrupt_wakeups_delta": counter_delta(samples, "interrupt_wakeups"),
            "gpu_execution_time": None,
            "limitations": "only complete samples inside measurement window; CPU and wakeup deltas cover sampled subinterval; CPU percent uses one core=100%; macOS wakeups use proc_pid_rusage v0 and are not all scheduler wakeups; GPU time unavailable"}
