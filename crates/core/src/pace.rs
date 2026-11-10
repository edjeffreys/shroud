//! Frame-rate policy. Time is passed in, so tests drive it with a fake clock.
//! Backpressure is the main flood control; the rate cap only bounds CPU when the
//! outer terminal keeps up.

use std::time::Duration;

pub const NORMAL_INTERVAL: Duration = Duration::from_micros(1_000_000 / 120);
pub const FLOOD_INTERVAL: Duration = Duration::from_micros(1_000_000 / 30);
pub const FLOOD_AFTER: Duration = Duration::from_millis(250);
pub const CALM_AFTER: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    Idle,
    RenderNow,
    RenderAt(Duration),
}

#[derive(Clone, Debug, Default)]
pub struct Pacer {
    last_frame: Option<Duration>,
    busy_since: Option<Duration>,
    flooding: bool,
    expedited: bool,
}

impl Pacer {
    pub fn new() -> Pacer {
        Pacer::default()
    }

    pub fn is_flooding(&self) -> bool {
        self.flooding
    }

    /// The next frame carries a key's echo: waiting for the rate cap is the latency people feel.
    /// It doesn't count towards flooding, or typing fast enough would trip it.
    pub fn expedite(&mut self) {
        self.expedited = true;
    }

    pub fn decide(&mut self, now: Duration, dirty: bool, backpressured: bool) -> Decision {
        if let Some(last) = self.last_frame
            && now.saturating_sub(last) >= CALM_AFTER
        {
            self.flooding = false;
            self.busy_since = None;
        }
        if !dirty || backpressured {
            return Decision::Idle;
        }
        if self.expedited {
            return Decision::RenderNow;
        }
        let interval = if self.flooding { FLOOD_INTERVAL } else { NORMAL_INTERVAL };
        match self.last_frame {
            Some(last) if now < last + interval => Decision::RenderAt(last + interval),
            _ => Decision::RenderNow,
        }
    }

    pub fn rendered(&mut self, now: Duration) {
        if std::mem::take(&mut self.expedited) {
            self.last_frame = Some(now);
            return;
        }
        let back_to_back = self.last_frame.is_some_and(|l| now.saturating_sub(l) < NORMAL_INTERVAL * 2);
        if back_to_back {
            let since = *self.busy_since.get_or_insert(now);
            if now.saturating_sub(since) >= FLOOD_AFTER {
                self.flooding = true;
            }
        } else {
            self.busy_since = None;
        }
        self.last_frame = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn first_frame_is_immediate_then_capped() {
        let mut p = Pacer::new();
        assert_eq!(p.decide(ms(0), true, false), Decision::RenderNow);
        p.rendered(ms(0));
        assert_eq!(p.decide(ms(1), true, false), Decision::RenderAt(NORMAL_INTERVAL));
    }

    #[test]
    fn backpressure_and_clean_frames_idle() {
        let mut p = Pacer::new();
        assert_eq!(p.decide(ms(0), true, true), Decision::Idle);
        assert_eq!(p.decide(ms(0), false, false), Decision::Idle);
    }

    #[test]
    fn sustained_output_drops_to_flood_rate_and_recovers() {
        let mut p = Pacer::new();
        let mut t = Duration::ZERO;
        while t < ms(400) {
            p.rendered(t);
            t += NORMAL_INTERVAL;
        }
        assert!(p.is_flooding());
        p.decide(t + ms(300), false, false);
        assert!(!p.is_flooding());
    }

    #[test]
    fn an_echo_skips_the_cap_and_never_counts_as_flooding() {
        let mut p = Pacer::new();
        let mut t = Duration::ZERO;
        while t < ms(400) {
            p.expedite();
            assert_eq!(p.decide(t, true, false), Decision::RenderNow);
            p.rendered(t);
            t += ms(1);
        }
        assert!(!p.is_flooding());
        assert_eq!(p.decide(t, true, false), Decision::RenderAt(t - ms(1) + NORMAL_INTERVAL));
        p.expedite();
        assert_eq!(p.decide(t, true, true), Decision::Idle, "backpressure still wins");
    }
}
