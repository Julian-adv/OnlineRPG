#!/usr/bin/env python3
"""Batch-generate monster death-cry candidates with the ElevenLabs SFX API.

Usage: gen-death-sfx.py [--takes N] [--duration S] [monster ...]
Key is read from ~/.config/elevenlabs/key (or $ELEVENLABS_API_KEY).
Candidates land in sfx-candidates/<monster>-<n>.mp3 (gitignored).
"""
import argparse, json, os, pathlib, sys, time, urllib.request

PROMPTS = {
    "orc": "The short, guttural death roar of an orc warrior cut down by a sword in battle",
    "orc_female": "The short death cry of a female orc warrior struck down by a sword in battle",
    "hobgoblin": "The harsh, barking death cry of a hobgoblin soldier felled by a sword",
    "gnoll": "The yelping, hyena-like death howl of a gnoll cut down in battle",
    "bugbear": "A big shaggy bear-like goblin monster's death cry: a snarling guttural growl breaking into a pained yelp as it is cut down. Animal voice only, no drums, no percussion, no music.",
    "ogre": "A huge brutish ogre monster's deep guttural death groan, a hoarse animal voice choking off as it collapses from a sword wound. Voice only, no horns, no music.",
    "troll": "The drawn-out, rasping death roar of a troll dying from a deep sword wound",
    "stone_golem": "A stone golem crumbling apart, grinding rock and falling rubble as it dies",
    "orc_boss": "The furious, booming death roar of a massive orc chieftain falling in battle",
    "ogre_boss": "A giant ogre warlord monster's short death cry: a deep hoarse beast voice that climbs sharply in pitch at the very end, finishing on a high choked yelp as it falls. Voice only, no horns, no music.",
    "scp939": "The wet, distorted death shriek of a fleshy eyeless monster, unnatural and wrong",
}

API = "https://api.elevenlabs.io/v1/sound-generation"
SUB = "https://api.elevenlabs.io/v1/user/subscription"


def key():
    k = os.environ.get("ELEVENLABS_API_KEY")
    if k:
        return k.strip()
    p = pathlib.Path.home() / ".config/elevenlabs/key"
    if p.exists():
        return p.read_text().strip()
    sys.exit("no API key: set $ELEVENLABS_API_KEY or write ~/.config/elevenlabs/key")


def credits(k):
    """Remaining credits, or None if the key lacks the user_read permission."""
    req = urllib.request.Request(SUB, headers={"xi-api-key": k})
    try:
        with urllib.request.urlopen(req) as r:
            d = json.load(r)
    except urllib.error.HTTPError:
        return None
    return d["character_limit"] - d["character_count"]


def generate(k, text, duration, influence):
    body = json.dumps(
        {"text": text, "duration_seconds": duration, "prompt_influence": influence}
    ).encode()
    req = urllib.request.Request(
        API, data=body, headers={"xi-api-key": k, "Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req) as r:
        return r.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("monsters", nargs="*", default=[])
    ap.add_argument("--takes", type=int, default=3)
    ap.add_argument("--duration", type=float, default=1.5)
    ap.add_argument("--influence", type=float, default=0.4)
    ap.add_argument("--out", default="sfx-candidates")
    args = ap.parse_args()

    targets = args.monsters or list(PROMPTS)
    unknown = [m for m in targets if m not in PROMPTS]
    if unknown:
        sys.exit(f"no prompt for: {', '.join(unknown)}")

    k = key()
    before = credits(k)
    print(f"credits left: {before}" if before is not None else "credits: n/a (key lacks user_read)")
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    prompts_log = {}

    for m in targets:
        prompts_log[m] = PROMPTS[m]
        for n in range(1, args.takes + 1):
            dest = out / f"{m}-{n}.mp3"
            if dest.exists():
                print(f"skip {dest}")
                continue
            audio = generate(k, PROMPTS[m], args.duration, args.influence)
            dest.write_bytes(audio)
            print(f"{dest}  {len(audio)} bytes")

    time.sleep(20)  # usage reporting lags a little behind generation
    after = credits(k)
    if before is not None and after is not None:
        print(f"credits left: {after}  (used {before - after})")

    log = out / "prompts.json"
    old = json.loads(log.read_text()) if log.exists() else {}
    old.update(prompts_log)
    log.write_text(json.dumps(old, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
