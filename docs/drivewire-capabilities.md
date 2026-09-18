# DriveWire capabilities and server design

This is a capability inventory and proposed design for extending CocoVM's
DriveWire server. It does not claim implementation or compatibility beyond the
existing code. The CocoVM baseline is `fcd60cf`.

CocoVM already embeds a disk-oriented DriveWire server in each VM. The useful
extension is host-file access and the services layered on virtual channels.
Keep protocol sessions per VM, with optional shared resource definitions in the
manager. Choose the guest interface for directory access before implementing it.

## Protocol capabilities

DriveWire combines byte-level transactions with higher-level services. A server
can implement one layer without implementing every service that uses it.

| Capability | Guest-visible behavior | CocoVM baseline |
| --- | --- | --- |
| Disk sectors | Read, write, retry, and extended-read transactions transfer 256-byte sectors. The wire carries a drive byte and a 24-bit sector number. [Specification][spec] | Implemented for four flat-file or memory images. |
| Lifecycle and time | Initialization, reset, driver handshake, no-op, and host date/time. [Specification][spec] | Clock and handshake work. Reset, init, and term are accepted without reset side effects. |
| Disk status | Guest disk status notifications. [Specification][spec] | Payloads are consumed without status-specific behavior. |
| Virtual serial channels | Multiplexed channel open/close, polling, byte/block I/O, status, and fast writes. [Specification][spec] | Framing stubs only; no usable channels. |
| Printing | Queue bytes and flush a job. [Specification][spec] | Absent from DriveWire. |
| Named objects | Mount or create an object by name, then access it through a returned drive number. [Specification][spec] | Absent. |
| Host directory browsing | `dw server dir` lists a host path or supported URI. [Java directory command][java-dir] | Absent. |
| Host file retrieval | `dw server list` returns file content through a command response. This is separate from opening a guest filesystem file. [Java file command][java-list] | Absent. |
| Remote disk management | Guest commands select a disk-image path and mount it in a virtual drive. [Java disk command][java-disk] | Mount/eject exists only in the frontend. |
| Networking | Virtual ports provide outgoing TCP, incoming listeners, and connection attachment. Telnet and modem-style commands are services above the channels. [Java virtual-port handler][java-ports] | Absent. |
| Host serial bridge | `ser join` connects a virtual channel to a physical host serial port. [Java serial API][java-serial] | Absent from DriveWire. |
| MIDI | The Java server routes channel data to MIDI output and exposes synthesizer controls. [Java MIDI routing][java-midi], [controls][java-midi-controls] | Absent from DriveWire. |
| Host terminals | Java virtual-window channels connect to a host terminal service. [Java virtual-port handler][java-ports] | Absent. |
| Remote filesystem extension | Java dispatches `OP_RFM` to an incomplete file-oriented service. Matching NitrOS-9 sources also have stubs. [Java RFM handler][java-rfm], [guest file manager][guest-rfm] | Absent. |
| WireBug | Register/memory debugging is described, but the specification marks it unimplemented. [Specification][spec] | Absent from DriveWire; CocoVM has a separate debugger. |

The Java implementation also has management interfaces for configuration,
logging, ports, printers, and server status. These are server product features,
not automatic consequences of accepting disk transactions. Its GUI control
interface is separate from the guest's disk protocol. [Java server source][java-root]

pyDriveWire adds implementation-specific features: SSH, per-service directories,
aliases, capability discovery through a `DWINIT` sequence, and experimental host
sound playback. EmCee and DLOAD are additional protocols hosted by the same
program. Dragon DOS Plus handling and image-format adapters are compatibility
features, not a generic host-filesystem mount. pyDriveWire's README explicitly
lists MIDI, OS-9 `/Z` consoles, and MShell as missing features.
[pyDriveWire manual][py-manual], [README][py-readme], [sound handlers][py-server]

## What serving a directory means

There are distinct guest interfaces to evaluate:

| User goal | Interface | Implication |
| --- | --- | --- |
| Browse a folder of disk images and insert one | `dw server dir` plus `dw disk insert` | Implement virtual channels, command responses, and a path resolver. The guest continues to use the disk filesystem inside the selected image. |
| Retrieve a host file from an OS-9 command | `dw server list` over a virtual channel | Useful for command-driven transfer. Define byte preservation and end-of-file behavior against the chosen client. |
| Load or save an object by name | Named-object mount/create plus sector I/O | Requires guest software that speaks named objects. It does not provide directory enumeration, rename, or delete by itself. |
| Use host folders as ordinary guest files and directories | Remote file manager, or a filesystem adapter | Evaluate the Java RFM extension and matching guest modules. A synthetic disk would be a separate design with filesystem translation and write-back rules. |

