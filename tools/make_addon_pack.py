"""Pack one folder of a converted install as a web add-on pack.

Add-on packs sit beside the main game-data pack (skate3-game-data.zip) and use
its layout, `skate3-web/data/installations/<id>/<path>`, so the web host and
`serve-web.js` find them by a marker file. Entries are stored, not deflated:
the browser reads single files out of the pack with HTTP range requests.

    python tools/make_addon_pack.py assets/private/frontend deploy/skate3-ui.zip
    python tools/make_addon_pack.py assets/private/audio deploy/skate3-audio.zip
    python tools/make_addon_pack.py assets/private/dropper deploy/skate3-dropper.zip
"""
from pathlib import Path
import argparse
import json
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def install_dir(data: Path) -> Path:
    record = json.loads((data / "installation.json").read_text(encoding="utf-8"))
    return data / record["directory"]


def pack(folder: str, out: Path, data: Path = ROOT / "data") -> int:
    install = install_dir(data)
    source = install / folder
    if not source.is_dir():
        raise SystemExit(f"{source} does not exist; run setup first")
    prefix = f"skate3-web/data/installations/{install.name}"
    files = sorted(p for p in source.rglob("*") if p.is_file())
    out.parent.mkdir(parents=True, exist_ok=True)
    temporary = out.with_suffix(".tmp")
    with zipfile.ZipFile(temporary, "w", zipfile.ZIP_STORED) as archive:
        for path in files:
            archive.write(path, f"{prefix}/{path.relative_to(install).as_posix()}")
    temporary.replace(out)
    return len(files)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("folder", help="folder inside the install, e.g. assets/private/frontend")
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    count = pack(args.folder, args.out.resolve())
    print(f"{args.out}: {count} files, {args.out.stat().st_size / 1e6:.1f} MB")
