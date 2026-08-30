# coco-mcp

An MCP server (stdio, newline-delimited JSON-RPC 2.0) that exposes a running
`cocovm` app's control protocol ([`coco-control`](../coco-control)) as tools,
so an AI can drive a CoCo 3 VM: type BASIC, press keys, read the screen,
manage disks, peek/poke memory.

The app must already be running — `cocovm-mcp` connects out to its control
port, it doesn't launch the app itself.

## Build

```sh
cargo build --release -p coco-mcp
```

The binary is `target/release/cocovm-mcp`.

## Register with Claude Code

```sh
claude mcp add cocovm -- /path/to/target/release/cocovm-mcp
```

Or, for any MCP-compatible client, add to its server config:

```json
{
  "mcpServers": {
    "cocovm": {
      "command": "/path/to/target/release/cocovm-mcp",
      "args": []
    }
  }
}
```

## Control port

`cocovm` listens on `127.0.0.1:6809` by default. Both sides read the same
override:

- `--control-port <PORT>` / `--port <PORT>` (app / `cocovm-mcp`, respectively)
- `COCOVM_CONTROL_PORT` environment variable

If the app isn't running or isn't reachable on that port, every tool call
returns an `isError` result explaining where it looked.

## Tools

| Tool | Purpose |
| --- | --- |
| `list_vms` | List every VM the manager knows, with its status |
| `start_vm` | Start (or resume) a VM by slug |
| `screen_text` | Read the text screen as lines, plus video mode |
| `screenshot` | Capture the framebuffer as a PNG |
| `type_text` | Type text through the keyboard type-ahead |
| `press_keys` | Hold a key chord, then release it |
| `joystick` | Set a joystick's axes and/or buttons |
| `insert_disk` / `eject_disk` | Mount/unmount a disk image in a floppy drive |
| `reset` | Reset (optionally hard) the VM |
| `set_running` | Pause or resume emulation |
| `wait` | Let video fields elapse before replying |
| `peek` / `poke` | Read or write VM memory |

Every per-VM tool takes an optional `vm` argument (the manager slug); omit
it when only one VM is running. Full argument schemas are in `tools/list`.

## Example session

```
list_vms                              -> "coco3 — CoCo 3 (powered_off)"
start_vm {"vm": "coco3"}
wait {"vm": "coco3", "fields": 300}   -- let BASIC boot to its OK prompt
type_text {"vm": "coco3", "text": "PRINT 2+2\n"}
wait {"vm": "coco3", "fields": 30}
screen_text {"vm": "coco3"}           -> the screen now shows "4"
```
