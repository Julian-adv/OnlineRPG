use super::*;

impl SharedState {
    /// Current game time snapshot for schedule resolution.
    pub fn time_context(&self) -> (Option<bool>, Option<u32>, Option<u32>, Option<bool>) {
        (
            self.is_night,
            self.game_hour,
            self.game_minute,
            // A played-out meeting stops matching; the NPC goes home.
            self.is_serin_dark_day.filter(|_| !self.meeting_done()),
        )
    }

    /// This agent's own laid-out stall, if one is out.
    pub fn own_stall(&self) -> Option<&onlinerpg_shared::stall::Stall> {
        self.self_player_id
            .and_then(|id| self.stalls.values().find(|s| s.owner == id))
    }

    pub fn format_world_state(&self) -> String {
        let mut lines = Vec::new();

        if let Some(ref p) = self.self_player {
            lines.push(format!(
                "You: {} Lv.{} {:?} HP {}/{} at ({:.1}, {:.1}, {:.1})",
                p.name,
                p.level,
                p.class,
                p.health,
                p.max_health,
                p.position.x,
                p.position.y,
                p.position.z
            ));
            if p.health == 0 {
                lines.push(
                    "You are DEFEATED (HP 0). You do NOT recover on your own and most \
                     actions stay blocked. Respawn now with {\"type\": \"respawn\"} — \
                     the death penalty was already paid when you fell; respawning \
                     costs nothing more."
                        .to_string(),
                );
            }
        }
        if let Some(line) = self.format_dungeon_state() {
            lines.push(line);
        }
        if let Some((satiation, state)) = self.self_hunger {
            let mut line = format!("Hunger: {state:?} ({satiation}/1000)");
            if !self.self_debuffs.is_empty() {
                line.push_str(&format!(", debuffs: {}", self.self_debuffs.join(", ")));
            }
            lines.push(line);
        }
        if let Some(p) = &self.self_player {
            let nearest_fire = self
                .campfires
                .values()
                .filter(|c| c.floor_level == p.floor_level)
                .map(|c| c.position.dist_xz_sq(&p.position))
                .min_by(f32::total_cmp);
            if let Some(d2) = nearest_fire {
                lines.push(format!(
                    "Campfire nearby: {:.1}m away (use a raw fish within 3m to grill it)",
                    d2.sqrt()
                ));
            }
            if let Some(own_stall) = self.own_stall() {
                lines.push(format!(
                    "Your stall is laid out {:.1}m away",
                    own_stall.position.dist_xz_sq(&p.position).sqrt()
                ));
            }
        }
        if let Some(gold) = self.self_gold {
            lines.push(format!(
                "Your gold: {}",
                crate::shop_info::format_price(gold)
            ));
        }
        if !self.self_bag.is_empty() {
            let items: Vec<String> = self
                .self_bag
                .iter()
                .map(|i| {
                    if i.quantity > 1 {
                        format!("{} x{}", i.item_def_id, i.quantity)
                    } else {
                        i.item_def_id.clone()
                    }
                })
                .collect();
            lines.push(format!("Your bag: {}", items.join(", ")));
        }
        if !self.self_equipped.is_empty() {
            let mut worn: Vec<String> = self
                .self_equipped
                .iter()
                .map(|(slot, i)| format!("{}: {}", slot.as_str(), i.item_def_id))
                .collect();
            worn.sort();
            lines.push(format!("You are wearing: {}", worn.join(", ")));
        }
        if !self.self_titles.is_empty() {
            let shown = self.self_player.as_ref().and_then(|me| me.title.as_deref());
            let list: Vec<String> = self
                .self_titles
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let mark = if shown == Some(id.as_str()) {
                        " (shown)"
                    } else {
                        ""
                    };
                    format!("{}. {}{mark}", i + 1, crate::title_defs::title_name(id))
                })
                .collect();
            lines.push(format!(
                "Your titles: {} — say \"/title N\" to show one, \"/title off\" to hide",
                list.join("; ")
            ));
        }
        // Data only — what to do with the list is the role template's call
        // (bard.txt: prefer something fresh, unless a listener asks again).
        if self.plays_music && !self.recent_songs.is_empty() {
            let list: Vec<&str> = self.recent_songs.iter().map(String::as_str).collect();
            lines.push(format!(
                "Songs you played recently, oldest first: {}",
                list.join(", ")
            ));
        }

        if !self.party_members.is_empty() {
            let names: Vec<String> = self
                .party_members
                .iter()
                .map(|m| {
                    if Some(m.id) == self.party_leader {
                        format!("{} (leader)", m.name)
                    } else {
                        m.name.clone()
                    }
                })
                .collect();
            lines.push(format!("Your party: {}", names.join(", ")));
        }
        for invite in self.live_party_invites() {
            lines.push(format!(
                "Pending party invite from {} — answer with party_accept or party_decline",
                invite.inviter_name
            ));
        }
        for summon in self.live_party_summons() {
            lines.push(format!(
                "{} calls you to their side (summoning scroll) — answer with summon_accept or summon_decline",
                summon.caster_name
            ));
        }
        for request in self.live_friend_requests() {
            lines.push(format!(
                "Pending friend request from {} — answer with friend_accept or friend_decline",
                request.requester_name
            ));
        }
        if !self.friends.is_empty() {
            let names: Vec<&str> = self.friends.iter().map(|f| f.name.as_str()).collect();
            lines.push(format!("Your friends: {}", names.join(", ")));
        }
        if let Some(offer) = self.pushed_trade.as_ref().filter(|t| t.is_live()) {
            let name = &offer.merchant_name;
            lines.push(format!(
                "{name}'s trade window is open on your screen — buy, sell, or decline_trade"
            ));
        }

        // Nearby players (exclude self and humans beyond the sight radius)
        let sp = self.self_player.as_ref();
        let sight_sq = NPC_SIGHT_RADIUS * NPC_SIGHT_RADIUS;
        let index = self.pricing.as_ref().map_or(100, |p| p.index_percent);
        for (_, p) in self.players_on_my_floor() {
            if self.self_player_id.as_ref() == Some(&p.id) {
                continue;
            }
            if let Some(sp) = sp {
                if p.position.dist_xz_sq(&sp.position) > sight_sq {
                    continue;
                }
            }
            let npc_tag = if p.is_official_npc { " (NPC)" } else { "" };
            let title_tag = p
                .title
                .as_deref()
                .map(|t| format!(" \"{}\"", crate::title_defs::title_name(t)))
                .unwrap_or_default();
            let favor_tag = match self.favor.get(&p.name) {
                Some(v) if !p.is_official_npc && *v != 0 => format!(" (favor {v:+})"),
                _ => String::new(),
            };
            lines.push(format!(
                "Player: {}{title_tag}{npc_tag}{favor_tag} Lv.{} HP {}/{} at ({:.1}, {:.1}, {:.1})",
                p.name, p.level, p.health, p.max_health, p.position.x, p.position.y, p.position.z
            ));
            if p.is_official_npc {
                if let Some(shop) = crate::shop_info::shop_line_for(&p.name, index) {
                    lines.push(shop);
                }
            }
        }

        // Exclude monsters beyond LLM sight radius
        for m in self.monsters_on_my_floor() {
            if let Some(sp) = sp {
                if m.position.dist_xz_sq(&sp.position) > sight_sq {
                    continue;
                }
            }
            lines.push(format!(
                "Monster: {} [{}] HP {}/{} state={} at ({:.1}, {:.1}, {:.1})",
                m.monster_type,
                m.id,
                m.health,
                m.max_health,
                m.state,
                m.position.x,
                m.position.y,
                m.position.z
            ));
        }

        // Items on the ground. Drops linger for minutes, so a busy hunting
        // ground would otherwise stack dozens of lines into every prompt.
        let ground = self.ground_items_in_sight();
        let hidden = ground.len().saturating_sub(MAX_LISTED_GROUND_ITEMS);
        for (d_sq, i) in ground.into_iter().take(MAX_LISTED_GROUND_ITEMS) {
            let dropped_by = match i.dropped_by.as_ref() {
                Some(id) if self.self_player_id.as_ref() == Some(id) => {
                    ", dropped by you".to_string()
                }
                Some(id) => format!(", dropped by {}", self.visible_name(id)),
                None => String::new(),
            };
            let amount = if i.quantity > 1 {
                format!(" x{}", i.quantity)
            } else {
                String::new()
            };
            let key_note =
                if crate::item_defs::get(&i.item_def_id).is_some_and(|d| d.is_dungeon_key()) {
                    " — DUNGEON KEY, pick it up first"
                } else {
                    ""
                };
            lines.push(format!(
                "Item on ground: {}{amount} ({:.1}m away) [id {}]{dropped_by}{key_note}",
                i.item_def_id,
                d_sq.sqrt(),
                i.instance_id
            ));
        }
        if hidden > 0 {
            lines.push(format!("(and {hidden} more items further away)"));
        }

        // Tip hats within sight on our floor: where to drop coins after a
        // performance. Our own hat is listed too (it shows as yours).
        if let Some(sp) = sp {
            for hat in self.tip_hats.values() {
                if hat.floor_level != sp.floor_level {
                    continue;
                }
                let d_sq = hat.position.dist_xz_sq(&sp.position);
                if d_sq > sight_sq {
                    continue;
                }
                let whose = if Some(hat.owner) == self.self_player_id {
                    "yours".to_string()
                } else {
                    format!("{}'s — drop coins in it with tip_hat", hat.owner_name)
                };
                lines.push(format!(
                    "Tip hat [id {}] {:.1}m away: {whose}",
                    hat.id,
                    d_sq.sqrt()
                ));
            }
        }

        if lines.is_empty() {
            "No state available yet.".to_string()
        } else {
            lines.join("\n")
        }
    }
}
