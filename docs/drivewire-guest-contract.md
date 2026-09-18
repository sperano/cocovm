# DriveWire guest compatibility contract

This design selects NitrOS-9 commands as the first host-access interface.
It defines implementation targets, not a claim that those services work in
CocoVM. The implementation baseline is `685cb87`. The
[capability inventory](drivewire-capabilities.md) records the broader scope,
and [VM settings](drivewire-settings.md) describes implemented configuration.

## Compatibility targets

Keep the existing CoCo 3 Becker disk clients working while adding NitrOS-9
virtual channels and `dw` commands. Use these separate acceptance targets:

| Target | Guest components | Compatibility boundary |
| --- | --- | --- |
| Existing BASIC disk access | `coco3.rom`, Becker `hdbdw3bc3.rom`, flat DECB images | `DIR` and `SAVE`; HDB-DOS mode enabled. This does not access host filenames. |
| Existing NitrOS-9 disk boot | NitrOS-9 Level 2 3.3.0 CoCo 3 Becker image, bootstrapped by that HDB-DOS ROM | `rbdw` and Becker `dwio`; HDB-DOS mode disabled. Boot and shell disk I/O are separate from virtual-channel acceptance. |
| First host-access target | NitrOS-9 `dw` utility, SCF, `scdwv`, its `/N` and numbered descriptors, and `dwio` | Browse a configured share, retrieve file bytes, and select an image through commands. Functional channels are a prerequisite. |
| Named-object follow-up | CoCoBoot is the client identified by the specification | Select and pin a runnable CoCoBoot build before claiming compatibility. NitrOS-9 opcode definitions alone do not prove client use. |
| Ordinary host files and directories | Candidate NitrOS-9 `rfm`, its descriptors, and server `OP_RFM` service | Research target only. The inspected client and server are incomplete. No working filesystem target has been selected. |

The pinned [CoCo 3 bootlist][guest-bootlist] includes disk modules but comments
out virtual-channel modules and descriptors. Building that bootlist unchanged
does not establish a working `/N`. The installed 3.3.0 boot-test image and the
pinned source revision are distinct fixtures. Record the built guest's modules
and image hash when testing commands.

Use the in-process Becker transport for these targets. Physical serial links,
external TCP clients, other machines, and other guest revisions need their own
acceptance evidence. Preserve one protocol session per VM.

## Service priorities and coverage

"First target" means selected for implementation after lifecycle and share
handling. "Follow-up" retains an existing implementation task without promising
a delivery order. "Research" requires a compatibility or product decision.
Unsupported services must remain absent from feature claims and settings.

| Inventoried service | Status | Guest interface and acceptance boundary |
| --- | --- | --- |
| Sector reads, writes, retries, and extended reads | Existing | HDB-DOS and NitrOS-9 disk regressions; keep status and checksum framing. |
| Time | Existing | `clock2_dw`; deterministic injected clock in core tests. |
| Init, reset, and terminate | Lifecycle prerequisite | Define resource teardown and stale-completion handling; baseline accepts these without service reset effects. |
| Disk status notifications | Lifecycle prerequisite | Preserve two-byte payload consumption and no reply; implement only status meanings verified against selected clients. |
| Virtual serial channels | First target prerequisite | `dwio` and `scdwv`; byte/block transfer, polling, status, close, and EOF. Baseline only consumes or fabricates replies. |
| Host directory listing | First target | `dw server dir`; enumerate paths inside an assigned share. |
| Host file retrieval | First target | `dw server list`; preserve payload bytes, including binary data. Guest redirection needs byte-for-byte acceptance. |
| Guest disk insertion and ejection | First target | `dw disk insert` and `dw disk eject`; change runtime mounts without changing startup settings. |
| Named-object mount/create | Follow-up | CoCoBoot candidate; sector access through a temporary drive lease. |
| Transparent host filesystem | Research | Validate `rfm` interoperability or choose a separate filesystem adapter before implementation. |
| Printer bytes and flush | Follow-up | Select `scdwp` with its printer descriptor or another verified guest; validate job bytes and flush behavior. |
| TCP, listeners, and Telnet/modem commands | Follow-up | Virtual channels plus a selected networking client; disk boot does not validate sockets. |
| Host serial bridge | Follow-up | `ser join` service; host port configuration, bidirectional bytes, and disconnect behavior. |
| MIDI | Follow-up | Virtual-channel MIDI routing; select guest sender and verify host output. |
| Terminal and virtual-window channels | Follow-up | Select `/Z` or terminal descriptors and a host backend; text transport alone does not implement window rendering. |
| SSH, aliases beyond named objects, capability extensions, host sound | Research | Server-specific clients and discovery require separate selection and evidence. |
| EmCee, DLOAD, Dragon DOS Plus, and image adapters | Research | Separate protocol or media compatibility tasks, not implied DW4 support. |
| WireBug | Research | Specification describes an unfinished feature; CocoVM's debugger is a separate interface. |
| Server management APIs and external transports | Research | Choose consumers and exposure model; each external connection would require its own session. |

