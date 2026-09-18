# DriveWire guest acceptance recipes

Use these recipes with the [guest contract](drivewire-guest-contract.md).
The disk checks are implemented. The command and named-object recipes are
future acceptance gates, not completed interoperability tests.

## Run the existing disk baseline

From the repository root, run:

```sh
cargo test -p coco-core --test drivewire_boot --test drivewire_bus -- --nocapture
```

The installed assets resolve through `cocovm-test-assets` under
`~/.local/share/cocovm/assets/`. Missing assets can cause tests to return early.
Inspect output for skip notices before reporting compatibility.

At baseline `685cb87`, all three boot tests and seven bus tests passed on
September 18, 2026, with no asset skips. The boot tests cover HDB-DOS `DIR`,
HDB-DOS `SAVE` to a scratch image, and NitrOS-9 boot followed by a shell `dir`.
NitrOS-9 reported 321 sector reads, zero unknown opcodes, and 14 virtual-serial
operations. Those serial operations exercised stubs, not a command service.

These SHA-256 values identify the local fixtures used for that run:

| Asset, relative to `assets/` | SHA-256 |
| --- | --- |
| `roms/coco3.rom` | `ad358430568250eb8330a37f85361d9ce67e5ae6d5a28c17b31802cd3953a5d2` |
| `roms/hdbdw3bc3.rom` | `a7342acfd01d466a7d4fe701dd8912dc1951a54395a5042d0b9446451aa905e7` |
| `tests/nos96809l2v030300coco3_becker.dsk` | `6fbea89a3544cf25d5cc6dc3ba5c70098f2b66bf234c4c20dbffac7bdb262cda` |
| `tests/spetris.dsk` | `72138b63f86b380774c370c8307146f3bf78db3c034fd9e950b1da7798bea662` |
| `tests/blank02.dsk` | `5addb3c6d487b227cda9033b091ab2334d60a52a6fd907fa4a83f968468c5f06` |

For manual reproduction, select a CoCo 3 with the Becker HDB-DOS cartridge and
enable DriveWire in VM settings. Use a scratch DECB image with HDB-DOS mode
enabled for BASIC `DIR` and `SAVE`. For NitrOS-9, mount a scratch copy of the
Becker boot image in DW0, disable HDB-DOS mode, and enter `DOS` at the BASIC
prompt. At the NitrOS-9 shell, `dir` must list `CMDS` and `SYS`.

The automated `DIR` fixture mounts `spetris.dsk` after BASIC reaches `OK`,
because its `AUTOEXEC.BAS` otherwise starts a game. Use a disk without an
autoexec program when reproducing through saved startup assignments.

## Prepare the NitrOS-9 command fixture

This gate requires the lifecycle, shares, virtual-channel, and command
implementations. It cannot pass against the baseline server.

Use a separate checkout of NitrOS-9 with its documented build tools installed.
From that checkout, build the CoCo 3 6809 Becker recipe:

```sh
git checkout 0c9940fd3517c633e0f5d6abce357681e7712698
make -C recipes/coco3/dw CPU=6809 \
  DW_BOOT_MODULE=boot_dw_becker \
  DWIO_MODULE=dwio_becker.sb
```

The output is `recipes/coco3/dw/l2_coco3_dw.dsk`. The [recipe][recipe] includes
SCF, `scdwv`, `/N`, `/N1`–`/N5`, `rbdw`, `dwio`, `/X1`, and the `dw` utility.
Both overrides matter: the defaults use the bit-banger transport. This recipe
differs from the [example bootlist][bootlist], whose channel entries are
commented out. The source-derived build command is a future gate; it was not
executed for this documentation change.

Prepare the acceptance environment:

1. Record the build command, tool versions, module identities, any recipe
   changes, and disk SHA-256 in the implementing PR. Do not assume the
   installed 3.3.0 image contains these exact source revisions.
2. Mount a scratch copy of the built image in DW0 and boot with HDB-DOS mode
   disabled. Keep `/DD` writable for output.
