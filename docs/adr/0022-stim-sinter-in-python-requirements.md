# Ship Stim/Sinter in the main Python requirements

`stim` and `sinter` are added to `python/requirements.txt` alongside the existing Qiskit Aer deps, so `just setup-python` and CI smoke tests can import them without a second requirements file.

A separate optional requirements file would reduce install size but would also make the QEC smoke path easy to skip or drift from the documented setup. The QEC harness CI smoke stays tiny-shot and deterministic; large sweeps remain local-only commands documented in the script help.