These follow-ups remain in the project backlog. NitrOS-9 command priority does
not remove printing, networking, named objects, or filesystem access from scope.
The extension-scoping task owns remaining feature and priority decisions.

## Disk addressing and allocation

### Established behavior

The wire carries an unsigned drive byte and a 24-bit logical sector number
(LSN). Sectors contain 256 bytes. The address field can represent drives
0–255; CocoVM's `DRIVE_COUNT = 4` is an implementation limit.
[Specification][spec], [core constants](../crates/coco-core/src/drivewire.rs)

With HDB-DOS mode disabled, CocoVM uses `(wire_drive, LSN)` directly. With the
mode enabled, it ignores the wire drive byte and computes:

```text
slot = LSN / HDBDOS_SECTORS_PER_DISK
local_sector = LSN % HDBDOS_SECTORS_PER_DISK
HDBDOS_SECTORS_PER_DISK = 630
```

Thus LSNs 0–629 address slot zero, 630–1259 address slot one, and so on.
Do not enable this remapping for NitrOS-9 `rbdw`. Keep the mode explicit in
saved VM settings; a handshake must not silently change the saved mode.
[Sector implementation](../crates/coco-core/src/drivewire/transfer.rs)

An empty or unsupported slot returns `NOT_READY` (`0xF6`); a read beyond an
image returns `READ` (`0xF4`); a failed write returns `WRITE` (`0xF5`); a checksum
mismatch returns `CRC` (`0xF3`). Success is zero. A read past the image is a disk
error, not channel EOF. Writes can extend a flat image.

Plain reads return status, then data and checksum only on success. Extended
reads return 256 bytes first, accept the client's checksum, and then return
status. On an extended-read failure, the baseline sends zero data and reports
the pending error after a matching checksum. Preserve this framing.

### Selected first-target policy

Retain four configured disk slots, DW0–DW3, for the first NitrOS-9 command
target. Guest commands can replace or eject these runtime mounts. An empty
slot is still a valid configured slot; it is not proof that a named-object
allocator can take ownership of it. Reject unsupported drive numbers before
calling core mount methods, which index fixed arrays.

The VM definition records startup assignments. Runtime state records the
mounted image, its access mode, and its origin. Guest insertion and ejection
do not rewrite the definition. A cold start reapplies startup assignments;
resume and save-state restore use session media according to the
[settings contract](drivewire-settings.md).

Replacing an image is atomic from the guest's perspective: validate and open
the replacement before removing the old mount. On failure, keep the old mount.
Do not mount over a guest filesystem with open files in acceptance recipes.
Host shares define which images the guest can select; command compatibility
does not grant access to arbitrary host paths or URLs.

### Proposed named-object capacity extension

For the named-object implementation, propose a runtime drive table with the
full 256 wire addresses. Preserve DW0–DW3 as configured disk slots and reserve
DW4–DW255 for dynamically allocated named objects. This is an explicit future
capacity change, not a change made by this document or an additional 252-slot
settings form. Update serialized media references, restore validation, and
activity accounting before enabling it.

Allocate the lowest available dynamic drive deterministically. Never evict a
configured disk to satisfy an object request. If the pool is exhausted or the
host operation fails, return the named-object failure byte. The exact table
representation and migration belong in implementation review.

The [named-object specification][spec-named] defines failure as zero and
success as a drive number from 1 through 255. Each completed named-object
operation ends the previous lease; a successful operation grants a new one.
During a lease, I/O to its drive must reach that object or fail. A failed host
reopen must not redirect traffic to a different image. Reject mount/eject
operations that would replace the active leased drive. An inactive object
mount can be reclaimed by a later named-object operation.

Proposed coexistence rule: while an object lease is active, sector requests
whose wire drive equals the leased drive use direct object addressing before
any HDB-DOS remapping. Other requests retain the selected disk mode. This
exception requires tests with the selected object client; it is not baseline
CocoVM behavior. Reset, cancellation, and snapshot lease behavior must be
specified by the lifecycle implementation before named objects are enabled.

## Version and feature claims

Preserve baseline `DWINIT` framing: opcode `0x5A`, one driver byte, and one
reply byte `0x04`. It is a legacy driver compatibility response, not a bitmap
of functioning services. Disk clients must not lose their existing boot path
because the server gains an optional service.

