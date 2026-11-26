"""Install the generated UI app on a private simulator and inspect a presented frame."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import time
import uuid


def sim(*args, **kwargs):
    return subprocess.check_output(["xcrun", "simctl", *args], text=True, **kwargs).strip()


@contextmanager
def installed_app(bundle, match_xcode=False):
    devices = json.loads(sim("list", "devices", "available", "-j"))["devices"]
    candidates = [(runtime, device["deviceTypeIdentifier"]) for runtime, items in devices.items()
                  if ".iOS-" in runtime for device in items if device.get("isAvailable") and
                  "iPhone" in device.get("deviceTypeIdentifier", "")]
    if match_xcode:
        version = subprocess.check_output(["xcrun", "--sdk", "iphonesimulator", "--show-sdk-version"], text=True)
        sdk = tuple(map(int, version.strip().split(".")[:2]))
        candidates = [(r, d) for r, d in candidates if tuple(map(int, r.split(".iOS-")[1].split("-")[:2])) <= sdk]
    if not candidates:
        raise RuntimeError("an installed iPhone simulator runtime is required")
    runtime, model = max(candidates, key=lambda p: tuple(map(int, p[0].split(".iOS-")[1].split("-"))))
    udid = sim("create", "Hyve-" + uuid.uuid4().hex[:8], model, runtime)
    try:
        sim("bootstatus", udid, "-b")
        sim("install", udid, str(bundle))
        package = subprocess.check_output(["/usr/bin/plutil", "-extract", "CFBundleIdentifier", "raw",
                                           "-o", "-", str(bundle / "Info.plist")], text=True).strip()
        data = Path(sim("get_app_container", udid, package, "data"))
        yield udid, package, data, runtime
    finally:
        subprocess.run(["xcrun", "simctl", "shutdown", udid], capture_output=True)
        sim("delete", udid)


def verify(bundle):
    with installed_app(bundle) as (udid, package, data, runtime):
        frame = data / "tmp/hyve-test.ppm"
        sim("launch", udid, package, env=dict(os.environ, SIMCTL_CHILD_HYPERISH_RENDER_BACKEND="cpu",
                                              SIMCTL_CHILD_SERGE_DEBUG_FRAME=str(frame)))
        until = time.monotonic() + 45
        while time.monotonic() < until:
            if frame.exists():
                parts = frame.read_bytes().split(b"\n", 3)
                if len(parts) == 4 and parts[0] == b"P6" and parts[2] == b"255":
                    width, height = map(int, parts[1].split())
                    if len(parts[3]) == width * height * 3 and min(width, height) > 200:
                        pixels = memoryview(parts[3])
                        if len({bytes(pixels[i:i+3]) for i in range(0, len(pixels), 3 * 17)}) > 8:
                            screenshot = data / "tmp/hyve-screen.png"
                            sim("io", udid, "screenshot", str(screenshot))
                            with screenshot.open("rb") as png:
                                header = png.read(24)
                            assert header[:8] == b"\x89PNG\r\n\x1a\n"
                            screen = struct.unpack(">II", header[16:24])
                            assert (width, height) == screen, f"application frame {width}x{height}, simulator {screen}"
                            print(f"Serge presented {width}x{height} pixels on {runtime}")
                            return
            time.sleep(0.2)
        raise RuntimeError("the launched app did not present the counter UI")


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve())
