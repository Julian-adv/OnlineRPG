#!/usr/bin/env python3
"""Heroic-tale candidates (doc/HEROIC_TALES.md) from openmmo-server journal
lines on stdin. Prints ledger-shaped lines for a human to pick from and
append to agent-client/data/tales/ledger.txt — never appends itself.

Usage: journalctl -u openmmo-server --since "<KST>" -o cat | python3 tales.py [DATE]
DATE defaults to today (UTC). Optional: --min-break N (default 7).
Place names come from ~/work/OnlineRPG/data/map_labels.json (the prod checkout).
"""
import collections
import datetime
import json
import math
import os
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")
LEVEL = re.compile(r"Player (\S+) reached level (\d+)")
KILL = re.compile(r"Player (\S+) killed (\S+) \(lvl \d+\) at \((-?[\d.]+),(-?[\d.]+)\) (.*?);")
DIED = re.compile(r"Player (\S+) died to (\S+) at \((-?[\d.]+),(-?[\d.]+)\) (.*)")
ENCHANT = re.compile(r"(\S+) enchanted (\S+) to \+(\d+)")
BREAK = re.compile(r"(\S+) destroyed (\S+) enchanting at \+(\d+)")
TITLE = re.compile(r"Player (\S+) earned title '([^']+)'")
TITLE_OFFLINE = re.compile(r"Character (\d+) earned title '([^']+)'")

BOSSES = {"goblin_boss", "orc_boss", "ogre_boss"}
LABELS_PATH = os.path.expanduser("~/work/OnlineRPG/data/map_labels.json")
try:
    with open(LABELS_PATH) as f:
        LABELS = {k: (v["x"], v["z"]) for k, v in json.load(f).items() if v["kind"] != "continent"}
except OSError:
    LABELS = {}
ALDERMARK = LABELS.get("aldermark", (-1475.2, 4741.6))

args = [a for a in sys.argv[1:] if not a.startswith("--")]
date = args[0] if args else datetime.datetime.utcnow().strftime("%Y-%m-%d")
min_break = 7
if "--min-break" in sys.argv:
    min_break = int(sys.argv[sys.argv.index("--min-break") + 1])

levels = collections.Counter()
top_level = {}
boss_kills = []
boss_deaths = []
enchants = []
breaks = []
titles = []
offline_titles = []
farthest = {}

def note_pos(name, x, z):
    d = math.hypot(x - ALDERMARK[0], z - ALDERMARK[1])
    if d > farthest.get(name, (0, 0, 0))[0]:
        farthest[name] = (d, x, z)

for raw in sys.stdin:
    line = ANSI.sub("", raw).rstrip()
    m = LEVEL.search(line)
    if m:
        levels[m[1]] += 1
        top_level[m[1]] = max(top_level.get(m[1], 0), int(m[2]))
        continue
    m = KILL.search(line)
    if m:
        note_pos(m[1], float(m[3]), float(m[4]))
        if m[2] in BOSSES:
            boss_kills.append((m[1], m[2], m[5]))
        continue
    m = DIED.search(line)
    if m:
        note_pos(m[1], float(m[3]), float(m[4]))
        if m[2] in BOSSES:
            boss_deaths.append((m[1], m[2], m[5]))
        continue
    m = ENCHANT.search(line)
    if m:
        enchants.append((int(m[3]), m[1], m[2]))
        continue
    m = BREAK.search(line)
    if m:
        breaks.append((int(m[3]), m[1], m[2]))
        continue
    m = TITLE.search(line)
    if m:
        titles.append((m[1], m[2]))
        continue
    m = TITLE_OFFLINE.search(line)
    if m:
        offline_titles.append((m[1], m[2]))

def out(kind, name, *rest):
    print(f"{date}  {kind:<14} {name:<12} " + "  ".join(str(r) for r in rest))


def slug(place):
    """Ledger args are single tokens; the log's 'Ogre Den depth 3' becomes ogre_den."""
    return re.sub(r"\s+depth \d+$", "", place).strip().lower().replace(" ", "_")

print("# candidates — check solo/first against the ledger and DB before appending")
for name, boss, place in boss_kills:
    out("boss_kill", name, boss, slug(place), "solo=?", "first=?")
for name, boss, place in boss_deaths:
    out("boss_death", name, boss, slug(place))
for name, title in titles:
    out("title", name, title)
for cid, title in offline_titles:
    out("title", f"character#{cid}", title, "(offline grant: resolve the name in DB)")
if enchants:
    plus, name, item = max(enchants)
    out("enchant_up", name, item, f"+{plus}", "record=?  (window high; compare with DB max)")
for plus, name, item in sorted(breaks, reverse=True):
    if plus >= min_break:
        out("enchant_break", name, item, f"+{plus}")
if levels:
    name, gained = levels.most_common(1)[0]
    out("most_levels", name, f"+{gained}", f"reached={top_level[name]}")
    print("# level_record: compare", name, "at", top_level[name], "with the DB max level")
if farthest:
    name, (d, x, z) = max(farthest.items(), key=lambda kv: kv[1][0])
    near = min(LABELS, key=lambda k: math.hypot(x - LABELS[k][0], z - LABELS[k][1]), default="?")
    out("farthest", name, f"near={near}", f"dist={int(d)}", f"# at ({x:.0f},{z:.0f})")
print("# most_xp: diff this audit's (name, level, xp) snapshot against the previous one")
