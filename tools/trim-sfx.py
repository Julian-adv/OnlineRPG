#!/usr/bin/env python3
"""Trim an SFX take to house style: cut, tail fade, peak-normalize, 44.1 kHz ogg.

Usage: trim-sfx.py SRC DEST DURATION [FADE_MS]
DURATION may be "auto" to cut where the take drops below -38 dB RMS.
"""
import array, math, re, subprocess, sys, tempfile, pathlib

FLOOR_DB = -38
MAX_AUTO = 1.2


def content_end(src):
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", src, "-ac", "1", "-ar", "44100", "-f", "s16le", "-"],
        capture_output=True,
    ).stdout
    a = array.array("h")
    a.frombytes(raw)
    w, last = 2205, 0
    for i in range(0, len(a), w):
        c = a[i : i + w]
        if not len(c):
            break
        r = math.sqrt(sum(x * x for x in c) / len(c)) + 1e-9
        if 20 * math.log10(r / 32768) > FLOOR_DB:
            last = i + w
    return round(min(last / 44100, MAX_AUTO), 2)


def run(args):
    return subprocess.run(args, capture_output=True, text=True)


def main():
    src, dest = sys.argv[1], sys.argv[2]
    dur = content_end(src) if sys.argv[3] == "auto" else float(sys.argv[3])
    fade = float(sys.argv[4]) / 1000 if len(sys.argv) > 4 else 0.08
    st = max(0.0, dur - fade)

    with tempfile.NamedTemporaryFile(suffix=".wav") as tmp:
        r = run(["ffmpeg", "-v", "error", "-y", "-i", src, "-ac", "1", "-ar", "44100",
                 "-af", f"atrim=0:{dur},afade=t=out:st={st}:d={fade}", tmp.name])
        if r.returncode:
            sys.exit(r.stderr)
        r = run(["ffmpeg", "-i", tmp.name, "-af", "volumedetect", "-f", "null", "-"])
        m = re.search(r"max_volume: (-?[\d.]+) dB", r.stderr)
        if not m:
            sys.exit("no peak found:\n" + r.stderr)
        gain = round(-3.0 - float(m.group(1)), 2)
        r = run(["ffmpeg", "-v", "error", "-y", "-i", tmp.name, "-af", f"volume={gain}dB",
                 "-c:a", "libvorbis", "-q:a", "5", dest])
        if r.returncode:
            sys.exit(r.stderr)
    size = pathlib.Path(dest).stat().st_size
    print(f"{dest}  {dur}s  fade {int(fade*1000)}ms  gain {gain:+} dB  {size} bytes")


if __name__ == "__main__":
    main()
