#!/usr/bin/env python3
"""Batch-generate death-cry candidates with the ElevenLabs SFX API.

Usage: gen-death-sfx.py [--takes N] [--duration S] [subject ...]
Key is read from ~/.config/elevenlabs/key (or $ELEVENLABS_API_KEY).
Candidates land in sfx-candidates/<subject>-<n>.mp3 (gitignored).
"""
import argparse, json, os, pathlib, sys, time, urllib.request

PROMPTS = {
    "orc": "The short, guttural death roar of an orc warrior cut down by a sword in battle",
    "orc_female": "A woman's very short, sharp, high-pitched death scream, a single shrill feminine cry less than half a second long, cut off instantly as a sword strikes her down. Clearly a female human-like voice, high soprano pitch, no growl, no male voice, no music.",
    "hobgoblin": "The harsh, guttural death cry of a hobgoblin soldier felled by a sword: a rough snarling humanoid war-cry choking off as he falls. Voice only, no dog, no barking, no animal, no music.",
    "gnoll": "The yelping, hyena-like death howl of a gnoll cut down in battle",
    "bugbear": "A big shaggy bear-like goblin monster's death cry: a snarling guttural growl breaking into a pained yelp as it is cut down. Animal voice only, no drums, no percussion, no music.",
    "ogre": "A huge brutish ogre monster's deep guttural death groan, a hoarse animal voice choking off as it collapses from a sword wound. Voice only, no horns, no music.",
    "troll": "The drawn-out, rasping death roar of a troll dying from a deep sword wound",
    "stone_golem": "A stone golem crumbling apart, grinding rock and falling rubble as it dies",
    "orc_boss": "The furious, booming death roar of a massive orc chieftain falling in battle",
    "ogre_boss": "A giant ogre warlord monster's short death cry: a deep hoarse beast voice that climbs sharply in pitch at the very end, finishing on a high choked yelp as it falls. Voice only, no horns, no music.",
    "player_death_female": "A young female warrior's short agonized death scream as a sword strikes her down, a single piercing cry cut off abruptly as she falls. Human voice only, no music, no reverb.",
    "player_death_male": "A young male warrior's short agonized death scream as a sword strikes him down, a single hoarse cry cut off abruptly as he falls. Human voice only, no music, no reverb.",
    "sword_stone": "A steel sword blade striking a stone golem: a hard metallic clang ringing off solid rock with a short spray of stone chips and grit. Impact only, no voice, no music.",
    "sword_flesh": "A sword blade cutting deep into a huge fleshy monster: a heavy wet meaty thud with a thick tearing of flesh. Impact only, no voice, no music.",
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
    ap.add_argument("subjects", nargs="*")
    ap.add_argument("--takes", type=int, default=3)
    ap.add_argument("--duration", type=float, default=1.5)
    ap.add_argument("--influence", type=float, default=0.4)
    ap.add_argument("--out", default="sfx-candidates")
    args = ap.parse_args()

    targets = args.subjects or list(PROMPTS)
    unknown = [m for m in targets if m not in PROMPTS]
    if unknown:
        sys.exit(f"no prompt for: {', '.join(unknown)}")

    k = key()
    before = credits(k)
    print(f"credits left: {before}" if before is not None else "credits: n/a (key lacks user_read)")
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    generated = 0
    for m in targets:
        for n in range(1, args.takes + 1):
            dest = out / f"{m}-{n}.mp3"
            if dest.exists():
                print(f"skip {dest}")
                continue
            audio = generate(k, PROMPTS[m], args.duration, args.influence)
            dest.write_bytes(audio)
            generated += 1
            print(f"{dest}  {len(audio)} bytes")

    if generated and before is not None:
        time.sleep(20)  # usage reporting lags a little behind generation
        after = credits(k)
        if after is not None:
            print(f"credits left: {after}  (used {before - after})")


if __name__ == "__main__":
    main()
