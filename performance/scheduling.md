# Schedule frontend work

The frontend uses absolute host deadlines for emulation service and presentation.
Input and control arrivals can wake the UI before a deadline. Extra UI updates do
not postpone the next deadline or advance TV noise by themselves.

Each VM uses its own viewport's focus and minimized state. A known unfocused or
minimized VM uses the background policy. Unknown focus remains at foreground
cadence unless minimization is known. egui's `ViewportInfo` does not expose
occlusion or general visibility, so covered windows use this fallback.

## Emulation and audio

Foreground service runs at the machine's configured field rate. Background
service runs one field before its audio cushion expires. The cushion starts at
two fields in the foreground and targets 100 ms in the background. Its field
count is capped so the cushion plus the maximum eight-field catch-up fits the
250 ms audio queue. This gives six cushion fields and five service fields for
NTSC, and four cushion fields and three service fields for PAL.

Increasing the cushion runs the additional fields ahead of wall-clock time.
Reducing it subtracts those fields from the clock debt, so the VM waits for real
time to catch up. Only completed fields count toward the cushion. A debugger
breakpoint stops further execution. Existing pause, reset, and restore paths
clear their clock or audio accounting.

Incidental UI events can still service owed fields and input. Field debt tracks
elapsed time independently of presentation cadence. A host stall retains the
existing 250 ms elapsed-time clamp and eight-field catch-up limit. These limits
bound recovery work; they do not guarantee real-time execution through arbitrary
host stalls.

## Presentation and animation

Running VMs consider framebuffer updates on their service cadence. This bounds
conversion and texture uploads during unrelated UI activity. Framebuffer and
settings changes appear at the next presentation deadline. The existing source
and settings cache skips unchanged textures.

Paused and suspended TVs with noise use a 60 Hz presentation timer in the
foreground and a 100 ms timer in the background. Noise seeds still follow the
elapsed 60 Hz clock, skipping missed animation ticks. Static paused and suspended
displays request no periodic repaint. Visible runtime statistics request a
one-second update only while the selected VM is executing.

The manager uses egui immediate viewports. Their repaint path executes the parent
and child UI callbacks together, so this change cannot isolate native window
presents. Per-VM deadlines limit framebuffer work even when another viewport
causes a callback. Changing viewport ownership would require a separate frontend
architecture change.

## Deferred controls

Incoming requests wake the manager through the control server's existing event
callback. The manager resolves deferred operations after the VM callbacks run,
so completed fields and typing or key-release progress are available immediately.
Pending requests schedule the earliest absolute timeout rather than a continuous
poll. A paused target can resume and complete, disappear and fail, or reach its
timeout. Completed operations retain precedence over an elapsed timeout.

## Backend timing

The pinned egui 0.33 implementation subtracts `predicted_dt` from repaint delays.
The deadline adapter compensates for that subtraction, preventing a one-field
or animation delay from becoming an immediate repaint request. Input events can
still wake the UI earlier. Native event delivery, rendering, and buffer swaps can
make a requested wake late; the audio cushion and bounded field debt account for
ordinary scheduling jitter.

Deterministic sibling tests exercise deadline advancement, incidental repaints,
focus and minimized transitions, unknown viewport state, mixed schedules,
pause/resume, host stalls, audio budgets, and deferred-control outcomes. Native
measurement results describe the observed timing and the limits of measurement
on the test host.
