use super::*;

impl SharedState {
    /// The pose an interaction holds us in — anything but the music emote
    /// (a bed, a chair). `None` while free or strumming.
    pub(super) fn held_pose(&self) -> Option<&str> {
        self.self_player
            .as_ref()?
            .object_type
            .as_deref()
            .filter(|held| *held != MUSIC_EMOTE)
    }

    /// A tune ended: say so, since the agent heard it start. Silent for
    /// anyone who was not playing.
    pub(super) fn finish_music(&mut self, player_id: &PlayerId) {
        let Some(track) = self.music_performers.remove(player_id) else {
            return;
        };
        let is_self = self.self_player_id.as_ref() == Some(player_id);
        let who = if is_self {
            self.self_performance = None;
            self.recital = None;
            let rest = rand::thread_rng().gen_range(MUSIC_REST_MIN_SECS..=MUSIC_REST_MAX_SECS);
            self.self_music_rest_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(rest));
            // The break gets its own allowance: the last song of the evening
            // must not leave the counter stuck with no next song to reset it.
            self.tips_noticed = 0;
            "You".to_string()
        } else {
            self.player_display_name(player_id)
        };
        let line = format!("[PlayMusic] {who} finished \"{track}\".");
        if is_self {
            // No wake yet: the rest ending is what invites the next song, and
            // waking now would only draw a command we would have to refuse.
            self.push_ambient_event_quiet(line);
            // Tips are the exception — the quiet spell is when they get
            // thanked and picked up, and nothing else wakes us before it ends.
            for (_, tip) in std::mem::take(&mut self.pending_tips) {
                self.push_ambient_event(tip);
            }
        } else {
            self.push_ambient_event(line);
        }
    }

    /// Someone took an item a player had put down — a tip snatched, or a
    /// gift collected again. Ordinary loot churn stays silent: the world
    /// state already lists what lies about, and a hunting ground would file
    /// a line for every corpse otherwise.
    pub(super) fn note_pickup(&mut self, item: &GroundItem, picker: &PlayerId) {
        let line = format!(
            "[GroundItem] {} picked up {} [id {}].",
            self.visible_name(picker),
            item.item_def_id,
            item.instance_id
        );
        // Mid-song this is not worth an LLM turn; it rides along with the
        // next prompt, which the end of the song brings soon enough.
        if self.self_performance.is_some() {
            self.push_ambient_event_quiet(line);
        } else {
            self.push_ambient_event(line);
        }
    }

    /// Someone left something at the busker's feet. Announced at once when we
    /// are between songs; held for the end of the song while we are playing.
    /// Only a busker is tipped — a guard's kill drops and a merchant's
    /// neighbours stay ordinary loot — and not while the schedule holds us
    /// in a pose: walking over from a bed would drop it with nothing to
    /// restore it until morning.
    pub(super) fn note_tip(&mut self, item: &GroundItem) {
        let Some(dropper) = item
            .dropped_by
            .filter(|id| self.self_player_id != Some(*id))
        else {
            return;
        };
        let Some(me) = self.self_player.as_ref() else {
            return;
        };
        let posing = self.held_pose().is_some();
        if !self.plays_music
            || posing
            || item.floor_level != self.self_floor_level
            || self.tips_noticed >= MAX_TIPS_PER_SONG
            || item.position.dist_xz_sq(&me.position) > TIP_RADIUS * TIP_RADIUS
        {
            return;
        }
        let note = format!(
            "[Tip] {} left {} at your feet [id {}].",
            self.visible_name(&dropper),
            item.item_def_id,
            item.instance_id
        );
        self.tips_noticed += 1;
        if self.self_performance.is_some() {
            self.pending_tips.push((item.instance_id, note));
        } else {
            self.push_ambient_event(note);
        }
    }

    /// Drop a `/play_music` the agent typed for a song that does not exist, or
    /// while its own tune is still running, or during the quiet spell after it:
    /// a second command restarts the music for every listener, and the LLM is
    /// not patient enough to wait on its own. A timing refusal does not wake the
    /// driver — the end of the rest does that, and waking here would only invite
    /// another attempt. A made-up title does wake it, once: the bard has already
    /// announced the song to the square, and nothing else would prompt it to
    /// take that back before the idle interval, an hour later.
    pub fn refuses_play_command(&mut self, message: &str) -> bool {
        // The same parser the server runs on the other end of this command.
        let Some(query) = onlinerpg_shared::messages::strip_command(message, "/play_music") else {
            return false;
        };
        // An empty query is the server's random pick, and always resolves.
        let query = query.trim();
        if !query.is_empty() && !crate::bgm_defs::knows(query) {
            self.push_agent_event(format!(
                "[PlayMusic] Ignored — there is no song called \"{query}\" in your songbook. \
                 Use a title exactly as the songbook writes it. If you already announced this \
                 one, tell them you had the name wrong and offer a song you do know."
            ));
            if !self.bad_song_title_refused {
                self.bad_song_title_refused = true;
                self.wake(EventUrgency::Urgent);
            }
            return true;
        }
        let now = std::time::Instant::now();
        let why = if let Some(held) = self.held_pose() {
            // Playing would replace the pose the schedule put us in, and
            // nothing would put us back until the next schedule entry.
            format!("you are using the {held} and would have to get up first")
        } else if let Some(perf) = &self.self_performance {
            format!(
                "you are still playing, with about {}s to go",
                perf.ends_at.saturating_duration_since(now).as_secs()
            )
        } else if let Some(rest_until) = self.self_music_rest_until {
            format!(
                "the square is quiet between songs for another {}s",
                rest_until.saturating_duration_since(now).as_secs()
            )
        } else {
            return false;
        };
        self.push_agent_event_quiet(format!(
            "[PlayMusic] Ignored — {why}. One song at a time; wait for the note \
             that says you can start another. If you already announced the \
             title, tell them it is coming rather than leaving the promise \
             hanging."
        ));
        true
    }

    /// Stop strumming once the track we started has run its length, and invite
    /// the next song when the quiet spell after it is over. The web client
    /// ends a performance when its audio ends and rests before the next track;
    /// we have no audio, so this tick is our equivalent — without it an NPC
    /// bard plays one tune forever, or one unbroken stream of them.
    pub fn check_music_finished(&mut self) {
        self.tick_recital();
        if let Some(rest_until) = self.self_music_rest_until {
            if std::time::Instant::now() >= rest_until {
                self.self_music_rest_until = None;
                self.push_ambient_event(
                    "[PlayMusic] The square is quiet again — time for another song.".to_string(),
                );
            }
        }

        let Some(perf) = &self.self_performance else {
            return;
        };
        let walked_off = self.self_player.as_ref().is_some_and(|me| {
            perf.from.dist_xz_sq(&me.position) > MUSIC_STAY_PUT_RADIUS * MUSIC_STAY_PUT_RADIUS
        });
        if !walked_off && std::time::Instant::now() < perf.ends_at {
            return;
        }
        self.self_performance = None;
        self.recital = None;
        // A schedule pose (a bed at 2:00) may have replaced the strum before
        // the song's clock ran out; a StopInteraction would only stand us up.
        if self.held_pose().is_none() {
            self.pending_commands.push(ClientMessage::StopInteraction);
        }
    }

    /// A busker plays on a workhorse instrument — never the starter sword,
    /// and never an offerable keepsake: those stay in the bag, the only
    /// place `shop_info::keepsake_section` offers from. On the join
    /// snapshot, anything else in the main hand is swapped for the cheapest
    /// workhorse. Snapshot-only: what to hold mid-session (a fishing rod,
    /// say) stays the agent's own choice.
    pub(super) fn take_up_instrument(&mut self) {
        if !self.plays_music {
            return;
        }
        let keepsakes = &self.keepsake_ids;
        let workhorse_instr = |i: &onlinerpg_shared::inventory::ItemInstance| {
            crate::item_defs::get(&i.item_def_id).is_some_and(|d| d.is_instrument())
                && !keepsakes.contains(&i.item_def_id)
        };
        let price = |i: &onlinerpg_shared::inventory::ItemInstance| {
            crate::item_defs::get(&i.item_def_id)
                .and_then(|d| d.base_price)
                .unwrap_or(0)
        };
        let Some(workhorse) = self
            .self_bag
            .iter()
            .filter(|i| workhorse_instr(i))
            .min_by_key(|i| price(i))
        else {
            return;
        };
        let held_is_workhorse = self
            .self_equipped
            .get(&onlinerpg_shared::inventory::EquipSlot::MainHand)
            .is_some_and(|i| workhorse_instr(i) && price(i) <= price(workhorse));
        if held_is_workhorse {
            return;
        }
        let instance_id = workhorse.instance_id;
        self.pending_commands
            .push(ClientMessage::EquipItem { instance_id });
    }
}

