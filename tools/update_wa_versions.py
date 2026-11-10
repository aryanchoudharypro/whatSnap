#!/usr/bin/env python3
"""Bring the WhatsApp versions in backend/identity.go up to date.

The web revision comes from web.whatsapp.com's service worker and the Windows
build from the public Microsoft Store listing of WhatsApp for Windows. Numbers
only ever go up. Prints one line per change; prints nothing when up to date.
"""

import json
import pathlib
import re
import sys
import urllib.request

IDENTITY = pathlib.Path(__file__).resolve().parent.parent / "backend" / "identity.go"
SERVICE_WORKER = "https://web.whatsapp.com/sw.js"
STORE_LISTING = (
    "https://displaycatalog.mp.microsoft.com/v7.0/products/9NKSQGP7F2NH"
    "?market=US&languages=en-us"
)


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "whatSnap-version-check"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode("utf-8", "replace")


def web_revision(service_worker):
    found = re.findall(r'client_revision\\?":(\d+)', service_worker)
    if not found:
        raise ValueError("no client_revision in the service worker")
    return max(int(revision) for revision in found)


def windows_build(listing):
    """2.2641.101.0 is sent to the server as 264101: minor * 100 + build % 100."""
    builds = []
    for sku in json.loads(listing)["Product"]["DisplaySkuAvailabilities"]:
        for package in sku["Sku"]["Properties"]["Packages"]:
            match = re.search(r"WhatsAppDesktop_(\d+)\.(\d+)\.(\d+)\.\d+_", package.get("PackageFullName", ""))
            if match:
                builds.append(int(match.group(2)) * 100 + int(match.group(3)) % 100)
    if not builds:
        raise ValueError("no WhatsAppDesktop package in the Store listing")
    return max(builds)


def bump(source, name, latest, low, high):
    if not low <= latest <= high:
        raise ValueError(f"{name} {latest} is outside {low}..{high}")
    pattern = re.compile(rf"(const {name} = )(\d+)")
    match = pattern.search(source)
    if not match:
        raise ValueError(f"{name} not found in {IDENTITY.name}")
    current = int(match.group(2))
    if latest <= current:
        return source
    print(f"{name} {current} -> {latest}")
    return pattern.sub(rf"\g<1>{latest}", source, count=1)


def main():
    source = IDENTITY.read_text(encoding="utf-8")
    updated = bump(source, "webClientRevision", web_revision(fetch(SERVICE_WORKER)), 1_000_000_000, 4_000_000_000)
    updated = bump(updated, "windowsAppBuild", windows_build(fetch(STORE_LISTING)), 100_000, 999_999)
    if updated != source:
        IDENTITY.write_text(updated, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        sys.exit(f"update_wa_versions: {error}")
