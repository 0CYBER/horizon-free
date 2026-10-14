//! Token buckets for request rates and relayed bandwidth, and the
//! per-address state behind them (kept in memory, forgotten after ten idle
//! minutes).
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    time::{Duration, Instant},
};

/// A token bucket: `rate` tokens a second up to `burst`.
#[derive(Clone, Debug)]
pub struct Bucket {
    tokens: f64,
    last: Instant,
    rate: f64,
    burst: f64,
}
impl Bucket {
    pub fn new(rate: f64, burst: f64, now: Instant) -> Self {
        Self {
            tokens: burst,
            last: now,
            rate,
            burst,
        }
    }
    fn refill(&mut self, now: Instant) {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        self.last = now;
    }
    /// Take one token if there is one.
    pub fn try_take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
    pub fn has(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }
    /// Take `n` tokens, going into debt if needed; how long to wait before
    /// sending them so the average stays at the rate.
    pub fn take(&mut self, n: f64, now: Instant) -> Duration {
        self.refill(now);
        self.tokens -= n;
        if self.tokens >= 0.0 || self.rate <= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.rate)
        }
    }
}

/// Addresses are counted by IPv4 address and by IPv6 /64 (one home).
pub fn address_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

#[derive(Clone, Debug)]
pub struct Rates {
    pub register_per_min: f64,
    pub register_burst: f64,
    pub join_per_min: f64,
    pub join_burst: f64,
    /// Unknown codes an address may try in `unknown_window` before it is
    /// blocked for `block`.
    pub unknown_limit: usize,
    pub unknown_window: Duration,
    pub block: Duration,
}
impl Default for Rates {
    fn default() -> Self {
        Self {
            register_per_min: 6.0,
            register_burst: 3.0,
            join_per_min: 12.0,
            join_burst: 6.0,
            unknown_limit: 5,
            unknown_window: Duration::from_secs(600),
            block: Duration::from_secs(600),
        }
    }
}

struct Address {
    register: Bucket,
    join: Bucket,
    unknown: VecDeque<Instant>,
    blocked_until: Option<Instant>,
    seen: Instant,
}

/// Per-address limits; disabled (everything allowed) when the relay can't
/// see client addresses.
pub struct Limiter {
    rates: Rates,
    enabled: bool,
    addresses: HashMap<IpAddr, Address>,
    last_sweep: Instant,
}
impl Limiter {
    pub fn new(rates: Rates, enabled: bool) -> Self {
        Self {
            rates,
            enabled,
            addresses: HashMap::new(),
            last_sweep: Instant::now(),
        }
    }
    fn entry(&mut self, ip: IpAddr, now: Instant) -> &mut Address {
        if now.saturating_duration_since(self.last_sweep) > Duration::from_secs(60) {
            self.last_sweep = now;
            let keep = Duration::from_secs(600).max(self.rates.block);
            self.addresses.retain(|_, a| {
                now.saturating_duration_since(a.seen) < keep
                    || a.blocked_until.is_some_and(|b| b > now)
            });
        }
        let r = &self.rates;
        let a = self
            .addresses
            .entry(address_key(ip))
            .or_insert_with(|| Address {
                register: Bucket::new(r.register_per_min / 60.0, r.register_burst, now),
                join: Bucket::new(r.join_per_min / 60.0, r.join_burst, now),
                unknown: VecDeque::new(),
                blocked_until: None,
                seen: now,
            });
        a.seen = now;
        a
    }
    pub fn allow_register(&mut self, ip: IpAddr, now: Instant) -> bool {
        !self.enabled || self.entry(ip, now).register.try_take(now)
    }
    pub fn allow_join(&mut self, ip: IpAddr, now: Instant) -> bool {
        if !self.enabled {
            return true;
        }
        let a = self.entry(ip, now);
        if a.blocked_until.is_some_and(|b| b > now) {
            return false;
        }
        a.blocked_until = None;
        a.join.try_take(now)
    }
    /// An unknown code from this address; blocks it past the limit.
    pub fn unknown(&mut self, ip: IpAddr, now: Instant) {
        if !self.enabled {
            return;
        }
        let (limit, window, block) = (
            self.rates.unknown_limit,
            self.rates.unknown_window,
            self.rates.block,
        );
        let a = self.entry(ip, now);
        a.unknown.push_back(now);
        while a
            .unknown
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > window)
        {
            a.unknown.pop_front();
        }
        if a.unknown.len() >= limit {
            a.blocked_until = Some(now + block);
            a.unknown.clear();
        }
    }
    pub fn tracked(&self) -> usize {
        self.addresses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_refill_at_their_rate() {
        let t = Instant::now();
        let mut b = Bucket::new(1.0, 2.0, t);
        assert!(b.try_take(t));
        assert!(b.try_take(t));
        assert!(!b.try_take(t));
        assert!(!b.try_take(t + Duration::from_millis(900)));
        assert!(b.try_take(t + Duration::from_millis(1000)));
        // Never above the burst.
        assert!(b.try_take(t + Duration::from_secs(100)));
        assert!(b.try_take(t + Duration::from_secs(100)));
        assert!(!b.try_take(t + Duration::from_secs(100)));
    }

    #[test]
    fn bandwidth_debt_turns_into_a_wait() {
        let t = Instant::now();
        let mut b = Bucket::new(1000.0, 500.0, t);
        assert_eq!(b.take(500.0, t), Duration::ZERO);
        assert_eq!(b.take(1000.0, t), Duration::from_secs(1));
        // After the wait the debt is paid.
        assert_eq!(b.take(0.0, t + Duration::from_secs(1)), Duration::ZERO);
    }

    #[test]
    fn ipv6_counts_by_home() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::9".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(address_key(a), address_key(b));
        assert_ne!(address_key(a), address_key(c));
        let mapped: IpAddr = "::ffff:203.0.113.7".parse().unwrap();
        assert_eq!(
            address_key(mapped),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn unknown_codes_block_an_address_for_a_while() {
        let t = Instant::now();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let other: IpAddr = "203.0.113.8".parse().unwrap();
        let mut l = Limiter::new(
            Rates {
                join_burst: 100.0,
                ..Rates::default()
            },
            true,
        );
        for _ in 0..4 {
            assert!(l.allow_join(ip, t));
            l.unknown(ip, t);
        }
        assert!(l.allow_join(ip, t));
        l.unknown(ip, t);
        assert!(!l.allow_join(ip, t));
        assert!(!l.allow_join(ip, t + Duration::from_secs(599)));
        assert!(l.allow_join(other, t));
        assert!(l.allow_join(ip, t + Duration::from_secs(601)));
        // Unknown codes spread beyond the window don't add up.
        let mut l = Limiter::new(Rates::default(), true);
        for i in 0..10 {
            l.unknown(ip, t + Duration::from_secs(i * 200));
        }
        assert!(l.allow_join(ip, t + Duration::from_secs(2000)));
    }

    #[test]
    fn join_and_register_rates() {
        let t = Instant::now();
        let ip: IpAddr = "198.51.100.1".parse().unwrap();
        let mut l = Limiter::new(Rates::default(), true);
        assert_eq!((0..10).filter(|_| l.allow_register(ip, t)).count(), 3);
        assert_eq!((0..10).filter(|_| l.allow_join(ip, t)).count(), 6);
        assert!(l.allow_join(ip, t + Duration::from_secs(5)));
        let mut off = Limiter::new(Rates::default(), false);
        assert!((0..100).all(|_| off.allow_join(ip, t)));
        assert_eq!(off.tracked(), 0);
    }
}
