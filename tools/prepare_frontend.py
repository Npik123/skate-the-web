"""Export an original front-end screen and every control it imports as one
runtime movie for the engine's APT player.

Like prepare_hud.py this reads the user's own extracted game, never launches
it, and writes copyrighted output that stays private to the installation.

APT bundles are separate movies that link through named imports and exports.
The engine's player runs one movie, so every bundle reached from the screen is
merged: each gets its own character-id range (bundle index * ID_STRIDE + local
id), imports are resolved to the exporting bundle's character, and action
blocks are keyed by (bundle index * ACTION_STRIDE + offset) so offsets from
different .apt files cannot collide. Linkage names stay global, which is how
attachMovie finds them in the shipped player as well.
"""
from pathlib import Path
import argparse
import hashlib
import json
import shutil

from vendor.skate3_ui.project import extract_project
from vendor.skate3_ui.scene_graph import AssetCache, SceneFlattener
from vendor.skate3_ui.actions import Actions
from prepare_hud import font_mapping

ID_STRIDE = 100_000
ACTION_STRIDE = 1 << 24
SCREENS = (
    "data/fe/source/screens/main/core_menu",
    # The game modes' HUD (mode_hud.rs).
    "data/fe/source/screens/hud2/hudobjective2",
    "data/fe/source/screens/hud2/leaderboardhud",
    "data/fe/source/screens/hud2/hudtimer",
    "data/fe/source/screens/hud2/homscoring",
    "data/fe/source/screens/hud2/hudabd",
    "data/fe/source/screens/hud2/hudthrowmes",
    "data/fe/source/screens/hud2/dpo",
    "data/fe/source/screens/hud2/hudcentral",
    "data/fe/source/screens/hud2/hudcountdown",
    "data/fe/source/screens/challenge/killedit",
    "data/fe/source/screens/challenge/ownthespot",
    "data/fe/source/screens/hudComponents/endchallenge_online",
    # The front end (main_menu.rs): title prompt, popup, loading screen.
    "data/fe/source/screens/hud2/actionoverlay2",
    "data/fe/source/screens/popup/sk8popup",
    "data/fe/source/screens/main/tiploader",
    # Create-a-Skater (cas_screen.rs).
    "data/fe/source/screens/cas/createskater",
    # The Trick Guide (trick_guide.rs).
    "data/fe/source/screens/tricks/trickguide",
    # The Object Dropper (object_dropper.rs).
    "data/fe/source/screens/objectdropper/objectdropper",
)


def bundle_name(import_file: str) -> str:
    return "data/fe/" + import_file.replace("\\", "/")


def collect(cache: AssetCache, root: str) -> list[str]:
    order, pending = [], [root]
    while pending:
        name = pending.pop(0)
        if name in order:
            continue
        order.append(name)
        for entry in cache.load_bundle(name)["imports"].values():
            pending.append(bundle_name(entry["file"]))
    return order


