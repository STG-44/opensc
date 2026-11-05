# CPU diagnostics

Run against an installed original copy without creating a window or GPU:

```sh
cargo run -p shadow-model-viewer --bin shadow-game -- --game-dir /path/to/original/game --cpu-profile-json /tmp/opensc-cpu.json
```

The report measures eight visibility updates with a moving observer, completed
background fog scans, nonblocking result polling and short navigation queries
in the actual training scene. It also identifies the actor's selected standing
walk/run clips and audits loop seams. Diagnostic code waits for the worker
between samples; ordinary game updates never wait for it.

`visibility_update_ms` measures main-thread submission/contact work.
`visibility_worker_ms` measures the fog scan itself; `maximum_poll_ms` measures
checking and applying a completed mask. These are CPU timings, not frame rate.
Loading, shader compilation, rendering, GPU stalls and display pacing are
outside this report. Compare equivalent build profiles and machines.

On the local unoptimized build before the scheduling fix, the 64 × 64 training
fog scan blocked the main thread for 2,092–2,144 ms per refresh (eight samples,
mean 2,124 ms). The configured refresh interval is 750 ms. Published Test uses
an optimized release build, so these absolute debug timings do not describe
its frame rate or other hardware. The diagnostic confirms a blocking workload;
the new scheduling removes that scan from the frame update.

With the same local profile after the fix, the first submission (pool/snapshot
initialization included) took 13.50 ms; seven subsequent submissions took
0.015–0.026 ms. Completed scans took 58–65 ms on the worker and polling/applying
the result took at most 0.052 ms. These eight samples demonstrate the reduced
main-thread workload, not a frame-rate guarantee. The two short navigation
queries in this run took 0.51 and 2.40 ms.

Root travel now samples only root translation instead of allocating two unused
skeletal poses. The visible pose interpolation and sequence timing remain
unchanged. John's stance mapping already selects the original `MSEQ_WALK`
loop, `bipedal_walk`; the old `bipedal_stand_walk_80` preview must not be used
as a fallback walking cycle because it includes a stop pose.

For validation, run the game unit tests (visibility reset/cache behavior,
immutable geometry, spatial queries and root travel), followed by the native
tutorial and vehicle input fixtures. Frame pacing still needs comparison on
hardware graphics devices; a software-rendered local capture cannot verify
cross-device performance.