The specification's version-byte prose is incomplete. The pinned NitrOS-9
[`dwio`][guest-dwio] sends driver byte one and installs its channel poller only
after receiving `0x04` or `0xFF`. Do not replace
`0x04` with an invented version or capability bitmap. pyDriveWire's discovery
sequence is a separate extension and remains in the research backlog.

Describe the baseline as disk/time support with virtual-channel framing
stubs. Advertise a service only after its guest recipe passes, including error
and EOF cases. A successful handshake or boot alone cannot establish channel,
networking, named-object, or host-filesystem compatibility. Until channels work,
do not offer host-access controls that rely on them.

## NitrOS-9 command contract

The [`dw` utility][guest-dw] opens `/N`, configures raw I/O, and sends `dw `
followed by its argument line. Provide SCF, `scdwv`, `dwio`, the wildcard
`n_scdwv` descriptor, and at least numbered `n1_scdwv` and `n2_scdwv`
descriptors for concurrent acceptance. The wildcard is a guest-side allocator,
not wire channel 255. [`scdwv`][guest-scdwv], [network library][guest-net]

Use these command forms. `SHARE` is the proposed per-VM logical share name,
not an unrestricted host path. The share implementation must define its path
grammar and enforce access rules before exposing these commands.

```text
dw server dir SHARE
dw server list SHARE/FILE
dw disk insert 1 SHARE/IMAGE.dsk
dw disk eject 1
```

The Java reference accepts a path or URI as the remaining argument for `dir`
and `list`, and a numeric drive followed by a path for `insert`. CocoVM will
retain the command verbs and response framing while resolving paths inside
assigned shares. Arbitrary URLs and Java's image adapters are separate scope.
The first fixture uses ASCII paths without spaces; path encoding, quoting,
and case handling require explicit share tests.

| Result | Reference command code | Proposed CocoVM behavior |
| --- | --- | --- |
| Missing required argument | 10 | Return syntax failure without opening a host resource. |
| Invalid or unsupported drive | 101 | Reject the command; preserve mounts. |
| Eject an empty drive | 102 | Report that no image is loaded. |
| Image already loaded | 103 | Report failure according to the command policy; preserve mounts. |
| Unsupported image format | 104 | Reject before replacing an image. |
| Filesystem resolution failure | 201 | Report an inaccessible share/path without exposing host paths. |
| Host I/O failure | 202 | Return failure; preserve the old image on insertion failure. |
| Missing image on insertion | 203 | Return failure; preserve the old mount. |

These are command-layer codes, not the sector status bytes. Java `dir` maps
filesystem exceptions to 201; `list` uses 201 or 202; `insert` supplies the
disk-specific codes. Do not promise that every missing path maps to 203.
[Directory command][java-dir], [file command][java-list],
[insert command][java-insert], [eject command][java-eject]

### Status envelope and EOF

Target the pinned Java implementation and NitrOS-9 client pair. Java sends
`OK command successful` followed by LF then CR before a successful payload.
On failure it sends `FAIL `, a three-digit code, a space, the explanation,
and LF then CR. The guest consumes the initial status through CR and strips
it from successful output. The specification instead describes a numeric
status-line envelope; do not substitute that prose for this client's actual
compatibility target. [Command thread][java-command], [guest utility][guest-dw]

For `list`, the server queues the file's raw bytes. The guest writes payload
bytes to standard output without line conversion. The target therefore
preserves NUL, CR, LF, and high-bit bytes when redirected into a guest file.
Implement bounded streaming; the Java implementation's whole-file buffering
is not an ownership or memory-limit requirement. [File command][java-list]

After all response bytes drain, close the channel and report its close status
through polling. Each command opens another channel. Do not append a payload
EOF byte or close while data remains queued. The guest observes remote closure
through `S$HUP`; its `SS.EOF` handler is not a file-style EOF test. Also, this
`dw` source clears its exit code even after displaying a server failure.
Acceptance must inspect the response and channel completion, not rely on a
nonzero shell status. [Guest utility][guest-dw], [driver][guest-scdwv],
[command thread][java-command]

## Virtual-channel target

The first command-service target uses numbered `/N1`–`/N13` descriptors and
wildcard `/N`. Keep wire port zero for the terminal role and port 14 for MIDI;
do not allocate them to wildcard command traffic. `/Z` windows and extended
channel ranges belong to their service follow-up. This is a selected target,
not the protocol's universal channel limit. The pinned descriptor maps port
14 to `/MIDI`, even though the example bootlist names an `n14_scdwv` artifact.
[Descriptor source][guest-descriptor], [bootlist][guest-bootlist]

The specification describes 15-channel groups and a later 30-channel total;
the Java defaults allocate 16 N and 16 Z indices. Do not infer usable channels
from the size of an opcode range. The baseline CocoVM fast-write range is
`0x80`–`0x8F`; consuming those opcodes does not establish working channels.
[Specification][spec], [Java port configuration][java-ports]

