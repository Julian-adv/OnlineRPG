#!/usr/bin/env python3
"""Gold faucet/sink summary from openmmo-server journal lines on stdin.

Merchants (Rica, Wick) have infinite wallets: selling to them creates gold,
buying from them destroys it. Buybacks ("bought back") reverse a sale and are
excluded from both sides. 1g = 10,000 copper.
"""
import collections
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")
SELL = re.compile(r"(\S+) sold (\d+)x(\S+) to (\S+) for (\d+)")
BUY = re.compile(r"(\S+) bought (back )?(?:(\d+)x)?(\S+) from (\S+) for (\d+)")
CHEST = re.compile(r"Player (\S+) opened dungeon chest '[^']*': .*\+ (\d+) gold")
COIN = re.compile(r"Player (\S+) picked up a coin pile: \+(\d+) copper")
POUCH = re.compile(r"Player (\S+) opened a coin pouch: \+(\d+) copper")
SALARY = re.compile(r"salary: (\S+) paid (\d+) on day")
TRADE = re.compile(r"Trade: (\S+) gave .*?\+ (\d+) copper to (\S+), who gave .*?\+ (\d+) copper")

inflow = collections.Counter()
outflow = collections.Counter()
by_day = collections.defaultdict(lambda: [0, 0])
sold_item = collections.defaultdict(lambda: [0, 0])
bought_item = collections.defaultdict(lambda: [0, 0])
player_net = collections.Counter()
events = collections.Counter()
buyback = [0, 0]
big_trades = []

for raw in sys.stdin:
    line = ANSI.sub("", raw).rstrip()
    day = line[:10]
    m = SELL.search(line)
    if m:
        g = int(m[5])
        inflow["sell"] += g
        by_day[day][0] += g
        sold_item[m[3]][0] += int(m[2])
        sold_item[m[3]][1] += g
        player_net[m[1]] += g
        events["sell"] += 1
        continue
    m = BUY.search(line)
    if m:
        g = int(m[6])
        if m[2]:
            buyback[0] += 1
            buyback[1] += g
            inflow["sell"] -= g
            by_day[day][0] -= g
            sold_item[m[4]][1] -= g
            player_net[m[1]] -= g
            continue
        outflow["buy"] += g
        by_day[day][1] += g
        bought_item[m[4]][0] += int(m[3] or 1)
        bought_item[m[4]][1] += g
        player_net[m[1]] -= g
        events["buy"] += 1
        continue
    for key, rx in (("chest", CHEST), ("coinpile", COIN), ("pouch", POUCH)):
        m = rx.search(line)
        if m:
            g = int(m[2])
            inflow[key] += g
            by_day[day][0] += g
            player_net[m[1]] += g
            events[key] += 1
            break
    else:
        m = SALARY.search(line)
        if m:
            inflow["salary"] += int(m[2])
            by_day[day][0] += int(m[2])
            events["salary"] += 1
            continue
        m = TRADE.search(line)
        if m:
            a, b = int(m[2]), int(m[4])
            events["p2p"] += 1
            player_net[m[1]] += b - a
            player_net[m[3]] += a - b
            inflow["_p2p"] += a + b
            if max(a, b) >= 200_000:
                big_trades.append(f"{line[:16]} {m[1]} -> {m[3]}: {a / 1e4:.1f}g / back {b / 1e4:.1f}g")


def g(c):
    return f"{c / 10000:,.2f}g"


p2p = inflow.pop("_p2p", 0)
ti, to = sum(inflow.values()), sum(outflow.values())
print("events:", dict(events), f"buybacks excluded: {buyback[0]} / {g(buyback[1])}")
print("INFLOW ", {k: g(v) for k, v in inflow.items()}, "total", g(ti))
print("OUTFLOW", {k: g(v) for k, v in outflow.items()}, "total", g(to))
print("NET", g(ti - to), "| p2p moved", g(p2p))
print("by day (in / out / net):")
for d in sorted(by_day):
    i, o = by_day[d]
    print(f"  {d} {g(i)} {g(o)} {g(i - o)}")
print("top sold (qty, gold):", [(k, q, g(v)) for k, (q, v) in sorted(sold_item.items(), key=lambda x: -x[1][1])[:8]])
print("top bought (qty, gold):", [(k, q, g(v)) for k, (q, v) in sorted(bought_item.items(), key=lambda x: -x[1][1])[:8]])
print("top net players:", [(k, g(v)) for k, v in player_net.most_common(6)])
print("bottom net players:", [(k, g(v)) for k, v in sorted(player_net.items(), key=lambda x: x[1])[:6]])
if big_trades:
    print("large p2p transfers (>=20g one-way):")
    for t in big_trades:
        print("  " + t)
