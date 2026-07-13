"""Launch and verify the real R-103 editor process workflow on Windows."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path

import blake3


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    binary = args.binary or root / "target" / "debug" / "hyge-tools.exe"
    evidence = args.evidence_dir or root / "target" / "r103-evidence"
    project = root / "target" / "r103-project"
    fixture = root / "crates" / "hyge-editor" / "tests" / "fixtures" / "r103-editor-project"
    shutil.rmtree(project, ignore_errors=True)
    shutil.rmtree(evidence, ignore_errors=True)
    shutil.copytree(fixture, project)
    subprocess.run(
        [
            str(binary), "import", str(project / "assets" / "source" / "triangle.gltf"),
            "--out", str(project / "assets" / "cook"),
        ],
        cwd=root,
        check=True,
        timeout=30,
    )
    shutil.copy2(project / "assets" / "cook" / ".hyge.db", project / ".hyge.db")
    environment = os.environ.copy()
    environment.update({"QT_QPA_PLATFORM": "offscreen", "QT_QUICK_BACKEND": "software"})
    subprocess.run(
        [
            str(binary),
            "editor",
            str(project),
            "--port",
            "0",
            "--scene",
            "main.hyge-world",
            "--external-scene",
            "external.hyge-world",
            "--frontend",
            str(root / "tools" / "hyge-editor-python" / "main.py"),
            "--evidence-dir",
            str(evidence),
        ],
        cwd=root,
        env=environment,
        check=True,
        timeout=30,
    )
    workflow = json.loads((evidence / "workflow.json").read_text(encoding="utf-8"))
    manifest = json.loads((evidence / "manifest.json").read_text(encoding="utf-8"))
    manifest.setdefault("hashes", {})["protocol.jsonl"] = blake3.blake3(
        (evidence / "protocol.jsonl").read_bytes()
    ).hexdigest()
    (evidence / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    if not workflow.get("success") or not all(
        manifest.get(key) for key in ("editor_png", "viewport_png", "saved_scene", "workflow_success")
    ):
        raise RuntimeError(f"R-103 evidence is incomplete: {workflow=} {manifest=}")
    for name in ("editor.png", "viewport.png"):
        if not (evidence / name).read_bytes().startswith(b"\x89PNG\r\n\x1a\n"):
            raise RuntimeError(f"{name} is not a PNG")
    trace = [json.loads(line) for line in
             (evidence / "protocol.jsonl").read_text(encoding="utf-8").splitlines()]
    trace_types = {record["message_type"] for record in trace}
    errors = [record for record in trace if record.get("error")]
    if errors:
        raise RuntimeError(f"protocol trace contains structured errors: {errors}")
    outgoing_positions = {
        record["message_id"]: index
        for index, record in enumerate(trace)
        if record["direction"] == "out"
    }
    for index, record in enumerate(trace):
        correlation = record.get("correlation_id")
        if record["direction"] == "in" and correlation in outgoing_positions:
            if outgoing_positions[correlation] >= index:
                raise RuntimeError(f"protocol response precedes request: {record}")
    required = {
        "open_project", "open_scene", "select_entities", "edit_components", "undo", "redo",
        "save_scene", "scene_reloaded", "open_viewport_transport", "server_shutdown",
    }
    missing = sorted(required - trace_types)
    if missing:
        raise RuntimeError(f"protocol evidence is missing: {missing}")
    command_order = ["open_project", "open_scene", "select_entities", "edit_components", "undo", "redo", "save_scene", "server_shutdown"]
    positions = [next(index for index, record in enumerate(trace)
                      if record["direction"] == "out" and record["message_type"] == kind)
                 for kind in command_order]
    if positions != sorted(positions):
        raise RuntimeError(f"protocol command order is invalid: {positions}")
    if workflow.get("reload_translation") != [3.0, 1.0, 0.0]:
        raise RuntimeError(f"persistent Transform was not restored: {workflow}")
    if manifest.get("hash_algorithm") != "blake3":
        raise RuntimeError(f"unexpected evidence hash algorithm: {manifest}")
    for field in ("scene_revision", "camera_revision", "entity", "scene_node_id", "assertions"):
        if not manifest.get(field):
            raise RuntimeError(f"manifest is missing semantic field {field}: {manifest}")
    if not all(manifest["assertions"].values()):
        raise RuntimeError(f"manifest assertions failed: {manifest['assertions']}")
    for name, expected in manifest.get("hashes", {}).items():
        actual = blake3.blake3((evidence / name).read_bytes()).hexdigest()
        if actual != expected:
            raise RuntimeError(f"evidence hash mismatch for {name}: {actual} != {expected}")
    print(json.dumps({"status": "complete", "evidence": str(evidence), **manifest}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
