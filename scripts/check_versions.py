"""Fail CI when the Cargo workspace, desktop bundle, and npm metadata diverge."""

import json
import tomllib
from pathlib import Path


root = Path(__file__).resolve().parent.parent
workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
desktop = tomllib.loads((root / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
tauri = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
package = json.loads((root / "frontend/package.json").read_text(encoding="utf-8"))
lock = json.loads((root / "frontend/package-lock.json").read_text(encoding="utf-8"))
cargo_lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))

version = workspace["workspace"]["package"]["version"]
if workspace["package"]["version"] != {"workspace": True}:
    raise SystemExit("CLI package must inherit the workspace version")
if desktop["package"]["version"] != {"workspace": True}:
    raise SystemExit("Desktop package must inherit the workspace version")
versions = [
    ("Tauri", tauri["version"]),
    ("frontend", package["version"]),
    ("frontend lockfile", lock["version"]),
    ("frontend lockfile root", lock["packages"][""]["version"]),
]
for name in ("ai-agent", "via-agent"):
    locked = [package["version"] for package in cargo_lock["package"] if package["name"] == name]
    if len(locked) != 1:
        raise SystemExit(f"expected exactly one Cargo.lock entry for {name}")
    versions.append((f"Cargo.lock {name}", locked[0]))

for name, actual in versions:
    if actual != version:
        raise SystemExit(f"{name} version {actual} does not match Cargo {version}")

print(f"CLI, Desktop bundle, and frontend versions agree: {version}")
