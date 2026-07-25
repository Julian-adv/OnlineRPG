//! NPC salaries (economy phase 3): the controlled gold faucet that funds
//! non-merchant traders' wallets. Once per game-day rollover, every online
//! NPC with a trader definition is credited its salary, capped at its
//! wallet cap. Wallets persist through the normal character save path.

use tracing::info;

use crate::npc_defs::npc_defs;
use crate::types::{PlayerId, ServerMessage};

impl super::GameState {
    /// Pay resident NPC salaries when the game day rolls over. Called from
    /// the periodic maintenance loop; cheap no-op within the same day.
    /// A multi-day debug time jump pays a single salary — the faucet never
    /// emits more than one payment per rollover observed.
    pub async fn tick_npc_salaries(&self) {
        let game_day = self.current_game_day();
        {
            let mut last = self.npc_salary_last_day.write().await;
            match *last {
                // First tick after boot: record the day, don't pay (a
                // restart must not be a salary source).
                None => {
                    *last = Some(game_day);
                    return;
                }
                Some(day) if day >= game_day => return,
                Some(_) => *last = Some(game_day),
            }
        }

        let recipients: Vec<(PlayerId, String, i64, i64)> = {
            let players = self.players.read().await;
            players
                .values()
                .filter(|p| p.is_official_npc)
                .filter_map(|p| {
                    npc_defs()
                        .get_trader_by_npc_name(&p.name)
                        .map(|def| (p.id, p.name.clone(), def.salary_per_day, def.wallet_cap))
                })
                .collect()
        };

        for (player_id, name, salary, cap) in recipients {
            let paid = {
                let mut gold_map = self.player_gold.write().await;
                let gold = gold_map.entry(player_id).or_insert(0);
                let before = *gold;
                // Cap accumulation, but never confiscate an above-cap wallet
                // (trade proceeds may legitimately exceed the cap).
                *gold = (before + salary).min(cap.max(before));
                (*gold != before).then_some(*gold)
            };
            if let Some(gold) = paid {
                info!("salary: {name} paid {salary} on day {game_day}, wallet now {gold}");
                self.mark_dirty(&player_id).await;
                self.send_direct_message(&player_id, ServerMessage::GoldUpdate { gold })
                    .await;
            } else {
                info!("salary: {name} wallet at cap, no payment on day {game_day}");
            }
        }
    }
}
