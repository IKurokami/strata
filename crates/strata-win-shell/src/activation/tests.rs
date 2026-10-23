// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::AtomicU64;

fn scope() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "strata-activation-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[test]
fn primary_queues_exact_requests_and_rejects_duplicate_claims() {
    let scope = scope();
    let server = Server::bind(&scope).expect("primary endpoint");
    assert!(
        Server::bind(&scope).is_err(),
        "only one primary may own this endpoint"
    );
    let started = Instant::now();
    for index in 0..12 {
        let message = format!("request {index}");
        forward(&scope, message.as_bytes()).expect("acknowledged activation");
        assert_eq!(server.receive().as_deref(), Some(message.as_bytes()));
        assert!(server.receive().is_none());
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "acknowledged requests must release the endpoint immediately: {:?}",
        started.elapsed()
    );
    assert!(forward(&scope, &vec![0; MAX_MESSAGE + 1]).is_err());
    forward(&scope, b"healthy after rejected oversized request").expect("healthy endpoint");
    assert_eq!(
        server.receive().as_deref(),
        Some(b"healthy after rejected oversized request".as_slice())
    );
    drop(server);
    assert!(forward(&scope, b"no primary").is_err());
    Server::bind(&scope).expect("endpoint can be claimed after shutdown");
}

#[test]
fn separate_scopes_do_not_receive_each_others_requests() {
    let first = scope();
    let second = scope();
    let a = Server::bind(&first).expect("first endpoint");
    let b = Server::bind(&second).expect("second endpoint");
    forward(&first, b"a").expect("first activation");
    assert_eq!(a.receive(), Some(vec![b'a']));
    assert!(b.receive().is_none());
    forward(&second, b"b").expect("second activation");
    assert_eq!(b.receive(), Some(vec![b'b']));
}

#[test]
fn abandoned_partial_clients_do_not_hold_shutdown_or_next_launch() {
    let scope = scope();
    let server = Server::bind(&scope).expect("endpoint");
    let (sid, session) = identity().expect("current user");
    let mut client = OpenOptions::new()
        .access_mode(0xc0020000)
        .open(endpoint(&scope, &sid, session))
        .expect("client");
    client
        .write_all(&100u32.to_le_bytes())
        .expect("partial frame");
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if forward(&scope, b"after disconnect").is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "abandoned client wedged the endpoint"
        );
        thread::sleep(POLL);
    }
    assert_eq!(
        server.receive().as_deref(),
        Some(b"after disconnect".as_slice())
    );
    drop(server);
    Server::bind(&scope).expect("shutdown released the pipe");
}
