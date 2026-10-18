"""Record one declared, semantic-only ARDY motion attempt without editing poses."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--description", default="A person stands upright, jumps straight up once, lands on both feet, and then stands still.")
    parser.add_argument("--seed", type=int, default=731)
    parser.add_argument("--session-id", default="tissue-lab-jump-731")
    args = parser.parse_args()
    if not args.description.strip() or not args.session_id.strip():
        parser.error("description and session ID must be nonempty")
    args.output.mkdir(parents=True, exist_ok=False)
    request = dict(kind="motion_goal", version=2, session_id=args.session_id,
                   generation_revision=1, pts_ms=0, seed=args.seed,
                   goal=dict(description=args.description))
    (args.output / "attempt-plan.json").write_text(json.dumps(
        dict(request=request, seconds=8, attempts=1, policy="Semantic only; preserve every generated sample."), indent=2))
    os.environ["ARDY_JOURNAL_DIR"] = str(args.output / "journal")
    with contextlib.redirect_stdout(sys.stderr):
        from ardy_worker import ArdySession, load_office_model
        service = ArdySession(load_office_model())
    with (args.output / "requests.ndjson").open("w") as requests, \
         (args.output / "responses.ndjson").open("w") as responses, \
         (args.output / "packets.ndjson").open("w") as packets:
        while request["pts_ms"] < 8000:
            requests.write(json.dumps(request) + "\n")
            requests.flush()
            with contextlib.redirect_stdout(sys.stderr):
                response = service.step(request)
            responses.write(json.dumps(response, allow_nan=False) + "\n")
            responses.flush()
            if response.get("audit") != dict(policy="model-owned-v1", future_nonzero=0, future_channels=[], exceptions=[]):
                raise RuntimeError("Generated response did not establish semantic-only motion ownership")
            if not response["frames"]:
                raise RuntimeError("No generated continuation")
            for frame in response["frames"]:
                packets.write(json.dumps(frame, allow_nan=False) + "\n")
            packets.flush()
            request["pts_ms"] = response["frames"][-1]["pts_ms"]
            print(json.dumps({"generated_until_ms": request["pts_ms"]}), flush=True)


if __name__ == "__main__":
    main()
