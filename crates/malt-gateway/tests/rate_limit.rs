use malt_gateway::rate_limit::RateLimiter;

#[test]
fn under_limit_allowed() {
    let limiter = RateLimiter::new(10);
    for _ in 0..10 {
        assert!(limiter.check("client-1"));
    }
}

#[test]
fn at_limit_rejected() {
    let limiter = RateLimiter::new(5);
    for _ in 0..5 {
        assert!(limiter.check("client-1"));
    }
    assert!(!limiter.check("client-1"));
}

#[test]
fn per_client_isolation() {
    let limiter = RateLimiter::new(2);
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(!limiter.check("client-1"));
    // client-2 is unaffected
    assert!(limiter.check("client-2"));
    assert!(limiter.check("client-2"));
    assert!(!limiter.check("client-2"));
}

#[test]
fn refill_resets() {
    let limiter = RateLimiter::new(2);
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(!limiter.check("client-1"));
    limiter.refill("client-1");
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(!limiter.check("client-1"));
}
