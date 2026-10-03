# DriveWire host-service lifecycle

Each VM owns a bounded host executor. File-backed sector reads and writes run
on its worker, outside Becker register accesses and the emulation thread.
Memory-backed images remain synchronous for deterministic headless use.
Opening startup images and hashing snapshot media still use the existing
frontend paths.

The executor is the shared interface for later host services. This change
does not implement virtual channels, host shares, or `dw` commands. Their
guest-facing requirements remain in the [guest contract](drivewire-guest-contract.md).

## Queue and ownership rules

The worker starts on the first submitted job. Request and completion queues
each hold at most 16 entries, with at most one job executing. The executor
limits total accepted work to 33 requests, including undelivered completions.
Each response is limited to 4,096 bytes. Service implementations must also
bound the data captured by their jobs and stream larger transfers in chunks.
Disk jobs capture one 256-byte sector or a read offset.

Submission returns the original job when capacity is exhausted. The disk
parser retains that job and retries without requesting the guest's bytes
again. It withholds the disk response until completion. Becker reads and
frontend frames poll without waiting for the worker. A poll examines at most
eight completions and delivers at most one current result.

`submit_host_service` uses the same executor. `poll_host` routes non-disk
results into a separate queue of at most 16 entries; the owning service
drains it through `take_host_completion`. A full result queue stops polling
until the service consumes results. Results are not silently discarded to
make space.

Request identities include a host session, cancellation generation, and
sequence. Reset invalidates old generations. Restore creates a different
host session. A stale or duplicate completion cannot update guest replies,
disk counters, or replacement media. Replacing a disk cancels only its request;
unrelated service jobs and completed results remain available. Jobs receive a cooperative cancellation
token; queued cancelled jobs do not run. A job panic becomes a bounded error
and does not terminate the worker.

Host file handles, jobs, queues, threads, and cancellation tokens are excluded
from machine serialization. The emulated parser, pending guest replies,
mounted-media references, selected disk mode, and dirty flags retain their
existing roles. Restore reopens the referenced images through the frontend
and reattaches the host clock.

## Lifecycle behavior

| Event | Behavior |
| --- | --- |
| `INIT`, `TERM` | No-op notifications, matching the specification and Java reference. |
| `DWINIT` | Consume the driver byte, cancel service work, clear old replies, and return `0x04`. Preserve disk mounts, dirty flags, counters, and selected HDB-DOS mode. |
| Protocol reset (`0xF8`, `0xFE`, `0xFF`) | Cancel work and parser state, clear old replies and transfer statistics, and preserve mounted media, dirty flags, and mode. There is no buffered image cache to flush. |
| Machine reset or power cycle | Cancel host work and partial transactions without remounting media. The existing machine reset controls retain their other behavior. |
| Mount, eject, or reattach during disk I/O | Invalidate the old request and return its disk error with the original read/write framing. A late result cannot update the replacement image's state. |
| Save state | Reject unfinished host work before media hashing. Completed guest response bytes and partial guest payloads can still be serialized. Retry after I/O finishes. |
| Suspend | Save only when host work is idle, then suspend the executor and stop emulation. Mounted images remain associated with the VM. |
| Resume | Resume the executor for a warm session, or recreate host resources while loading the checkpoint for a cold session. |
| Restore | Reject unfinished work in the live VM before resolving snapshot media. Stop the replaced session. Preserve serialized disk replies and parser state that do not depend on host jobs. Never replay an external write. |
| Stop or close | Reject further jobs, invalidate accepted results, and drop host resources without joining a blocked worker on the UI thread. |

Cancellation cannot undo a host write already executing. A disk becomes dirty
when its write is submitted, even if that request is later cancelled. Saving
remains unavailable until that work and its result have been drained. Stop
does not wait for an operating-system call already in progress; the worker
releases its handles after it returns. Cross-VM write coordination belongs to
the host-share implementation.

A decoded snapshot containing an unfinished host request receives a disk
read/write error instead of replaying that request. Extended reads preserve
their data/checksum/status exchange even on this error path. Ordinary snapshots
cannot be saved in that condition through the snapshot API.

The wire remains request/reply based. A reset-valued byte inside an incomplete
guest payload is data. A client that sends another opcode while waiting for
host I/O abandons its unanswered request; the new opcode starts another
transaction. Host latency does not consume the timeout allowed for the
checksum bytes after an extended-read response.

## Diagnostics and verification

Hover over a DriveWire status entry to inspect host state, pending and
outstanding work, completion/error/cancellation counts, backpressure, and the
last bounded error. The entry is also available with no disks mounted.

Sibling tests cover deterministic delayed completion, both queue directions,
job retention, stale and duplicate results, independent VMs, panic recovery,
media replacement, reset, suspend/resume, busy snapshots, and restore without
replaying host work. The existing HDB-DOS and NitrOS-9 boot tests exercise the
file-backed worker through the real guest ROMs and disks.

The [specification][spec] and [Java protocol handler][java] establish the
wire lifecycle notifications. CoCoVM deliberately preserves its configured
HDB-DOS mode on `DWINIT`, as required by the guest contract, instead of copying
Java's driver-dependent mode change. Host cancellation and snapshot policy are
CoCoVM behavior, not additional wire commands.

[spec]: https://github.com/DrPitre/DriveWire/blob/a795310089b00710d797ebc7d0eb0942e439572d/DriveWire%20Specification.md
[java]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwprotocolhandler/DWProtocolHandler.java
