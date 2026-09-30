"""Update every desktop release manifest when bumping the Cargo workspace."""

import json
import re
import sys
from pathlib import Path


if len(sys.argv) != 2 or not re.fullmatch(r"\d+\.\d+\.\d+", sys.argv[1]):
    raise SystemExit("usage: python3 scripts/set_versions.py MAJOR.MINOR.PATCH")

version = sys.argv[1]
root = Path(__file__).resolve().parent.parent
manifest = root / "Cargo.toml"
content = manifest.read_text(encoding="utf-8")
updated, count = re.subn(
    r'(?m)^(version = ")[0-9]+\.[0-9]+\.[0-9]+("$)',
    lambda match: f"{match[1]}{version}{match[2]}",
    content,
    count=1,
)
if count != 1:
    raise SystemExit("expected one Cargo workspace version")
manifest.write_text(updated, encoding="utf-8")

for name in ("src-tauri/tauri.conf.json", "frontend/package.json", "frontend/package-lock.json"):
    path = root / name
    data = json.loads(path.read_text(encoding="utf-8"))
    data["version"] = version
    if name.endswith("package-lock.json"):
        data["packages"][""]["version"] = version
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

print(f"Updated Cargo, Tauri, and npm release metadata to {version}")
