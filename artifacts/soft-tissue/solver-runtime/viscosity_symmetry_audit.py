"""Verify the viscosity optimization against the strict-convergence baseline."""
import hashlib
import json
from pathlib import Path

from live_clock_audit import inspect as inspect_live
from startup_cadence_audit import inspect as inspect_fixed, compare

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent


def main():
    inputs = json.loads((HERE / "viscosity-symmetry-inputs.json").read_text())
    assert all(hashlib.sha256(Path(path).read_bytes()).hexdigest() == digest
               for path, digest in inputs["files"].items())
    launches = json.loads((HERE / "viscosity-symmetry-launches.json").read_text())
    assert len(launches) == 2 and all(r["exit_code"] == 0 for r in launches)
    assert all(r["binary_sha256"] == inputs["files"]["target/debug/examples/tissue_lab"] for r in launches)
    folders = [HERE / name for name in ["viscosity-symmetry-baseline-01", "viscosity-symmetry-01"]]
    runs = [json.loads((folder / "run.json").read_text()) for folder in folders]
    counters = [json.loads((folder / "counters.json").read_text()) for folder in folders]
    assert (folders[0] / "positions.bin").read_bytes() == (folders[1] / "positions.bin").read_bytes()
    for field in ["linear_solves", "cg_steps", "true_relative_residual", "nonlinear_force_n_correction_m"]:
        assert counters[0][field] == counters[1][field], field
    for field in ["steps", "regions", "subdivisions", "metrics"]:
        assert runs[0][field] == runs[1][field], field
    packets = (ROOT / "lab-jump-01/packets.ndjson").read_bytes()
    before = inspect_fixed("lab-front-nonlinear-strict-fixed-01", packets)
    after = inspect_fixed("lab-front-viscosity-symmetry-review-01", packets)
    regression = compare(before[1], after[1])
    assert regression["identical_traced_physics_and_source"]
    live = inspect_live("viscosity-symmetry-live120-01", packets)[0]
    report = {
        "source_manifest": "viscosity-symmetry-inputs.json",
        "source_sha256": hashlib.sha256(packets).hexdigest(),
        "cpu_positions_and_convergence_records_bit_identical": True,
        "cpu": {
            "baseline": runs[0], "optimized": runs[1],
            "linearize_seconds_before_after": [c["linearize_ns"] / 1e9 for c in counters],
            "solve_time_reduction_percent": 100 * (1 - runs[1]["solve_seconds"] / runs[0]["solve_seconds"]),
        },
        "gpu_physics_regression": regression,
        "fixed_clock_review": after[0],
        "live_timing": live,
        "limitations": [
            "Single shared-host measurements; CPU kernel replay is not a real-time renderer benchmark.",
            "Fixed-clock image capture is for visual/numerical review, not an interactive performance claim.",
            "Live capture uses prerecorded native motion; it does not measure browser or generation latency.",
            "The optimization preserves the strict baseline; it does not calibrate tissue or validate active avatar contact.",
        ],
    }
    (HERE / "viscosity-symmetry-comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
