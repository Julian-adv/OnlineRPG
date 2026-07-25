use crate::types::{GameDateTime, ServerMessage};

pub const REAL_DAY_DURATION_SECONDS: f64 = 3.0 * 60.0 * 60.0;
pub const GAME_HOURS_PER_DAY: i64 = 24;
pub const GAME_MINUTES_PER_HOUR: i64 = 60;
pub const GAME_DAYS_PER_MONTH: i64 = 30;
pub const GAME_MONTHS_PER_YEAR: i64 = 12;
pub const GAME_DAYS_PER_YEAR: i64 = GAME_DAYS_PER_MONTH * GAME_MONTHS_PER_YEAR;
pub const GAME_START_YEAR: i64 = 217;
pub const GAME_SECONDS_PER_REAL_SECOND: f64 =
    (GAME_HOURS_PER_DAY as f64 * GAME_MINUTES_PER_HOUR as f64 * 60.0) / REAL_DAY_DURATION_SECONDS;
pub const GAME_SECONDS_PER_DAY: i64 = GAME_HOURS_PER_DAY * GAME_MINUTES_PER_HOUR * 60;

impl super::GameState {
    pub fn default_start_datetime() -> GameDateTime {
        GameDateTime {
            year: GAME_START_YEAR as u32,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
        }
    }

    pub fn datetime_to_total_game_seconds(datetime: &GameDateTime) -> i64 {
        let year = i64::from(datetime.year).max(GAME_START_YEAR);
        let month = i64::from(datetime.month).clamp(1, GAME_MONTHS_PER_YEAR);
        let day = i64::from(datetime.day).clamp(1, GAME_DAYS_PER_MONTH);
        let hour = i64::from(datetime.hour).clamp(0, GAME_HOURS_PER_DAY - 1);
        let minute = i64::from(datetime.minute).clamp(0, GAME_MINUTES_PER_HOUR - 1);

        let years_since_start = year - GAME_START_YEAR;
        let total_days =
            years_since_start * GAME_DAYS_PER_YEAR + (month - 1) * GAME_DAYS_PER_MONTH + (day - 1);
        let total_minutes = total_days * GAME_HOURS_PER_DAY * GAME_MINUTES_PER_HOUR
            + hour * GAME_MINUTES_PER_HOUR
            + minute;
        total_minutes * 60
    }

    pub fn total_game_seconds_to_datetime(total_game_seconds: i64) -> GameDateTime {
        let total_seconds = total_game_seconds.max(0);
        let total_minutes = total_seconds / 60;
        let total_days = total_minutes / (GAME_HOURS_PER_DAY * GAME_MINUTES_PER_HOUR);

        let minutes_in_day = total_minutes % (GAME_HOURS_PER_DAY * GAME_MINUTES_PER_HOUR);
        let hour = (minutes_in_day / GAME_MINUTES_PER_HOUR) as u8;
        let minute = (minutes_in_day % GAME_MINUTES_PER_HOUR) as u8;

        let year = GAME_START_YEAR + (total_days / GAME_DAYS_PER_YEAR);
        let day_of_year = total_days % GAME_DAYS_PER_YEAR;
        let month = (day_of_year / GAME_DAYS_PER_MONTH) + 1;
        let day = (day_of_year % GAME_DAYS_PER_MONTH) + 1;

        GameDateTime {
            year: year as u32,
            month: month as u8,
            day: day as u8,
            hour,
            minute,
        }
    }

    pub fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    pub fn current_total_game_seconds(&self) -> i64 {
        let clock = self.game_clock.read().expect("game clock lock poisoned");
        let elapsed_real_seconds = clock.start_real.elapsed().as_secs_f64();
        let elapsed_game_seconds =
            (elapsed_real_seconds * GAME_SECONDS_PER_REAL_SECOND).floor() as i64;
        clock.start_game_seconds + elapsed_game_seconds
    }

    /// Debug: jump the game clock to the next occurrence of `hour:minute`,
    /// then broadcast the new time immediately. Only ever moves time forward —
    /// rewinding the persisted world clock would confuse schedules and saves.
    pub fn debug_set_time(&self, hour: u8, minute: u8) -> GameDateTime {
        let hour = i64::from(hour).clamp(0, GAME_HOURS_PER_DAY - 1);
        let minute = i64::from(minute).clamp(0, GAME_MINUTES_PER_HOUR - 1);

        let current = self.current_total_game_seconds();
        let day_start = current - current.rem_euclid(GAME_SECONDS_PER_DAY);
        let mut target = day_start + (hour * GAME_MINUTES_PER_HOUR + minute) * 60;
        if target <= current {
            target += GAME_SECONDS_PER_DAY;
        }

        // The write guard must drop before broadcast_game_time, which
        // re-acquires the clock lock for reading.
        {
            let mut clock = self.game_clock.write().expect("game clock lock poisoned");
            clock.start_real = std::time::Instant::now();
            clock.start_game_seconds = target;
        }
        self.broadcast_game_time()
    }

    pub fn current_game_datetime(&self) -> GameDateTime {
        Self::total_game_seconds_to_datetime(self.current_total_game_seconds())
    }

    pub fn is_night(datetime: &GameDateTime) -> bool {
        crate::celestial::is_night(datetime)
    }

    /// Whole game days since the clock's epoch — the rollover key for
    /// midnight-based daily resets (NPC salaries, haggling budgets).
    pub fn game_day(total_game_seconds: i64) -> i64 {
        total_game_seconds.div_euclid(GAME_SECONDS_PER_DAY)
    }

    pub fn current_game_day(&self) -> i64 {
        Self::game_day(self.current_total_game_seconds())
    }

    /// How many nightfalls the world clock has passed, as a monotonically
    /// increasing index. Sunset at this latitude always lands between noon
    /// and midnight, so every game day holds exactly one boundary and the
    /// index never moves backwards. Resets that key off nightfall rather
    /// than midnight (the dungeon chest) compare two of these instead of a
    /// wall-clock duration, which keeps them correct across a server restart
    /// (the world clock is persisted, wall time spent offline is not).
    pub fn night_epoch(total_game_seconds: i64) -> i64 {
        let datetime = Self::total_game_seconds_to_datetime(total_game_seconds);
        Self::game_day(total_game_seconds) + i64::from(crate::celestial::is_after_sunset(&datetime))
    }

    #[cfg(test)]
    pub(super) fn night_epoch_at(year: u32, month: u8, day: u8, hour: u8, minute: u8) -> i64 {
        Self::night_epoch(Self::datetime_to_total_game_seconds(&GameDateTime {
            year,
            month,
            day,
            hour,
            minute,
        }))
    }

    pub fn broadcast_game_time(&self) -> GameDateTime {
        let datetime = self.current_game_datetime();
        self.broadcast(ServerMessage::GameTimeSync {
            is_night: Self::is_night(&datetime),
            datetime: datetime.clone(),
        });
        datetime
    }
}