/// Seconds each verse stays up before the next replaces it.
const RECITE_LINE_SECS: u64 = 9;
/// How long a recital runs with no song of ours playing yet.
const RECITE_GRACE_SECS: u64 = 25;
const MAX_RECITE_LINES: usize = 12;
/// The web client's bubble truncates past 300 characters.
const MAX_RECITE_LINE_CHARS: usize = 280;

impl SharedState {
    /// Start reciting: the first verse waits one interval so the opening
    /// line and the song announcement keep their bubbles, then the verses
    /// follow every `RECITE_LINE_SECS`, cycling until our song ends. A new
    /// recital replaces one still running.
    pub fn begin_recital(&mut self, verses: &[String]) -> Result<(), String> {
        let lines: Vec<String> = verses
            .iter()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.chars().take(MAX_RECITE_LINE_CHARS).collect())
            .take(MAX_RECITE_LINES)
            .collect();
        if lines.is_empty() {
            return Err("no verses to recite".to_string());
        }
        let now = std::time::Instant::now();
        self.recital = Some(Recital {
            lines,
            sent: 0,
            next_at: now + std::time::Duration::from_secs(RECITE_LINE_SECS),
            until: now + std::time::Duration::from_secs(RECITE_GRACE_SECS),
        });
        Ok(())
    }

    fn tick_recital(&mut self) {
        let ends_at = self.self_performance.as_ref().map(|p| p.ends_at);
        let Some(recital) = self.recital.as_mut() else {
            return;
        };
        let now = std::time::Instant::now();
        if let Some(ends_at) = ends_at {
            recital.until = ends_at;
        }
        if now >= recital.until {
            self.recital = None;
            return;
        }
        if now < recital.next_at {
            return;
        }
        let first_pass = recital.sent < recital.lines.len();
        let command = if first_pass {
            "/recite"
        } else {
            "/recite_quiet"
        };
        let line = &recital.lines[recital.sent % recital.lines.len()];
        let message = format!("{command} {line}");
        recital.sent += 1;
        recital.next_at = now + std::time::Duration::from_secs(RECITE_LINE_SECS);
        self.pending_commands
            .push(ClientMessage::ChatMessage { message });
    }
}
