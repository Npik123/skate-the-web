"""Convert a locally owned Skate 3 disc into engine assets.

Accepts your own Xbox 360 ISO, or the default.xex inside an extracted game
folder. Nothing is downloaded except the ISO extractor tool; no game content
leaves this machine.

Writes a fresh installation under <base>/installations/<id>/ containing
`assets/`, `maps/` and `settings/`. That folder is what the web client imports.
"""
from pathlib import Path
import argparse, sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.asset_pipeline.install import install


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path, help='Skate 3 .iso, or default.xex in an extracted game folder')
    parser.add_argument('--base', type=Path, default=Path('data'), help='Installation root (default: data)')
    parser.add_argument('--game-exe', type=Path, required=True, help='Built skate3rust executable used to validate each converted map')
    parser.add_argument('--refresh', action='store_true', help='Reuse the previous installation and rebuild only changed groups')
    args = parser.parse_args()
    source = args.source
    if not source.exists():
        parser.error(f'Not found: {source}')
    if source.is_file() and source.suffix.lower() not in {'.iso', '.xex'}:
        parser.error('Select a .iso, or default.xex inside an extracted game folder')
    root = install(source, args.base, args.game_exe.resolve(), lambda text: print(text, flush=True), refresh=args.refresh)
    print(f'\nINSTALLED {root}')
    print(f'Import this folder in the web client: {root}')


if __name__ == '__main__':
    main()
