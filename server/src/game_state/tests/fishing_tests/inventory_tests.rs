use super::*;

/// An overweight bag spills the catch as a ground item — never silently lost.
#[tokio::test]
async fn a_full_bag_spills_the_catch_on_the_ground() {
    let game_state = make_test_game_state("fishing_spill");
    let (id, mut rx) = make_angler(&game_state, "angler_hoarder").await;
    // 150 torches = the full STR-10 carry allowance before the catch.
    game_state
        .inventories
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .bag
        .push(bag_item(700, "torch", 150));

    game_state.award_item(&id, "raw_minnow").await;

    assert!(
        drain(&mut rx).iter().any(|m| matches!(
            m,
            ServerMessage::GroundItemSpawned { item, .. } if item.item_def_id == "raw_minnow"
        )),
        "the overflow catch must spill to the ground"
    );
    assert!(
        game_state
            .inventories
            .read()
            .await
            .get(&id)
            .unwrap()
            .bag
            .iter()
            .all(|i| i.item_def_id != "raw_minnow"),
        "nothing was bagged"
    );
    assert!(
        game_state
            .ground_items
            .read()
            .await
            .values()
            .any(|g| g.item.item_def_id == "raw_minnow"),
        "the fish is on the ground, pickable"
    );
}

/// `category "fish"` rides the same heal path as potions: eating a trout
/// restores its dice and consumes the fish.
#[tokio::test]
async fn eating_a_fish_heals_like_a_potion() {
    let game_state = make_test_game_state("fishing_eat");
    let (id, mut rx) = make_angler(&game_state, "angler_hungry").await;
    game_state
        .inventories
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .bag
        .push(bag_item(800, "raw_trout", 1));
    game_state
        .players
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .health = 2;

    game_state.use_item(&id, 800).await;

    let health = game_state.players.read().await.get(&id).unwrap().health;
    assert!(
        (4..=10).contains(&health),
        "2d4 heal from 2 HP lands in 4..=10 (capped), got {health}"
    );
    assert!(
        game_state
            .inventories
            .read()
            .await
            .get(&id)
            .unwrap()
            .bag
            .iter()
            .all(|i| i.item_def_id != "raw_trout"),
        "the fish was eaten"
    );
    let _ = drain(&mut rx);
}

/// The pure catch-table math: every roll in range maps to a candidate, the
/// boundaries are exact, and skill shifts weight toward rare fish while
/// never boosting rarity-0 junk.
#[test]
fn pick_catch_maps_every_roll_and_skill_keeps_the_table_ordered() {
    use crate::game_state::fishing::{effective_weights, pick_catch, CatchCandidate};
    use onlinerpg_shared::fishing::FLOTSAM_SHARE_PCT;
    use onlinerpg_shared::skills::SKILL_LEVEL_CAP;

    let candidate = |id: &str, rarity, catch_weight, min_fishing_level| CatchCandidate {
        item_def_id: id.into(),
        rarity,
        catch_weight,
        min_fishing_level,
    };
    let candidates = vec![
        candidate("common", 1, 50, 0),
        candidate("rare", 5, 2, 0),
        candidate("junk", 0, 8, 0),
    ];

    let weights = effective_weights(&candidates, 0);
    let total: u64 = weights.iter().sum();
    for roll in 0..total {
        let idx = pick_catch(&weights, roll).expect("every in-range roll lands");
        assert!(idx < candidates.len());
    }
    assert!(
        pick_catch(&weights, total).is_none(),
        "an out-of-range roll picks nothing"
    );
    // Exact boundaries of the cumulative walk.
    assert_eq!(pick_catch(&weights, 0), Some(0));
    assert_eq!(pick_catch(&weights, weights[0] - 1), Some(0));
    assert_eq!(pick_catch(&weights, weights[0]), Some(1));
    assert_eq!(pick_catch(&weights, weights[0] + weights[1]), Some(2));

    // Skill lifts rare fish toward the common without ever passing it, and
    // junk keeps its fixed share instead of thinning out.
    for level in 0..=SKILL_LEVEL_CAP {
        let w = effective_weights(&candidates, level);
        let total: u64 = w.iter().sum();
        assert!(
            w[0] > w[1],
            "the rarer fish overtook the common at level {level}"
        );
        assert_eq!(
            w[2] * 100,
            total * FLOTSAM_SHARE_PCT,
            "junk share drifted at level {level}"
        );
    }
    let gap = |level| {
        let w = effective_weights(&candidates, level);
        w[0] as f64 / w[1] as f64
    };
    assert!(gap(SKILL_LEVEL_CAP) < gap(0), "skill should close the gap");
}

/// Every drop landed on the player's exact position, so a bagful of fish
/// became one sprite underfoot and read as "nothing dropped".
#[tokio::test(start_paused = true)]
async fn dropped_items_scatter_instead_of_stacking() {
    let game_state = make_test_game_state("drop_scatter");
    let (id, _rx) = make_angler(&game_state, "angler_litterbug").await;
    for instance in 600u64..606 {
        game_state
            .inventories
            .write()
            .await
            .get_mut(&id)
            .unwrap()
            .bag
            .push(bag_item(instance, "raw_minnow", 1));
    }

    for instance in 600u64..606 {
        game_state.drop_item(&id, instance).await;
    }

    let dropped: Vec<_> = game_state
        .ground_items
        .read()
        .await
        .values()
        .filter(|g| g.item.item_def_id == "raw_minnow")
        .map(|g| (g.item.position.x, g.item.position.z))
        .collect::<Vec<(f32, f32)>>();
    assert_eq!(dropped.len(), 6, "every drop should reach the ground");
    for (i, a) in dropped.iter().enumerate() {
        for b in dropped.iter().skip(i + 1) {
            assert!(
                (a.0 - b.0).abs() > f32::EPSILON || (a.1 - b.1).abs() > f32::EPSILON,
                "two drops landed on the same spot"
            );
        }
    }
    let player = game_state.players.read().await.get(&id).unwrap().position;
    for (x, z) in &dropped {
        let reach = ((x - player.x).powi(2) + (z - player.z).powi(2)).sqrt();
        assert!(reach <= 2.0, "a drop landed {reach:.2}m away, out of reach");
    }
}
