#!/usr/bin/env python3
"""Run simultaneous 1.17 Eagler and 26.3 Eagler/Gaius/vanilla probes.

Requires the running gateway -> Multiversion -> offline Pumpkin test route.
Use --sustain for timed sessions and --reconnect to repeat login afterwards.
These simulated clients do not establish graphical acceptance.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8081)
    parser.add_argument("--seconds", type=int, default=30)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--sustain", action="store_true")
    parser.add_argument("--reconnect", action="store_true",
                        help="repeat smoke checks with the same identities after all sessions close")
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error("--seconds must be positive")
    args.output_dir.mkdir(parents=True, exist_ok=False)
    scripts = Path(__file__).resolve().parent
    cases = [
        ("eagler-755", "probe_1_17.py", ["--name", "MatrixOld"]),
        ("eagler-777", "probe_26_3.py", ["--transport", "eagler", "--name", "MatrixEagler"]),
        ("gaius-777", "probe_26_3.py", ["--transport", "gaius", "--name", "MatrixGaius"]),
        ("vanilla-777", "probe_26_3.py", ["--transport", "vanilla", "--name", "MatrixVanilla"]),
    ]

    def run(case, reconnect=False):
        name, script, extra = case
        if reconnect:
            name += "-reconnect"
        seconds = 30 if reconnect else args.seconds
        result_path = args.output_dir / f"{name}.json"
        command = [sys.executable, str(scripts / script), "--host", args.host,
                   "--port", str(args.port), "--seconds", str(seconds),
                   "--json-output", str(result_path), *extra]
        if args.sustain and not reconnect:
            command.append("--sustain")
        try:
            completed = subprocess.run(command, text=True, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, timeout=seconds + 15)
            output, code = completed.stdout, completed.returncode
        except subprocess.TimeoutExpired as error:
            output = (error.stdout or b"")
            if isinstance(output, bytes):
                output = output.decode(errors="replace")
            output += "\nProbe process exceeded its time budget.\n"
            code = 124
        (args.output_dir / f"{name}.log").write_text(output)
        return dict(case=name, passed=code == 0 and result_path.exists(), exit_code=code)

    started = datetime.now(timezone.utc).isoformat()
    with ThreadPoolExecutor(max_workers=len(cases)) as pool:
        results = list(pool.map(run, cases))
        if args.reconnect:
            results.extend(pool.map(lambda case: run(case, reconnect=True), cases))
    summary = dict(started_utc=started, finished_utc=datetime.now(timezone.utc).isoformat(),
                   host=args.host, port=args.port, scope="simulated mixed-client sustained" if args.sustain else "simulated mixed-client smoke",
                   requested_seconds=args.seconds, reconnect=args.reconnect,
                   cases=results)
    (args.output_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    return 0 if all(result["passed"] for result in results) else 1


if __name__ == "__main__":
    sys.exit(main())
