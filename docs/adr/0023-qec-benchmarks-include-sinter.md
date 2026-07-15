# QEC benchmarks nest schedule ablations with tiny Sinter samples

The #254 harness runs a grid over QEC workloads (repetition/surface memory, optional CX) and compiler ablations (`--na-backend`, `--na-placer`, compaction on/off). Each cell records schedule/resource/error-budget fields and also runs a tiny fixed-seed Sinter sample so the CSV includes sampled logical failure columns.

Separating schedule-only benchmarks from Sinter would under-deliver the epic’s “reproducible evaluation” story; claiming RAP Table I (#111) numbers for QEC workloads would be dishonest — the harness may cite #111 only as the physical-NA external methodology anchor, with QEC rows clearly marked as a distinct experiment class. CI stays a single tiny grid point; full grids are local.
