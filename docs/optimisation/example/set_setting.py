"""Set optimise settings that have no command-line flag, in a design file.

    python3 set_setting.py design.owd terminal_min_level_m=6.5 [name=value ...]

A design file is JSON. The optimise stage reads `stages.optimise.settings` and lays the flags of
`owd optimise` over it. A name the stage does not know is refused when the stage runs.
"""
import json
import sys


def main(argv):
    path, pairs = argv[1], argv[2:]
    with open(path, encoding="utf-8") as f:
        doc = json.load(f)
    stage = doc["stages"].setdefault("optimise", {})
    settings = stage.get("settings") or {}
    for pair in pairs:
        name, _, value = pair.partition("=")
        settings[name] = json.loads(value)
    stage["settings"] = settings
    with open(path, "w", encoding="utf-8") as f:
        json.dump(doc, f)


if __name__ == "__main__":
    main(sys.argv)
