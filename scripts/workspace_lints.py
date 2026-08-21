#!/usr/bin/env python3
"""Workspace dependency lints, fed by `cargo metadata` on stdin.

T-103 (ADR-004, local-only): HTTP client crates are banned as direct
dependencies everywhere except fndr-downloader and fndr-updater. If this
fires, the answer is a design change (route the download through those
crates), not an exemption. cargo-deny covers the transitive graph.

T-104 (ADR-001, shell-agnostic engine): only fndr-shell may depend on
Tauri, and no engine crate may depend on fndr-shell.
"""

import json
import sys

HTTP_CLIENTS = {
    "reqwest",
    "ureq",
    "curl",
    "curl-sys",
    "isahc",
    "attohttpc",
    "surf",
}
EGRESS_ALLOWED = {"fndr-downloader", "fndr-updater"}
SHELL = "fndr-shell"


def main() -> int:
    meta = json.load(sys.stdin)
    members = set(meta["workspace_members"])
    failures = []

    for pkg in meta["packages"]:
        if pkg["id"] not in members:
            continue
        for dep in pkg["dependencies"]:
            name = dep["name"]
            if name in HTTP_CLIENTS and pkg["name"] not in EGRESS_ALLOWED:
                failures.append(
                    f"{pkg['name']} depends on HTTP client '{name}'. "
                    f"Only {sorted(EGRESS_ALLOWED)} may have egress (ADR-004)."
                )
            if name.startswith("tauri") and pkg["name"] != SHELL:
                failures.append(
                    f"{pkg['name']} depends on '{name}'. "
                    f"Only {SHELL} may import Tauri (ADR-001)."
                )
            if name == SHELL:
                failures.append(
                    f"{pkg['name']} depends on {SHELL}. The engine must not "
                    "depend on the shell (ADR-001)."
                )

    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    if not failures:
        print("workspace lints: ok")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
