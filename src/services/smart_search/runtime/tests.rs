// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn fake_embedding_protocol_rejects_stale_malformed_and_nonfinite_vectors() {
    let mut values = vec![0.0_f32; 768];
    values[0] = 2.0;
    let bytes: Vec<_> = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let mut response = json!({"id":7,"ok":true,"dim":768,"count":1,"vectors":base64::engine::general_purpose::STANDARD.encode(&bytes),"errors":[null]});
    let vectors = decode_vectors(&response, 7, 1).expect("normalized vectors");
    assert_eq!(vectors[0].as_ref().expect("vector")[0], 1.0);
    assert!(decode_vectors(&response, 8, 1).is_err());
    assert!(decode_vectors(&response, 7, 2).is_err());
    response["dim"] = json!(384);
    assert!(decode_vectors(&response, 7, 1).is_err());
    response["dim"] = json!(768);
    response["vectors"] = json!(base64::engine::general_purpose::STANDARD.encode([0; 4]));
    assert!(decode_vectors(&response, 7, 1).is_err());
    values[0] = f32::NAN;
    let bytes: Vec<_> = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    response["vectors"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
    assert!(decode_vectors(&response, 7, 1).is_err());
}

#[cfg(unix)]
#[test]
fn cancelled_timed_out_and_malformed_replies_stop_the_worker_before_returning() {
    for failure in ["cancelled", "timed out", "malformed"] {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("fake worker");
        let input = child.stdin.take().expect("worker input");
        let (send, replies) = mpsc::channel();
        if failure == "malformed" {
            send.send(Err("malformed".into())).expect("fake reply");
        }
        let mut worker = Embedder {
            child,
            input,
            replies,
            sequence: 0,
            last_used: Instant::now(),
        };
        let timeout = if failure == "timed out" {
            Duration::ZERO
        } else {
            Duration::from_secs(5)
        };
        let error = worker
            .receive(timeout, &|| failure == "cancelled")
            .expect_err("rejected request");
        assert!(error.contains(failure), "{error}");
        assert!(
            worker.child.try_wait().expect("worker status").is_some(),
            "failed worker must be stopped before returning"
        );
    }
}