Implement the selected driver's open, close, polling, status, and transfer
behavior. Its ordinary writes use `0x80 + channel` and one data byte.
`SERREADM` requests an advertised byte count and receives exactly that many
raw bytes. Only advertise bytes already queued, retain them until consumed,
and poll active channels fairly. Never fabricate zero-filled reads as a
successful host transfer. [`scdwv`][guest-scdwv], [`dwio`][guest-dwio]

For N-channel `c` from one through 13, the pinned client/server exchange is:

| Operation | Bytes, in hexadecimal unless expressed as a formula |
| --- | --- |
| Initialize descriptor | Guest sends `45 c`; this alone does not open the Java port. |
| Open SCF path | Guest sends `C4 c 29` (`SS.Open`); no reply. |
| Set communication options | Guest sends `C4 c 28`, then exactly 26 option bytes; no reply. |
| Write one byte | Guest sends opcode `0x80 + c`, then the byte; no reply. |
| Poll | Guest sends `43`; server returns exactly two bytes. |
| Idle poll response | `00 00`. |
| One input byte | First byte `c + 1`, second byte data. |
| Buffered input | First byte `c + 17`, second byte count; guest follows with `63 c count`, then receives count bytes. |
| Remote close | `10 c`, after queued output drains; guest receives `S$HUP`. |
| Close last SCF path | Guest sends `C4 c 2A` (`SS.Close`); no reply. |
| Terminate descriptor | Guest can send `C5 c`; reference Java consumes it without closing the port. |

Do not confuse descriptor initialization with path opening, or descriptor
termination with `SS.Close`. The lifecycle task must specify local resource
cleanup while preserving these wire shapes. [Guest driver][guest-scdwv],
[guest poller][guest-dwio], [Java protocol handler][java-protocol],
[Java poll responses][java-ports]

For future windows, the specification and this source pair disagree about
poll mode bits: the Java server and NitrOS-9 driver use `0x40`, while the
specification's window table uses `0x80`. Resolve this against the selected
window client before implementing `/Z`; do not extend N-channel formulas
blindly. [Specification][spec], [guest poller][guest-dwio],
[Java poll responses][java-ports]

`SERWRITEM` exists as a protocol operation, but this guest's normal write path
does not exercise it. Its `SS.BlkWr` branch sends unframed data for a different
use and must not be confused with `SERWRITEM`. Cover block writes with protocol
tests or a selected client before claiming them. Unknown or partial requests
must retain framing and bounded timeout behavior; consuming a payload is not
service success. [Driver source][guest-scdwv]

## Named-object reference boundary

Mount uses opcode `0x01`, create uses `0x02`, followed by an unsigned one-byte
name length and exactly that many name bytes, up to 255. Reply with one byte:
zero on failure or the allocated drive on success. Mount fails for an absent
object; create fails for an existing object. Neither operation reports a
detailed error code. An object exposes sector-addressed bytes, not directory
enumeration, rename, or deletion. [Named-object specification][spec-named]

The pinned Java handler dispatches mount but does not dispatch create. It can
also return an already-mounted object in drive zero, contrary to the specified
failure sentinel. Do not use that implementation as proof of create or lease
correctness. Follow the specification's nonzero drive and object-identity
guarantees, then validate them with the selected CoCoBoot client.
[Java protocol handler][java-protocol]

## Sources and evidence

Primary-source revisions match the inventory: DriveWire `a795310`, Java server
`4e57ffe`, NitrOS-9 `0c9940f`, and pyDriveWire `9c85a9f`. Source inspection gives
wire and command expectations; it does not substitute for guest execution.
The [acceptance recipes](drivewire-acceptance.md) distinguish runnable baseline
checks from future service gates.

[spec]: https://github.com/DrPitre/DriveWire/blob/a795310089b00710d797ebc7d0eb0942e439572d/DriveWire%20Specification.md
[spec-named]: https://github.com/DrPitre/DriveWire/blob/a795310089b00710d797ebc7d0eb0942e439572d/DriveWire%20Specification.md#named-objects
[guest-bootlist]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level2/coco3/bootlists/dw.bl
[guest-dw]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level1/cmds/dw.as
[guest-net]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/lib/net.as
[guest-dwio]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level1/modules/dwio.asm
[guest-scdwv]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level1/modules/scdwv.asm
[guest-descriptor]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level1/modules/scdwvdesc.asm
[java-dir]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdServerDir.java
[java-list]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdServerList.java
[java-insert]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdDiskInsert.java
[java-command]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/virtualserial/DWUtilDWThread.java
[java-eject]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdDiskEject.java
[java-ports]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/virtualserial/DWVSerialPorts.java
[java-protocol]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwprotocolhandler/DWProtocolHandler.java
