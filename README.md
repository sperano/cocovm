# CocoVM

A Tandy Color Computer (CoCo 1/2/3) emulator in Rust + egui, aiming for
Virtual ][-level polish.

- `crates/mc6809` — MC6809 CPU core
- `crates/coco-core` — the headless machine (GIME, SAM, PIAs, disk, tape, sound…)
- `crates/coco-egui` — the frontend: a VirtualBox-style VM manager with
  per-machine suspend/resume, and a built-in MCP server for driving a VM
  from an AI
- `book/` — a 16-chapter course that builds the emulator from scratch

Run `cargo run` to open the VM manager. ROM images are not included in the
repository; the manager downloads them on first launch into its data
directory (`~/.local/share/cocovm/roms/` on Linux/macOS).

## Driving a VM from an AI (MCP)

`cocovm` serves an [MCP](https://modelcontextprotocol.io) server directly —
no separate process. It listens on `http://127.0.0.1:6809/mcp` by default
("streamable HTTP" transport, JSON responses only); override the port with
`--control-port` or `COCOVM_CONTROL_PORT` (`0` disables it).

Register it with Claude Code:

```sh
claude mcp add --transport http cocovm http://127.0.0.1:6809/mcp
```

Or, for any MCP-compatible client, add an HTTP server pointing at that URL.

The server exposes tools to list and start VMs, type BASIC and press keys,
read the screen as text or a screenshot, mount/eject disks, reset or
pause/resume a VM, wait for video fields to elapse, and peek/poke memory
(`list_vms`, `start_vm`, `screen_text`, `screenshot`, `type_text`,
`press_keys`, `joystick`, `insert_disk`, `eject_disk`, `reset`,
`set_running`, `wait`, `peek`, `poke`). Every per-VM tool takes an optional
`vm` argument (the manager slug); omit it when only one VM is running. Full
argument schemas are in `tools/list`.

Example session:

```
list_vms                              -> "coco3 — CoCo 3 (powered_off)"
start_vm {"vm": "coco3"}
wait {"vm": "coco3", "fields": 300}   -- let BASIC boot to its OK prompt
type_text {"vm": "coco3", "text": "PRINT 2+2\n"}
wait {"vm": "coco3", "fields": 60}
screen_text {"vm": "coco3"}           -> the screen now shows "4"
```
