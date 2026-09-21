# Configure DriveWire

Select a VM in the manager and use its **DriveWire** tab to enable
DriveWire, select **HDB-DOS mode**, and assign images to DW0 through DW3.
Enter a disk image path or click **Browse…** beside a drive to select its
startup image. Clear the path or click **×** inside the field to leave the
drive empty. Empty fields display **No disk image**. Changes save automatically.

All DriveWire settings apply at the next start from power off. Changing
settings while the VM runs leaves its active session and mounted images
intact. Resetting the CPU does not apply these settings. Resuming a suspended
VM or restoring a saved state restores that session's mode and mounts.
Guest-selected runtime mounts do not replace the saved startup assignments.

Disabling DriveWire retains its mode and image assignments for later use.
DriveWire defaults to disabled for definitions without a `[drivewire]`
section. The Games Master Cartridge conflicts with the Becker port, including
when the cartridge occupies a MultiPak slot. Remove the cartridge or disable
DriveWire before saving the conflicting change.

Selecting the HDB-DOS DOS ROM for the FD-502 turns on DriveWire and HDB-DOS
mode once, as a default. Both checkboxes stay editable, and the VM starts with
whatever the definition records, so an HDB-DOS machine with DriveWire disabled
starts without a Becker port.

The definition records startup settings in a separate section:

```toml
[drivewire]
enabled = true
hdbdos_mode = false
disk0 = "system.dsk"
disk3 = "/path/to/data.vhd"
```

Omitted disk keys represent empty drives. Relative paths resolve against the
VM's artifact directory, as other media paths do. Missing or inaccessible
images prevent startup and display an error in the VM's detail pane. Images
open for reading and writing, and disk writes update their backing files.

Each VM owns its DriveWire session. Its activity indicators remain in the
running window's status bar. Host shares and service options belong in this
settings section as those features become available.

File-backed disk transfers run on a host worker. Hover over a DriveWire status
entry to inspect pending work and errors. If host I/O is pending, save, suspend,
and restore report an error so you can retry after it finishes. See
[host-service lifecycle](drivewire-lifecycle.md) for reset and cancellation behavior.
