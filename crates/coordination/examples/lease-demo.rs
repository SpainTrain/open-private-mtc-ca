//! `cargo run -p coordination --example lease-demo`
//!
//! Walks the primary-region lease lifecycle against the in-memory backend with
//! a controllable [`FakeClock`]: acquire, renew, a takeover refused while the
//! lease is still valid (and while expired-but-within the safety margin), then
//! a successful takeover that atomically bumps the fencing epoch, after which
//! the demoted primary is fenced out (spec §8.2/§8.3).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clock::{Clock, FakeClock};
use cloud_memory::MemoryReplicatedKv;
use cloud_types::ReplicatedKv;
use coordination::{
    HolderId, Lease, LeaseCoordinator, LogId, Region, INITIAL_EPOCH, LEASE_TTL, RENEWAL_INTERVAL,
    TAKEOVER_SAFETY_MARGIN,
};
use eyre::Result;

/// Prints a lease with its expiry expressed relative to the clock's start.
fn describe(tag: &str, lease: &Lease, start: SystemTime) {
    let expires_in = lease.expires_at.duration_since(start).unwrap_or_default();
    println!(
        "    -> {tag}: epoch={}  holder={}  region={}  expires_at=start+{}s",
        lease.epoch.0,
        lease.holder_id,
        lease.region,
        expires_in.as_secs()
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let clock = Arc::new(FakeClock::new(
        UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    ));
    let start = clock.now();
    let kv: Arc<dyn ReplicatedKv> = Arc::new(MemoryReplicatedKv::new());
    let log_id = LogId::new("demo-log")?;

    let dyn_clock: Arc<dyn Clock> = clock.clone();
    let primary = LeaseCoordinator::new(
        Arc::clone(&kv),
        dyn_clock.clone(),
        &log_id,
        HolderId::new("us-east-1/primary"),
        Region::new("us-east-1"),
    );
    let standby = LeaseCoordinator::new(
        Arc::clone(&kv),
        dyn_clock,
        &log_id,
        HolderId::new("us-west-2/standby"),
        Region::new("us-west-2"),
    );

    println!(
        "lease protocol: renew every {}s, {}s TTL, {}s takeover safety margin",
        RENEWAL_INTERVAL.as_secs(),
        LEASE_TTL.as_secs(),
        TAKEOVER_SAFETY_MARGIN.as_secs()
    );

    println!("\n[1] us-east-1 acquires the lease from the unheld state");
    let acquired = primary.acquire().await?;
    describe("acquired", &acquired, start);

    println!(
        "\n[2] +{}s: us-east-1 renews (epoch unchanged, expiry extended)",
        RENEWAL_INTERVAL.as_secs()
    );
    clock.advance(RENEWAL_INTERVAL);
    let renewed = primary.renew(INITIAL_EPOCH).await?;
    describe("renewed", &renewed, start);

    println!("\n[3] us-west-2 tries to take over a valid lease -> refused");
    report_refusal(&standby).await;

    println!(
        "\n[4] +{}s: lease has expired, but is within the {}s safety margin -> takeover still refused (no split brain)",
        (LEASE_TTL + Duration::from_secs(1)).as_secs(),
        TAKEOVER_SAFETY_MARGIN.as_secs()
    );
    clock.advance(LEASE_TTL + Duration::from_secs(1));
    report_refusal(&standby).await;

    println!(
        "\n[5] +{}s: past the safety margin -> us-west-2 takes over and ATOMICALLY bumps the epoch",
        TAKEOVER_SAFETY_MARGIN.as_secs()
    );
    clock.advance(TAKEOVER_SAFETY_MARGIN);
    let taken = standby.claim_lease().await?;
    describe("took over", &taken, start);
    println!(
        "    -> epoch bump: {} -> {} (fencing token advanced)",
        acquired.epoch.0, taken.epoch.0
    );

    println!("\n[6] the demoted us-east-1 primary tries to renew at its stale epoch -> fenced out");
    match primary.renew(INITIAL_EPOCH).await {
        Ok(_) => println!("    -> UNEXPECTED: stale primary renewed"),
        Err(err) => println!("    -> refused as expected: {err}"),
    }

    println!("\ndone.");
    Ok(())
}

/// Attempts a takeover expected to be refused, printing the typed reason.
async fn report_refusal(standby: &LeaseCoordinator) {
    match standby.claim_lease().await {
        Ok(lease) => println!("    -> UNEXPECTED takeover at epoch {}", lease.epoch.0),
        Err(err) => println!("    -> refused as expected: {err}"),
    }
}
