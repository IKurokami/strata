// SPDX-License-Identifier: MIT
use super::*;
use std::time::{Duration, Instant};

#[test]
fn cancellation_releases_a_full_packet_queue() {
    let (sender, _receiver) = mpsc::sync_channel(1);
    sender
        .send(Event::Packet(Packet::End(1)))
        .expect("fill queue");
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = thread::spawn(move || send(&sender, Event::Packet(Packet::End(2)), &flag));
    thread::sleep(Duration::from_millis(10));
    cancelled.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(1);
    while !worker.is_finished() {
        assert!(
            Instant::now() < deadline,
            "cancelled worker retained full queue"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert!(worker.join().expect("worker").is_err());
}
