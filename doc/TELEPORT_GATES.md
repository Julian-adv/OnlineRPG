# Town Teleport Gates

Town teleport gates are a paid convenience network linking the eight authored
towns: Aldermark, Brovik, Edra, Frihavn, Garasden, Mistfall, Riftmark, and
Stenhavn. Each gate stands beside its town label and presents the same set of
other-town destinations.

## Player contract

- Clicking a gate walks the character into interaction range and opens a list
  of destinations.
- Every destination shows its server-quoted distance and fare before travel.
- Fares rise with distance: `1,000c + 500c * ceil(distance_m / 1,000)`.
- The gate has a 0.5% misfire chance: one activation in 200 on average. A
  misfire throws the character to a random point on the surface or inside a
  real generated dungeon and still charges the originally quoted fare.
- The physical notice in front of every gate and the destination dialog both
  disclose distance pricing and the 0.5% wild-misfire risk before payment.
- A character must be alive, on the surface, out of recent combat, within six
  metres of the source gate, and able to pay the full fare.

Surface points span the full baked world and may resolve to remote land, a
river, or open sea. Dungeon points select a generated dungeon, depth, and
clear authored carved cell. The outcome is intentionally risky and can strand a
character far from a town, so the dialog recommends carrying a Scroll of
Return. The server still avoids solid dungeon walls, props, treasure chests,
and monster spawn cells so “anywhere” remains a valid game position.

## Authority and transaction order

The browser does not calculate fares, choose the random outcome, deduct money,
or choose terrain height. It sends source and destination IDs. The server then:

1. validates the source gate and proximity;
2. calculates the fare from shared gate data;
3. chooses the normal destination or a seeded wild surface/dungeon point;
4. samples terrain and water height for surface arrivals, or uses a carved
   dungeon cell and its real floor level;
5. revalidates source access after the asynchronous height lookup;
6. atomically checks and deducts the wallet balance;
7. sends the balance update and teleports the character; and
8. reports the requested town, actual arrival description, charged fare, and
   misfire result.

A per-character in-flight guard rejects duplicate travel requests. Destination
height is resolved before payment, so an unavailable destination does not
charge the player. Twenty percent of the already-rare misfires choose a dungeon;
the rest choose a random surface point, whose terrain naturally determines
whether it is land or water.

## Shared data and tuning

Gate placement is authored in `data-src/teleport_gates.csv`. Network tuning is
authored in `data-src/teleport_gate_config.csv`, including fare bands,
interaction distance, arrival offset, the 50-basis-point misfire chance, and
the dungeon share of misfires. The generated JSON is read by both Rust and the
browser, while `shared/src/teleport.rs` owns validation and fare calculation.
Changing protocol meaning or message shape requires another protocol-version
bump.

Future additions can extend the registry with unlock rules, regional taxes,
reputation discounts, temporary closures, or non-town networks. Those systems
should preserve server-issued quotes and visibly disclose any changed risk or
cost before activation.