3. Use `mdir -e` to verify that `dwio`, `scdwv`, `N`, `N1`, and `N2` are
   resident after boot. Use `dir /dd/cmds` to verify that `dw` is installed.
4. Create a host folder containing a valid scratch OS-9 disk image
   `alpha.dsk`, a second image `beta.dsk`, a text file `hello.txt`, and
   `probe.bin`. Define `probe.bin` as the byte sequence 0 through 255 repeated
   four times, for exactly 1,024 bytes. Also create an empty file `empty.bin`.
5. Assign this folder as logical share `acceptance` in the VM's DriveWire
   settings. The share implementation must make the following paths resolve
   within that root. Do not enable arbitrary host-path access for the recipe.

## Exercise commands and end-of-file behavior

Run these commands from the guest shell:

```text
dw server dir acceptance
dw server list acceptance/hello.txt
dw server list acceptance/probe.bin >/dd/probe.out
dw server list acceptance/empty.bin >/dd/empty.out
dw disk insert 1 acceptance/alpha.dsk
dir /x1
dw disk eject 1
dw server dir
dw server list acceptance/missing.txt
dw disk insert 255 acceptance/alpha.dsk
```

Require both image names in the share listing, the expected text, and a
return to the shell after every command. Extract `/dd/probe.out` with a
validated OS-9 image tool and compare its bytes and length against the host
fixture. Require an empty `/dd/empty.out`. Neither file may contain the status
envelope or an extra EOF byte. Record the extraction tool and version.

`dir /x1` must list the inserted image's guest filesystem, which is separate
from `dw server dir` listing the host share. Eject only after the guest closes
files on `/x1`. The final three commands must report readable errors and
return to the shell. Inspect the wire response for the command code; this
`dw` utility does not reliably propagate server failures as shell exit codes.

Test failed replacement separately: mount `alpha.dsk`, request a nonexistent
replacement, and confirm `/x1` still reads `alpha.dsk`. Restart from power off
and confirm saved startup assignments return. Saving an unrelated setting
while running must preserve the guest-selected mount.

Add these protocol and integration cases before claiming the command target:

- Transfer data containing opcode values, NUL, CR, LF, and high-bit bytes;
  fragment requests and responses across multiple polls and host completions.
- Read more than 255 bytes to exercise repeated poll/block-read cycles.
  Advertise only queued bytes and drain them before reporting closure.
- Run two commands concurrently through `/N`, with at least `N1` and `N2`
  available, and compare each output independently.
- Run commands in two VMs with different share contents; verify that paths,
  channels, mounts, failures, and resets remain isolated.
- Test queue saturation, partial-request timeout, reset during a host read,
  close with queued data, and stale completions after restore.
- Reject traversal, symlink escape, access to unassigned shares, writes to
  read-only roots, and conflicting image writes under the share policy.
- Interleave disk I/O with channel traffic and rerun the disk baseline.

## Gate named objects and ordinary host folders separately

The named-object follow-up must pin a runnable CoCoBoot build and record its
ROM or boot-image hash. A protocol harness alone is insufficient guest
acceptance. Use the named-object wire contract to test mount-existing,
mount-missing, create-new, create-existing, maximum name length, and exhaustion.
Read and write through the returned nonzero drive and verify the host bytes.

Hold a lease while attempting a conflicting disk mount. Require that I/O
continues to reach the same object or fails. Issue another named-object call,
including a failing call, and verify the prior lease ends. Test the proposed
direct-addressing exception with HDB-DOS mode both enabled and disabled.
Test reset and restore against the lifecycle policy selected before release.

For transparent folders, the separate research task must first produce a
working client/server pair or select a filesystem adapter. Require ordinary
guest open, read, write, directory traversal, rename, and deletion tests
against host files before advertising that service. `dw server list` or
mounting a disk image cannot satisfy that gate.

[bootlist]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/level2/coco3/bootlists/dw.bl
[recipe]: https://github.com/nitros9project/nitros9/blob/0c9940fd3517c633e0f5d6abce357681e7712698/recipes/coco3/dw/recipe.mak