The first two interfaces are verified in the Java command implementations.
[Directory listing][java-dir], [file retrieval][java-list], [image mounting][java-disk]

Named objects use a temporary association between a name and a drive. Do not
reuse that drive for another object while the association remains valid.
[Named-object specification][spec-named]

RFM needs more than enabling an existing opcode. The inspected Java handler
implements some file operations, but binary write, delete, change-directory,
and make-directory only log the request. Its path setter replaces the supplied
path with `/`, and directory reads use a buffer that directory opening does not
populate. These are reasons to avoid treating this revision as a working
filesystem reference. [RFM dispatch][java-rfm], [path implementation][java-rfm-path]

The NitrOS-9 file manager uses the same `0xD6` opcode under the name `OP_VFM`.
Its directory-changing operations send only operation bytes, and its size and
position status handlers are stubs. A guest/server interoperability investigation
must precede a commitment to transparent folder access. The inspected CoCo 3
DriveWire bootlist does not include RFM modules.
[Guest file manager][guest-rfm], [bootlist][guest-bootlist]

Selecting a directory in CocoVM therefore needs a defined guest contract.
A folder containing `.dsk` images and a folder containing individual host files
are different features. Ordinary HDB-DOS disk access alone cannot distinguish
host filenames because CocoVM receives sector requests.

## Existing implementation and gaps

The relevant code is split across these modules:

- [`drivewire.rs`](../crates/coco-core/src/drivewire.rs) owns four drive slots,
  image backends, the injected clock, reply queue, counters, and serialized state.
- [`protocol.rs`](../crates/coco-core/src/drivewire/protocol.rs) frames requests.
  Serial polls return idle, reads return zero-filled payloads, and writes are
  discarded. `SERWRITEM` is not dispatched. A version-four handshake reply does
  not establish virtual-channel support.
- [`transfer.rs`](../crates/coco-core/src/drivewire/transfer.rs) implements sector
  checksums and HDB-DOS addressing. Its HDB-DOS mode selects a slot from the
  sector number, with 630 sectors per image. Keep this separate from host paths.
- [`bus.rs`](../crates/coco-core/src/bus.rs) owns the server for each machine.
  [`bus/io.rs`](../crates/coco-core/src/bus/io.rs) feeds it synchronously through
  the in-process Becker port. There is no external DriveWire TCP service.
- The [DriveWire menu](../crates/coco-egui/src/chrome/menu_bar/drivewire.rs)
  enables the port, selects HDB-DOS mode, and mounts images. It prevents enabling
  Becker alongside the conflicting Games Master cartridge.
- [Media handling](../crates/coco-egui/src/media/drivewire.rs) opens images for
  direct writes. It has no read-only mount choice or cross-VM write coordination.
- [`AppParams`](../crates/coco-egui/src/app.rs) has a `DriveWireLaunch` payload,
  but [production launch](../crates/coco-egui/src/launch.rs) leaves it unset.
  Machine definitions do not persist DriveWire settings for a fresh start.
- [Save states](../crates/coco-egui/src/save_state/save.rs) record media references.
  [Restore](../crates/coco-egui/src/save_state/restore.rs) reopens image files and
  reinjects the clock. Protocol state is serialized separately from host handles.
- [Machine reset](../crates/coco-core/src/machine.rs) retains DriveWire state.
  Open channels and pending host operations need an explicit reset policy.

Existing [protocol tests](../crates/coco-core/src/drivewire_test.rs),
[bus tests](../crates/coco-core/tests/drivewire_bus.rs), and
[real-ROM boot tests](../crates/coco-core/tests/drivewire_boot.rs) cover the disk
service and stub framing. Booting NitrOS-9 does not test file sharing or working
virtual serial channels.

## Proposed ownership

Retain one protocol session per VM. A session owns its parser, pending replies,
mount table, channel state, selected directories, and named-object associations.
This follows the existing machine ownership and prevents one guest's reset or
mount command from changing another guest's session.

