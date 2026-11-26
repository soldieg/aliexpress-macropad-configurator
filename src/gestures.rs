use std::{
    fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};

const POWER_DELAY: Duration = Duration::from_secs(5);
const SHUTDOWN_TRIAL_RECEIPT: &[u8] = b"mini-teclado:teste-de-desligamento:v1\n";

pub struct ShutdownTrial {
    path: PathBuf,
    complete: bool,
}

impl ShutdownTrial {
    pub fn load(path: PathBuf) -> Self {
        let complete = fs::read(&path).is_ok_and(|value| value == SHUTDOWN_TRIAL_RECEIPT);
        Self { path, complete }
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn allow_real(&self, armed: bool, simulation: bool) -> bool {
        self.complete && armed && !simulation
    }

    /// A caller records only a real physical gesture's simulation or explicit cancellation.
    pub fn complete(&mut self) -> io::Result<bool> {
        if self.complete {
            return Ok(false);
        }
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, SHUTDOWN_TRIAL_RECEIPT)?;
        self.complete = true;
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GestureConfig {
    pub display: u8,
    pub shutdown: u8,
    pub cancel: u8,
    pub enabled: bool,
    pub shutdown_armed: bool,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            display: 0b0001,
            shutdown: 0b0111,
            cancel: 0b0110,
            enabled: true,
            shutdown_armed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureAction {
    Individual(u8),
    DisplayOff,
    CountdownStarted,
    Cancelled,
    CountdownAborted,
    Shutdown,
    ShutdownSimulation,
}

struct Countdown {
    until: Instant,
    shutdown_mask: u8,
    cancel_mask: u8,
    armed: bool,
}

#[derive(Default)]
struct Stroke {
    held: u8,
    seen: u8,
    overlap: u8,
    order: [u8; 4],
    config: Option<GestureConfig>,
    began_during_countdown: bool,
    consumed: bool,
}

#[derive(Default)]
pub struct GestureEngine {
    stroke: Stroke,
    countdown: Option<Countdown>,
}

impl GestureEngine {
    /// Gesture input is a physical key index (0–3) and a make/break event.
    /// The action fires on full release so a proper superset can take precedence.
    pub fn event(
        &mut self,
        key: u8,
        down: bool,
        now: Instant,
        config: &GestureConfig,
    ) -> Vec<GestureAction> {
        if key >= 4 {
            return Vec::new();
        }

        let bit = 1 << key;
        if (self.stroke.held & bit != 0) == down {
            return Vec::new();
        }

        if down {
            if self.stroke.held == 0 {
                self.stroke.config = Some(*config);
                self.stroke.began_during_countdown = self.countdown.is_some();
            }
            self.stroke.held |= bit;
            if self.stroke.seen & bit == 0 {
                self.stroke.order[self.stroke.seen.count_ones() as usize] = key;
                self.stroke.seen |= bit;
            }
            if self.stroke.held.count_ones() > self.stroke.overlap.count_ones() {
                self.stroke.overlap = self.stroke.held;
            }

            // Cancellation is immediate so a key released just after the deadline
            // still cancels if the full gesture was pressed during the countdown.
            if config.enabled
                && self.countdown.as_ref().is_some_and(|pending| {
                    now <= pending.until && self.stroke.held == pending.cancel_mask
                })
            {
                return self.cancel().into_iter().collect();
            }
            return Vec::new();
        }

        self.stroke.held &= !bit;
        if self.stroke.held != 0 {
            return Vec::new();
        }

        let stroke = std::mem::take(&mut self.stroke);
        if stroke.consumed {
            return Vec::new();
        }

        if config.enabled
            && stroke.config == Some(*config)
            && !stroke.began_during_countdown
            && self.countdown.is_none()
            && stroke.overlap == stroke.seen
        {
            if stroke.seen == config.display {
                return vec![GestureAction::DisplayOff];
            }
            if stroke.seen == config.shutdown {
                self.countdown = Some(Countdown {
                    until: now + POWER_DELAY,
                    shutdown_mask: config.shutdown,
                    cancel_mask: config.cancel,
                    armed: config.shutdown_armed,
                });
                return vec![GestureAction::CountdownStarted];
            }
        }

        stroke.order[..stroke.seen.count_ones() as usize]
            .iter()
            .copied()
            .map(GestureAction::Individual)
            .collect()
    }

    pub fn tick(&mut self, now: Instant, config: &GestureConfig) -> Option<GestureAction> {
        let pending = self.countdown.as_mut()?;
        if !config.enabled
            || pending.shutdown_mask != config.shutdown
            || pending.cancel_mask != config.cancel
        {
            self.cancel();
            return Some(GestureAction::CountdownAborted);
        }

        // A countdown begun or continued in simulation can never become live.
        pending.armed &= config.shutdown_armed;
        if now < pending.until {
            return None;
        }

        let action = if pending.armed {
            GestureAction::Shutdown
        } else {
            GestureAction::ShutdownSimulation
        };
        self.countdown = None;
        Some(action)
    }

    pub fn cancel(&mut self) -> Option<GestureAction> {
        self.countdown.take()?;
        self.stroke.consumed = self.stroke.held != 0;
        Some(GestureAction::Cancelled)
    }

    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.countdown
            .as_ref()
            .map(|pending| pending.until.saturating_duration_since(now))
    }

    pub fn present_countdown(&mut self, now: Instant) {
        if let Some(pending) = self.countdown.as_mut() {
            pending.until = now + POWER_DELAY;
        }
    }

    pub fn will_shutdown(&self) -> bool {
        self.countdown.as_ref().is_some_and(|pending| pending.armed)
    }

    pub fn pressed_mask(&self) -> u8 {
        self.stroke.held
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_shutdown_runs_the_countdown_in_simulation_and_receipt_survives_restart() {
        let path = std::env::temp_dir().join(format!(
            "mini-teclado-first-shutdown-{}",
            std::process::id()
        ));
        let mut trial = ShutdownTrial::load(path.clone());
        assert!(!trial.is_complete());
        assert!(!trial.allow_real(true, false));

        let mut engine = GestureEngine::default();
        let mut cfg = GestureConfig {
            shutdown_armed: trial.allow_real(true, false),
            ..Default::default()
        };
        let now = Instant::now();
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        assert!(!engine.will_shutdown());
        assert_eq!(engine.tick(now + Duration::from_secs(4), &cfg), None);
        assert_eq!(
            engine.tick(now + Duration::from_secs(5), &cfg),
            Some(GestureAction::ShutdownSimulation)
        );
        assert!(trial.complete().unwrap());
        assert!(!trial.complete().unwrap());

        let restored = ShutdownTrial::load(path.clone());
        assert!(restored.is_complete());
        assert!(!restored.allow_real(true, true));
        assert!(!restored.allow_real(false, false));
        cfg.shutdown_armed = restored.allow_real(true, false);
        let later = now + Duration::from_secs(6);
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, later, &cfg),
            [GestureAction::CountdownStarted]
        );
        assert!(engine.will_shutdown());
        assert_eq!(engine.cancel(), Some(GestureAction::Cancelled));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn unreadable_or_invalid_shutdown_receipt_keeps_real_shutdown_disarmed() {
        let path = std::env::temp_dir().join(format!(
            "mini-teclado-invalid-shutdown-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"partial").unwrap();
        assert!(!ShutdownTrial::load(path.clone()).allow_real(true, false));
        let mut blocked = ShutdownTrial::load(path.join("blocked"));
        assert!(blocked.complete().is_err());
        assert!(!blocked.allow_real(true, false));
        std::fs::remove_file(path).unwrap();
    }

    fn press(engine: &mut GestureEngine, key: u8, now: Instant, cfg: &GestureConfig) {
        assert!(engine.event(key, true, now, cfg).is_empty());
    }

    fn stroke(
        engine: &mut GestureEngine,
        mask: u8,
        now: Instant,
        cfg: &GestureConfig,
    ) -> Vec<GestureAction> {
        let keys = (0..4)
            .filter(|key| mask & (1 << key) != 0)
            .collect::<Vec<_>>();
        for &key in &keys {
            press(engine, key, now, cfg);
        }
        keys.into_iter()
            .flat_map(|key| engine.event(key, false, now, cfg))
            .collect()
    }

    #[test]
    fn one_key_defaults_to_display_off_on_release_and_repeat_does_not_retrigger() {
        let (mut engine, cfg, now) = (
            GestureEngine::default(),
            GestureConfig::default(),
            Instant::now(),
        );
        press(&mut engine, 0, now, &cfg);
        press(&mut engine, 0, now, &cfg);
        assert_eq!(
            engine.event(0, false, now, &cfg),
            [GestureAction::DisplayOff]
        );
        assert!(engine.event(0, false, now, &cfg).is_empty());
    }

    #[test]
    fn four_key_shutdown_waits_for_release_and_beats_shorter_display_gesture() {
        let mut engine = GestureEngine::default();
        let cfg = GestureConfig {
            shutdown: 0b1111,
            shutdown_armed: true,
            ..Default::default()
        };
        let now = Instant::now();
        for key in 0..4 {
            press(&mut engine, key, now, &cfg);
        }
        for key in 0..3 {
            assert!(engine.event(key, false, now, &cfg).is_empty());
        }
        let released = now + Duration::from_secs(2);
        assert_eq!(
            engine.event(3, false, released, &cfg),
            [GestureAction::CountdownStarted]
        );
        assert_eq!(engine.remaining(released), Some(Duration::from_secs(5)));
        assert_eq!(
            engine.tick(released + Duration::from_millis(4999), &cfg),
            None
        );
        assert_eq!(
            engine.tick(released + Duration::from_secs(5), &cfg),
            Some(GestureAction::Shutdown)
        );
        assert_eq!(engine.tick(released + Duration::from_secs(6), &cfg), None);
    }

    #[test]
    fn a_delayed_countdown_gets_five_full_seconds_from_presentation() {
        let mut engine = GestureEngine::default();
        let cfg = GestureConfig {
            shutdown_armed: true,
            ..Default::default()
        };
        let queued = Instant::now();
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, queued, &cfg),
            [GestureAction::CountdownStarted]
        );

        let shown = queued + Duration::from_secs(7);
        engine.present_countdown(shown);
        assert_eq!(engine.remaining(shown), Some(Duration::from_secs(5)));
        assert_eq!(engine.tick(shown, &cfg), None);
        assert_eq!(engine.tick(shown + Duration::from_millis(4999), &cfg), None);
        assert_eq!(
            engine.tick(shown + Duration::from_secs(5), &cfg),
            Some(GestureAction::Shutdown)
        );
    }

    #[test]
    fn cancellation_wins_even_if_it_is_also_the_display_gesture() {
        let mut engine = GestureEngine::default();
        let cfg = GestureConfig {
            display: 0b11,
            cancel: 0b11,
            shutdown_armed: true,
            ..Default::default()
        };
        let now = Instant::now();
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        let late = now + Duration::from_millis(4950);
        press(&mut engine, 0, late, &cfg);
        assert_eq!(
            engine.event(1, true, late, &cfg),
            [GestureAction::Cancelled]
        );
        assert!(engine.event(0, false, late, &cfg).is_empty());
        assert!(engine.event(1, false, late, &cfg).is_empty());
        assert_eq!(engine.tick(now + Duration::from_secs(6), &cfg), None);
        assert_eq!(
            stroke(&mut engine, cfg.display, late, &cfg),
            [GestureAction::DisplayOff]
        );
    }

    #[test]
    fn disarmed_countdown_never_becomes_live_and_can_be_cancelled_or_reset() {
        let (mut engine, cfg, now) = (
            GestureEngine::default(),
            GestureConfig::default(),
            Instant::now(),
        );
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        let armed = GestureConfig {
            shutdown_armed: true,
            ..cfg
        };
        assert_eq!(
            engine.tick(now + Duration::from_secs(5), &armed),
            Some(GestureAction::ShutdownSimulation)
        );
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        assert_eq!(engine.cancel(), Some(GestureAction::Cancelled));
        assert_eq!(engine.cancel(), None);
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        engine.reset();
        assert_eq!(engine.remaining(now), None);
    }

    #[test]
    fn pause_preserves_single_actions_and_aborts_power_countdown() {
        let (mut engine, cfg, now) = (
            GestureEngine::default(),
            GestureConfig::default(),
            Instant::now(),
        );
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        let paused = GestureConfig {
            enabled: false,
            ..cfg
        };
        assert_eq!(
            engine.tick(now, &paused),
            Some(GestureAction::CountdownAborted)
        );
        assert_eq!(
            stroke(&mut engine, paused.display, now, &paused),
            [GestureAction::Individual(0)]
        );
        assert_eq!(
            stroke(&mut engine, paused.shutdown, now, &paused),
            [
                GestureAction::Individual(0),
                GestureAction::Individual(1),
                GestureAction::Individual(2)
            ]
        );
    }

    #[test]
    fn rolled_two_key_gestures_cannot_form_a_three_key_shutdown() {
        let (mut engine, cfg, now) = (
            GestureEngine::default(),
            GestureConfig::default(),
            Instant::now(),
        );
        press(&mut engine, 2, now, &cfg);
        press(&mut engine, 1, now, &cfg);
        assert!(engine.event(2, false, now, &cfg).is_empty());
        press(&mut engine, 0, now, &cfg);
        assert!(engine.event(1, false, now, &cfg).is_empty());
        assert_eq!(
            engine.event(0, false, now, &cfg),
            [
                GestureAction::Individual(2),
                GestureAction::Individual(1),
                GestureAction::Individual(0)
            ]
        );
        assert_eq!(engine.remaining(now), None);
    }

    #[test]
    fn a_duplicate_power_binding_prefers_display_and_ignores_out_of_range_keys() {
        let (mut engine, now) = (GestureEngine::default(), Instant::now());
        let cfg = GestureConfig {
            shutdown: 1,
            shutdown_armed: true,
            ..Default::default()
        };
        assert_eq!(
            stroke(&mut engine, 1, now, &cfg),
            [GestureAction::DisplayOff]
        );
        assert!(engine.event(255, true, now, &cfg).is_empty());
        assert!(engine.event(255, false, now, &cfg).is_empty());
    }

    #[test]
    fn all_four_keys_can_cancel_and_remain_visible_until_release() {
        let mut engine = GestureEngine::default();
        let cfg = GestureConfig {
            shutdown: 0b0100,
            cancel: 0b1111,
            ..Default::default()
        };
        let now = Instant::now();
        assert_eq!(
            stroke(&mut engine, cfg.shutdown, now, &cfg),
            [GestureAction::CountdownStarted]
        );
        for key in 0..3 {
            press(&mut engine, key, now, &cfg);
        }
        assert_eq!(engine.pressed_mask(), 0b0111);
        assert_eq!(engine.event(3, true, now, &cfg), [GestureAction::Cancelled]);
        assert_eq!(engine.pressed_mask(), 0b1111);
        for key in 0..4 {
            assert!(engine.event(key, false, now, &cfg).is_empty());
        }
        assert_eq!(engine.pressed_mask(), 0);
    }
}