def export(cache: AssetCache, raw: Path, root: str, output: Path, mappings: dict) -> dict:
    names = collect(cache, root)
    index = {name: i for i, name in enumerate(names)}
    bundles = {name: cache.load_bundle(name) for name in names}

    def global_id(name: str, local: int) -> int:
        bundle = bundles[name]
        entry = bundle["imports"].get(local) or bundle["imports"].get(str(local))
        if entry:
            target = bundle_name(entry["file"])
            exports = bundles[target]["exports"]
            if entry["name"] not in exports:
                raise ValueError(f"{name}: import {entry['name']!r} not exported by {target}")
            return global_id(target, int(exports[entry["name"]]))
        return index[name] * ID_STRIDE + int(local)

    characters, shapes, fonts, actions, linkage, init_actions = [], {}, {}, {}, {}, []
    dropped_clip_actions = []
    shadowed_linkage = []
    textures = {}
    for name in names:
        bundle = bundles[name]
        k = index[name]
        apt = raw / (name + ".apt")
        decoder = Actions(apt.read_bytes(), apt.with_suffix(".const").read_bytes())
        for export_name, local in bundle["exports"].items():
            if export_name in linkage and linkage[export_name] != global_id(name, int(local)):
                # Shared controls (button_item2, cas_slider...) each ship a copy
                # of small parts like "Notch". Bundles load screen first, then
                # imports in order, and the first registration stays.
                shadowed_linkage.append(f"{name}:{export_name}")
                continue
            linkage[export_name] = global_id(name, int(local))
        for character in bundle["characters"].values():
            c = json.loads(json.dumps(character))
            c["id"] = global_id(name, character["id"])
            for frame in c.get("frames", []):
                for control in frame["controls"]:
                    if control["type_name"].startswith("place_object"):
                        if control.get("character_id") is not None:
                            control["character_id"] = global_id(name, control["character_id"])
                        if control.get("flags", 0) & 0x80:
                            # Clip events are mouse handlers on a few shipped
                            # buttons (the DLC banner); the pad drives this menu.
                            control["flags"] &= ~0x80
                            control["actions_offset"] = 0
                            dropped_clip_actions.append(f"{name}:{character['id']}:{control.get('name', '')}")
                        continue
                    offset = control.get("actions_offset", 0)
                    if offset and control["type_name"] in ("do_action", "do_init_action"):
                        key = k * ACTION_STRIDE + offset
                        control["actions_offset"] = key
                        actions[str(key)] = decoder.stream(offset)
                        if control["type_name"] == "do_init_action":
                            init_actions.append({"sprite": global_id(name, control["sprite_id"]), "actions": key})
            if c.get("text"):
                c["text"]["font_id"] = global_id(name, c["text"]["font_id"])
            if c["type_name"] == "shape":
                scene = SceneFlattener(cache, lambda *_: {}).flatten(name, character["id"])
                if scene["unresolved"]:
                    raise ValueError(f"Unresolved original shape {name}:{character['id']}: {scene['unresolved']}")
                for primitive in scene["primitives"]:
                    if primitive["texture"]:
                        textures[primitive["texture"]["rgba"]] = primitive["texture"]
                shapes[str(c["id"])] = scene["primitives"]
            elif c["type_name"] == "font":
                family = c["font"]["name"]
                fonts[family] = cache.font_asset(family)
            characters.append(c)

    # FontManager draws Futura Shadow under a futuraheavy foreground it
    # attaches natively, whether or not the screen imports that font itself.
    heavy = next((apt for apt, m in mappings.items() if m["file_name"] == "futuraheavy"), None)
    if "Futura Shadow" in fonts and heavy and heavy not in fonts:
        fonts[heavy] = cache.font_asset(heavy)
        characters.append({"id": len(names) * ID_STRIDE, "type": 3, "type_name": "font",
                           "font": {"name": heavy, "glyph_character_ids": []}})

    result = {
        "format": "skate3-frontend-screen", "version": 1,
        "screen": root,
        "source": {name: {
            "apt_sha256": hashlib.sha256((raw / (name + ".apt")).read_bytes()).hexdigest(),
            "const_sha256": hashlib.sha256((raw / (name + ".const")).read_bytes()).hexdigest(),
        } for name in names},
        "bundles": names,
        "root": global_id(root, 0),
        "characters": characters,
        "shapes": shapes,
        "fonts": fonts,
        "font_mappings": mappings,
        "actions": actions,
        "linkage": linkage,
        "init_actions": init_actions,
        "dropped_clip_actions": dropped_clip_actions,
        "shadowed_linkage": shadowed_linkage,
        "unresolved_fonts": sorted(f for f, asset in fonts.items() if asset is None),
    }
    return result


# Controller glyphs the image renderer drew natively from the shipped web
# pages' art (miscboot.big), by the names screens pass (button_item2 "B").
BUTTONS = {"A": "A", "B": "B", "X": "X", "Y": "Y", "LB": "L_Bumper", "RB": "R_Bumper",
           "LT": "L_Trigger", "RT": "R_Trigger", "START": "Start", "BACK": "Back",
           "DPAD": "DPad", "LS": "Stick_Left", "RS": "Stick_Right"}