The manager can own reusable share definitions and coordinate access to common
host files. Each VM explicitly selects those shares. Sharing a root does not
require sharing a current directory, channel number, or drive table. Define how
multiple VMs write the same image or host file before allowing concurrent writes.

Use the in-process Becker path for local VMs. An external TCP listener or serial
adapter is a separate transport that can create its own sessions if external
clients become part of the scope. A global listening socket does not imply one
global protocol state machine.

Keep slow filesystem and network work outside synchronous bus writes. Introduce
bounded requests and completion queues with backpressure, keeping the protocol
parser in the core and host-service ownership outside serialized machine state.
The existing [serial endpoint](../crates/coco-core/src/serial.rs) is an
architectural reference, but its byte-dropping backpressure behavior is unsuitable
for DriveWire transfers.

Persist enablement, image mounts, selected shares, and compatibility settings in
the VM definition. Specify reset, stop, suspend, and snapshot behavior for every
resource. Snapshots must not imply that a TCP connection or open host handle can
be restored by deserializing it.

## Proposed implementation sequence

This sequence is a recommendation, not an approved reduction to a particular
subset of DriveWire:

1. Select the directory experience and guest software to support. Evaluate RFM
   before claiming transparent host-folder access.
2. Persist per-VM DriveWire configuration and define share roots, write access,
   path handling, and resource lifecycle.
3. Implement functional virtual channels with bounded buffering, polling,
   status, block transfers, and end-of-file behavior.
4. Add directory listing, file retrieval, and disk selection through compatible
   commands. Add named objects if the selected clients require them.
5. Add direct filesystem access through the chosen guest interface, if selected.
6. Implement printing, networking, MIDI, terminals, or server-specific extensions
   according to the selected compatibility target.

Each implemented layer needs sibling tests for malformed or incomplete requests,
timeouts, errors, and two independent VM sessions. Host-file tests must cover
paths outside the selected root, symlinks, read-only roots, and conflicting writes.
Run guest-level acceptance with the actual NitrOS-9 modules or BASIC tools that
use each service. Preserve the existing HDB-DOS and NitrOS-9 boot coverage.

## Sources and validation boundary

The inventory uses the official specification and primary server sources.
Reference revisions are pinned: DriveWire `a795310`, Java DriveWire `4e57ffe`,
pyDriveWire `9c85a9f`, and NitrOS-9 `0c9940f`.
This is source inspection, not an interoperability test.
The specification includes unfinished sections, so an opcode name alone is not
evidence of an operational feature.

[spec]: https://github.com/DrPitre/DriveWire/blob/a795310089b00710d797ebc7d0eb0942e439572d/DriveWire%20Specification.md
[spec-named]: https://github.com/DrPitre/DriveWire/blob/a795310089b00710d797ebc7d0eb0942e439572d/DriveWire%20Specification.md#named-objects
[java-root]: https://github.com/qbancoffee/drivewire4/tree/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver
[java-rfm]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwprotocolhandler/DWRFMHandler.java
[java-rfm-path]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwprotocolhandler/DWRFMPath.java
[guest-rfm]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level1/modules/rfm.asm
[guest-bootlist]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level2/coco3/bootlists/dw.bl
[java-dir]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdServerDir.java
[java-list]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdServerList.java
[java-disk]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdDiskInsert.java
[java-ports]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/virtualserial/DWVPortHandler.java
[java-serial]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/virtualserial/api/DWAPISerial.java
[java-midi]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/virtualserial/DWVSerialPorts.java
[java-midi-controls]: https://github.com/qbancoffee/drivewire4/blob/4e57ffef5340b521f004597b7d002604f5809b32/drivewire4_maven/src/main/java/java/com/groupunix/drivewireserver/dwcommands/DWCmdMidi.java
[py-manual]: https://github.com/n6il/pyDriveWire/blob/9c85a9fe43234c7ee3f57efaa49e05902884ae36/docs/The%20pyDriveWire%20Manual.md
[py-readme]: https://github.com/n6il/pyDriveWire/blob/9c85a9fe43234c7ee3f57efaa49e05902884ae36/README.md
[py-server]: https://github.com/n6il/pyDriveWire/blob/9c85a9fe43234c7ee3f57efaa49e05902884ae36/dwserver.py
