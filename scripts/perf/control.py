"""Bounded concurrent MCP load against the isolated benchmark instance."""
import concurrent.futures
import json
import threading
import time
import urllib.request

CLIENTS = 4
REQUEST_TIMEOUT = 2
FAILURE_BACKOFF = 0.1
MILLISECONDS_PER_SECOND = 1000
MAX_LATENCY_MS = REQUEST_TIMEOUT * MILLISECONDS_PER_SECOND
PERCENT = 100
PERCENTILE_95 = 95
PERCENTILE_99 = 99


def post(port, payload, session=None):
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if session:
        headers["Mcp-Session-Id"] = session
    request = urllib.request.Request(f"http://127.0.0.1:{port}/mcp",
                                     data=json.dumps(payload).encode(), headers=headers)
    with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT) as response:
        return json.load(response), response.headers.get("Mcp-Session-Id")


def percentile(histogram, percent):
    count = sum(histogram)
    if count == 0:
        return None
    rank = (count * percent + PERCENT - 1) // PERCENT
    seen = 0
    for latency, frequency in enumerate(histogram):
        seen += frequency
        if seen >= rank:
            return latency
    raise AssertionError("histogram count changed")


def client(port, stop):
    successes, failures, saturated = 0, 0, 0
    histogram = [0] * (MAX_LATENCY_MS + 1)
    maximum = 0.0
    session = None
    while not stop.is_set():
        try:
            if session is None:
                _, session = post(port, {"jsonrpc": "2.0", "id": 0, "method": "initialize",
                    "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                               "clientInfo": {"name": "perf-baseline", "version": "1"}}})
            start = time.monotonic()
            result, _ = post(port, {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                   "params": {"name": "list_vms", "arguments": {}}}, session)
            if "error" in result or result.get("result", {}).get("isError"):
                raise ValueError(result)
            elapsed_ms = (time.monotonic() - start) * MILLISECONDS_PER_SECOND
            successes += 1
            maximum = max(maximum, elapsed_ms)
            saturated += int(elapsed_ms > MAX_LATENCY_MS)
            histogram[min(int(elapsed_ms), MAX_LATENCY_MS)] += 1
        except (OSError, ValueError):
            failures += 1
            session = None
            stop.wait(FAILURE_BACKOFF)
    return {"successes": successes, "failures": failures, "latency_ms_histogram": histogram,
            "p95_ms_bucket": percentile(histogram, PERCENTILE_95),
            "p99_ms_bucket": percentile(histogram, PERCENTILE_99),
            "max_ms": maximum if successes else None, "saturated_samples": saturated,
            "histogram_semantics": "integer-ms lower bounds; final bucket includes larger latencies"}


def start(port):
    stop = threading.Event()
    pool = concurrent.futures.ThreadPoolExecutor(max_workers=CLIENTS)
    futures = [pool.submit(client, port, stop) for _ in range(CLIENTS)]
    return stop, pool, futures


def finish(load):
    stop, pool, futures = load
    stop.set()
    pool.shutdown(wait=True, cancel_futures=True)
    return [future.result() for future in futures if not future.cancelled()]
