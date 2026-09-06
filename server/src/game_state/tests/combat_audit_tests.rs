use super::*;
use serde_json::Value;
use std::path::Path;

fn config(dir: &Path, ids: &[i64]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("combat-audit.txt"),
        ids.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
}

fn rows(dir: &Path) -> Vec<Value> {
    std::fs::read_dir(dir.join("combat-audit"))
        .unwrap()
        .flat_map(|entry| {
            std::fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

async fn tracked_player(game: &GameState, name: &str, character_id: i64, hp: u32) -> PlayerId {
    let id = pid(name);
    game.register_player_character(&id, character_id, 0, attrs_with_cha(10), 0, Some(500))
        .await;
    let mut player = make_player(name, 100.0, 50.0);
    player.health = hp;
    game.add_player(player).await;
    game.inventories.write().await.insert(
        id,
        PlayerInventory {
            active_ammo: None,
            bag: vec![],
            equipped: HashMap::new(),
        },
    );
    id
}

#[tokio::test]
async fn combat_audit_records_server_rejections_and_filters_other_characters() {
    let game = make_test_game_state("audit_rejections");
    let dir = crate::test_util::unique_temp_dir("audit_rejections");
    config(&dir, &[6229]);
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let id = tracked_player(&game, "watched", 6229, 10).await;
    let other = tracked_player(&game, "other", 6230, 10).await;
    let mut monster = make_monster("far", pos(500.0), 0);
    monster.monster_type = "troll".into();
    game.monsters.write().await.insert("far".into(), monster);
    game.monster_attack(None, "missing", &id).await;
    game.monster_attack(None, "far", &id).await;
    game.monster_attack(None, "far", &id).await;
    game.monster_attack(None, "missing", &other).await;
    game.remove_player(&id).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let rows = rows(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["character_id"], 6229);
    assert_eq!(rows[0]["reason"], "logout");
    assert_eq!(
        rows[0]["monsters"]["unknown"]["rejected"]["missing_monster"],
        1
    );
    assert_eq!(rows[0]["monsters"]["troll"]["server_attempts"], 2);
    assert_eq!(rows[0]["monsters"]["troll"]["rejected"]["out_of_range"], 1);
    assert_eq!(rows[0]["monsters"]["troll"]["rejected"]["cooldown"], 1);
    assert_eq!(rows[0]["end_hp"], 10);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(start_paused = true)]
async fn combat_audit_records_actual_food_potion_and_natural_healing() {
    let game = make_test_game_state("audit_healing");
    let dir = crate::test_util::unique_temp_dir("audit_healing");
    config(&dir, &[6229]);
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let id = tracked_player(&game, "watched", 6229, 1).await;
    game.tick_regeneration().await;
    let natural_hp = game.players.read().await[&id].health;
    assert!(natural_hp > 1);
    game.inventories.write().await.get_mut(&id).unwrap().bag =
        vec![bag_item(1, "bread", 1), bag_item(2, "healing_potion", 1)];
    game.use_item(&id, 1).await;
    game.tick_food_regeneration().await;
    let food_hp = game.players.read().await[&id].health;
    assert!(food_hp > natural_hp);
    game.use_item(&id, 2).await;
    let final_hp = game.players.read().await[&id].health;
    assert_eq!(final_hp, 10);
    config(&dir, &[]);
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let rows = rows(&dir);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["reason"], "disabled");
    assert_eq!(row["start_hp"], 1);
    assert_eq!(row["end_hp"], 10);
    assert_eq!(row["health_gained"]["natural"], natural_hp - 1);
    assert_eq!(row["health_gained"]["food"], food_hp - natural_hp);
    assert_eq!(row["health_gained"]["potion"], final_hp - food_hp);
    game.remove_player(&id).await;
    game.tick_combat_audit(dir.clone(), 30, true).await;
    assert_eq!(self::rows(&dir).len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn combat_audit_tracks_real_overkill_death_revive_and_unanswered_kill() {
    let game = make_test_game_state("audit_combat");
    let dir = crate::test_util::unique_temp_dir("audit_combat");
    config(&dir, &[6229]);
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let id = tracked_player(&game, "watched", 6229, 1).await;
    let position = game.players.read().await[&id].position;
    let mut attacker = make_monster("attacker", position, 0);
    attacker.level_override = Some(255);
    game.monsters
        .write()
        .await
        .insert("attacker".into(), attacker);
    game.monster_attack(None, "attacker", &id).await;
    assert_eq!(game.players.read().await[&id].health, 0);
    assert!(game.revive_in_place(&id, 100).await);
    game.player_characters
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .2
        .r#str = 255;
    let mut target = make_monster("target", position, 0);
    target.health = 1;
    game.monsters.write().await.insert("target".into(), target);
    game.broadcast_player_attack(&id, "target".into()).await;
    assert_eq!(game.monsters.read().await.get("target").unwrap().health, 0);
    game.tick_combat_audit(dir.clone(), 30, true).await;
    let rows = rows(&dir);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row["health_lost"]["monster"], 1);
    assert_eq!(row["health_gained"]["revive"], 10);
    assert_eq!(row["deaths"], 1);
    let monster = &row["monsters"]["test_monster"];
    assert_eq!(monster["hits"], 1);
    assert_eq!(monster["damage"], 1);
    assert_eq!(monster["kills"], 1);
    assert_eq!(monster["kills_without_observed_attempt"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(start_paused = true)]
async fn combat_audit_invalid_or_missing_config_does_not_stop_active_monitoring() {
    let game = make_test_game_state("audit_reload");
    let dir = crate::test_util::unique_temp_dir("audit_reload");
    config(&dir, &[6229]);
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let id = tracked_player(&game, "watched", 6229, 10).await;
    std::fs::write(dir.join("combat-audit.txt"), "{").unwrap();
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    game.monster_attack(None, "missing", &id).await;
    std::fs::remove_file(dir.join("combat-audit.txt")).unwrap();
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    game.monster_attack(None, "missing", &id).await;
    config(&dir, &[]);
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let rows = rows(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["monsters"]["unknown"]["server_attempts"], 2);
    assert_eq!(rows[0]["reason"], "disabled");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(start_paused = true)]
async fn combat_audit_reloads_targets_only_every_ten_minutes() {
    let game = make_test_game_state("audit_reload_interval");
    let dir = crate::test_util::unique_temp_dir("audit_reload_interval");
    config(&dir, &[6229]);
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let id = tracked_player(&game, "watched", 6229, 10).await;
    config(&dir, &[]);
    tokio::time::advance(std::time::Duration::from_secs(599)).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    assert!(rows(&dir).is_empty());
    game.monster_attack(None, "missing", &id).await;
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    game.tick_combat_audit(dir.clone(), 30, false).await;
    let rows = rows(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["reason"], "disabled");
    assert_eq!(rows[0]["monsters"]["unknown"]["server_attempts"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}
