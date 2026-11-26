"""Install the WiX and NSIS outputs of hyve build and run each installed executable."""
from pathlib import Path
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import uuid


def exercise(hyve):
    with tempfile.TemporaryDirectory(prefix="hyve-installers-") as temp:
        root = Path(temp)
        app = root / "app"
        (app / "src").mkdir(parents=True)
        name = "hyve-fixture-" + uuid.uuid4().hex[:10]
        (app / "Cargo.toml").write_text(f'''[package]
name="{name}"
version="1.0.0"
edition="2024"
description="Hyve local packaging acceptance"
authors=["Hyperish"]
[package.metadata.packager]
productName="{name}"
identifier="org.hyperish.{name}"
binaries=[{{path="{name}",main=true}}]
''')
        thumbprint = os.environ.get("HYVE_TEST_WINDOWS_SIGNING_CERT")
        if thumbprint:
            assert re.fullmatch(r"[a-fA-F0-9]{40}", thumbprint)
            with (app / "Cargo.toml").open("a") as manifest:
                manifest.write(f'[package.metadata.packager.windows]\ncertificateThumbprint="{thumbprint}"\n'
                               'digestAlgorithm="sha256"\n')
        (app / "src/main.rs").write_text('fn main() { println!("installed hyve application"); }')
        env = dict(os.environ, CARGO_TARGET_DIR=str(app / "target"))
        subprocess.run([hyve, "build", "--debug", "--format", "wix,nsis"], cwd=app, env=env, check=True)
        outputs = app / "target/debug"
        msi, = outputs.glob("*.msi")
        installers = [p for p in outputs.glob("*.exe") if p.stem != name]
        nsis, = installers

        signtool = shutil.which("signtool") if thumbprint else None
        if thumbprint and not signtool:
            kits = Path(os.environ["ProgramFiles(x86)"]) / "Windows Kits/10/bin"
            arch = {"AMD64": "x64", "ARM64": "arm64"}.get(os.environ["PROCESSOR_ARCHITECTURE"], "x86")
            signtool = max(kits.glob(f"10.*/{arch}/signtool.exe"),
                           key=lambda p: tuple(map(int, p.parents[1].name.split("."))))

        def signature(path):
            return subprocess.run([signtool, "verify", "/pa", "/v", str(path)], capture_output=True,
                                  text=True, errors="replace")

        def verify_signer(path):
            if thumbprint:
                check = signature(path)
                assert check.returncode == 0, check.stdout + check.stderr
                assert thumbprint.upper() in check.stdout.upper(), check.stdout

        for package in [msi, nsis]:
            verify_signer(package)

        def verify(dest):
            installed, = dest.rglob(name + ".exe")
            verify_signer(installed)
            assert subprocess.check_output([installed], text=True).strip() == "installed hyve application"
            if thumbprint:
                modified = bytearray(installed.read_bytes())
                pe, = struct.unpack_from("<I", modified, 0x3c)
                optional_size, = struct.unpack_from("<H", modified, pe + 20)
                section_offset, = struct.unpack_from("<I", modified, pe + 24 + optional_size + 20)
                modified[section_offset] ^= 1
                copy = root / "modified.exe"
                copy.write_bytes(modified)
                assert signature(copy).returncode != 0, "Windows accepted a modified signed executable"

        dest = root / "msi"
        subprocess.run(["msiexec", "/i", msi, "/qn", "/norestart", f"INSTALLDIR={dest}"], check=True)
        try:
            verify(dest)
        finally:
            subprocess.run(["msiexec", "/x", msi, "/qn", "/norestart"], check=True)

        dest = root / "nsis"
        # NSIS consumes the unquoted remainder of the command line as its /D parameter.
        command = subprocess.list2cmdline([str(nsis), "/S"]) + " /D=" + str(dest)
        subprocess.run(command, check=True, timeout=120)
        try:
            verify(dest)
        finally:
            uninstaller, = dest.rglob("uninstall.exe")
            command = subprocess.list2cmdline([str(uninstaller), "/S"]) + " _?=" + str(dest)
            subprocess.run(command, check=True, timeout=120)
        print("Both installed Windows applications ran successfully.")
        if thumbprint:
            print("Windows verified both installers, both installed signatures and rejected altered copies.")


if __name__ == "__main__":
    exercise(str(Path(sys.argv[1]).resolve()))
