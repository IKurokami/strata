// SPDX-License-Identifier: MIT

use super::*;
use std::time::Instant;

#[test]
fn dropping_windows_audio_releases_the_playing_pipeline() {
    let output = PcmOutput::new(true, 0.0, Duration::from_millis(100)).unwrap();
    let pipeline = output.pipeline.clone();
    output.push(vec![0; MAX_CHUNK_BYTES], 0).unwrap();
    output.play().unwrap();
    let (result, state, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    result.unwrap();
    assert_eq!(state, gst::State::Playing);
    drop(output);
    let deadline = Instant::now() + Duration::from_secs(5);
    while pipeline.current_state() != gst::State::Null {
        assert!(
            Instant::now() < deadline,
            "audio teardown retained a live pipeline"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