def export_buttons(game: Path, target: Path) -> dict:
    from io import BytesIO
    from PIL import Image
    from vendor.skate3_ui.big import BigArchive
    from vendor.skate3_ui.project import find_big_directory
    archive = BigArchive(find_big_directory(game) / "miscboot.big")
    files = {e.path.rsplit("/", 1)[-1]: e for e in archive.entries if "/Buttons/XBL2/" in e.path}
    glyphs = {}
    (target / "buttons").mkdir(parents=True, exist_ok=True)
    for key, stem in BUTTONS.items():
        entry = files.get(f"button_{stem}.png")
        if entry is None:
            continue
        image = Image.open(BytesIO(archive.read(entry))).convert("RGBA")
        relative = f"buttons/{key}.rgba"
        (target / relative).write_bytes(image.tobytes())
        glyphs[key] = {"texture": relative, "width": image.width, "height": image.height}
    return glyphs


def prepare(game: Path, work: Path, collections: Path, assets: Path):
    """Extract into `work`, export runtime screens into assets/private/frontend."""
    prefixes = {screen.rsplit("/", 1)[0] + "/" for screen in SCREENS}
    extract_project(game, work, prefixes=(*sorted(prefixes), "data/fe/source/controls/", "data/fe/fonts/"), update=True)
    cache = AssetCache(work)
    mappings = font_mapping(collections, cache)
    # The Trick Guide's text names HelveticaPresto, which the game's own font
    # table (class FECFBCAF356518C4) does not list: the shipped player falls
    # back for it. Fall back to the screens' standard face, said so here.
    for missing, fallback in (("HelveticaPresto", "Futura Std Medium"),):
        if missing not in mappings:
            # Same face and native layout as the font it stands in for.
            provenance = dict(mappings[fallback], fallback=True, reason="not in the game's font table")
            cache.add_font_alias(missing, cache.font_aliases[fallback]["target"], provenance)
            mappings[missing] = provenance
    language = {row["label"].strip(): row["value"] for row in
                json.loads((work / "metadata/languages/english_global.json").read_text(encoding="utf-8"))["entries"]}
    target = assets / "private" / "frontend"
    target.mkdir(parents=True, exist_ok=True)
    copied = set()
    for screen in SCREENS:
        result = export(cache, work / "raw", screen, target, mappings)
        # The string table is shared (language.json); the pause menu keeps its
        # own copy for engines that predate the shared file.
        if screen == SCREENS[0]:
            result["language"] = language
        if result["unresolved_fonts"]:
            raise ValueError("Unresolved original front-end fonts: " + ", ".join(result["unresolved_fonts"]))
        # Runtime files are referenced relative to the screen manifest.
        for shape in result["shapes"].values():
            for primitive in shape:
                if primitive["texture"]:
                    copied.add(primitive["texture"]["rgba"])
        for asset in result["fonts"].values():
            copied.add(asset["texture"])
        out = target / (Path(screen).name + ".json")
        out.write_text(json.dumps(result, separators=(",", ":")) + "\n", encoding="utf-8")
        print(f"{Path(screen).name}: {len(result['bundles'])} bundles, {len(result['characters'])} characters, "
              f"{len(result['shapes'])} shapes, {len(result['actions'])} action blocks, {len(result['linkage'])} linkage names")
    (target / "buttons.json").write_text(json.dumps(export_buttons(game, target)) + "\n", encoding="utf-8")
    (target / "language.json").write_text(json.dumps(language, separators=(",", ":")) + "\n", encoding="utf-8")
    for relative in sorted(copied):
        source = work / relative
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    print(f"Original front end ready: {len(copied)} textures in {target}")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--game", type=Path, required=True)
    p.add_argument("--work", type=Path, required=True, help="private extraction workspace")
    p.add_argument("--assets", type=Path, required=True, help="installation assets/ directory")
    args = p.parse_args()
    try:
        prepare(args.game.resolve(), args.work.resolve(),
                args.assets.resolve() / "private/stock/skater-collections.json", args.assets.resolve())
    except Exception as error:  # noqa: BLE001 - optional content
        # The pause menu falls back to the engine's own panel without these
        # files, so a damaged or unexpected disc must not fail the install.
        shutil.rmtree(args.assets.resolve() / "private" / "frontend", ignore_errors=True)
        print(f"Original pause menu unavailable: {error}", flush=True)
