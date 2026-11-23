"""Package an already-built release binary; run on all supported CI runners."""
import hashlib
import pathlib
import sys
import tarfile
import zipfile


def main():
    target = sys.argv[1]
    windows = "windows" in target
    executable = "rollback-risk-lint" + (".exe" if windows else "")
    binary = pathlib.Path("target") / target / "release" / executable
    if not binary.is_file():
        raise SystemExit(f"Missing release binary: {binary}")
    output = pathlib.Path("dist")
    output.mkdir(exist_ok=True)
    stem = f"rollback-risk-lint-{target}"
    files = [(binary, executable), (pathlib.Path("README.md"), "README.md"),
             (pathlib.Path("LICENSE"), "LICENSE")]
    archive = output / (stem + (".zip" if windows else ".tar.gz"))
    if windows:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as package:
            for path, name in files:
                package.write(path, f"{stem}/{name}")
    else:
        with tarfile.open(archive, "w:gz") as package:
            for path, name in files:
                package.add(path, arcname=f"{stem}/{name}")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="utf-8"
    )
    print(archive)


if __name__ == "__main__":
    main()
