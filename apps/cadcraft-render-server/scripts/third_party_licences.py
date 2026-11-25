#!/usr/bin/env python3
"""THIRD-PARTY-LICENSES.txt for the render server: every registry and git crate built into it
(normal dependencies, any platform), its declared licence and the licence files it ships.
Fails when a crate declares no licence and ships no licence file.

usage: third_party_licences.py OUT [PACKAGE]
"""

import json
import os
import subprocess
import sys

NAMES = ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE", "AUTHORS")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def licence_files(manifest_dir, root_limit):
    """Licence files beside the crate, else in the folders above it up to its checkout."""
    d = manifest_dir
    for _ in range(4):
        found = sorted(
            os.path.join(d, f)
            for f in os.listdir(d)
            if os.path.isfile(os.path.join(d, f)) and f.upper().startswith(NAMES)
        )
        if found:
            return found
        parent = os.path.dirname(d)
        if parent == d or not parent.startswith(root_limit) or os.path.isdir(os.path.join(d, ".git")):
            break
        d = parent
    return []


def main():
    out = sys.argv[1]
    package = sys.argv[2] if len(sys.argv) > 2 else "cadcraft-render-server"
    tree = run("cargo", "tree", "-p", package, "-e", "normal", "--target", "all", "--prefix", "none", "-f", "{p}")
    wanted = set()
    for line in tree.splitlines():
        line = line.replace(" (*)", "").replace(" (proc-macro)", "").strip()
        parts = line.split(" ")
        if len(parts) < 2:
            continue
        name, version = parts[0], parts[1].lstrip("v")
        source = " ".join(parts[2:])
        # Path crates (this workspace) are CADCraft's and this program's own: LICENSE-MIT, LICENSE-APACHE.
        if source.startswith("(/") or (len(source) > 2 and source[2] == ":"):
            continue
        wanted.add((name, version))
    meta = json.loads(run("cargo", "metadata", "--format-version", "1"))
    home = os.environ.get("CARGO_HOME", os.path.join(os.path.expanduser("~"), ".cargo"))
    entries, missing = [], []
    for p in sorted(meta["packages"], key=lambda p: (p["name"], p["version"])):
        if (p["name"], p["version"]) not in wanted or p.get("source") is None:
            continue
        d = os.path.dirname(p["manifest_path"])
        files = licence_files(d, os.path.abspath(home))
        lic = p.get("license") or ("see " + p["license_file"] if p.get("license_file") else None)
        if not lic and not files:
            missing.append(f"{p['name']} {p['version']}")
        entries.append((p, lic, files))
    if missing:
        print("no licence declared or shipped:", ", ".join(missing))
        sys.exit(1)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("Third-party software built into cadcraft-render-server\n")
        f.write("=" * 56 + "\n\n")
        f.write(f"{len(entries)} crates (normal dependencies, every platform), each with its declared licence\n")
        f.write("and the licence files it ships.\n\n")
        for p, lic, _ in entries:
            f.write(f"{p['name']} {p['version']}  ({lic or 'licence file only'})  {p.get('repository') or ''}\n")
        for p, lic, files in entries:
            f.write("\n\n" + "-" * 78 + "\n")
            f.write(f"{p['name']} {p['version']}  ({lic or 'licence file only'})\n")
            f.write("-" * 78 + "\n")
            seen = set()
            for path in files:
                with open(path, encoding="utf-8", errors="replace") as lf:
                    text = lf.read().strip()
                if text in seen:
                    continue
                seen.add(text)
                f.write(f"\n[{os.path.basename(path)}]\n\n{text}\n")
    print(f"{len(entries)} third-party crates written to {out}")


if __name__ == "__main__":
    main()
