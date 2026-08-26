#!/usr/bin/env python3
"""Generate the level 2-50 XP table in doc/LEVEL_CURVE.md. Tunables at top."""
K = 100  # kills per hour by the reference player

HOURS_ANCHORS = [(2, 5 / 60), (10, 22 / 60), (30, 5.5), (50, 8.0)]


def monster_xp(level):
    return {1: 2, 2: 5, 3: 10}.get(level, level * level + 2 * level - 5)


def hours(level):
    for (l0, h0), (l1, h1) in zip(HOURS_ANCHORS, HOURS_ANCHORS[1:]):
        if level <= l1:
            return h0 + (h1 - h0) * (level - l0) / (l1 - l0)
    raise ValueError(level)


def round2(x):
    m = 10 ** max(len(str(int(x))) - 2, 0)
    return int(round(x / m) * m)


def hm(h):
    m = round(h * 60)
    return f"{m // 60}시간 {m % 60}분" if m >= 60 else f"{m}분"


def days(h, per_day):
    d = h / per_day
    return f"{d:.1f}일" if d < 100 else f"{d:.0f}일"


def main():
    cum = total = last = 0
    print("| 레벨 | 소요 시간 | 사냥감 XP | 구간 XP | 누적 XP | 누적 시간 | 4h/일 | 24h/일 봇 |")
    print("|---|---|---|---|---|---|---|---|")
    for level in range(2, 51):
        h = hours(level)
        band = round2(K * monster_xp(level - 1) * h)
        assert band >= last
        last = band
        cum += band
        total += h
        print(f"| {level} | {hm(h)} | {monster_xp(level - 1)} | {band:,} | {cum:,} | {hm(total)} | {days(total, 4)} | {days(total, 24)} |")
    print(f"\n총 {hm(total)} = 하루 4시간 기준 {total / 4:.0f}일")


if __name__ == "__main__":
    main()
