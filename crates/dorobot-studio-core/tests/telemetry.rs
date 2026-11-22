use dorobot_studio_core::api::telemetry::{Descriptor, PoseFrame, SourceCatalog, MAX_PAYLOAD};
use serde_json::json;
use std::time::{Duration, Instant};

const SESSION: &str = "a660dc29-8090-4553-858c-0219442d9d4d";
const NEXT: &str = "a660dc29-8090-4553-858c-0219442d9d4e";

fn key(session: &str, kind: &str) -> String {
    format!("dorobot/telemetry/v1/robots/bench/sessions/{session}/{kind}")
}

fn payload(session: &str, sequence: u64) -> serde_json::Value {
    json!({
        "version": 1, "robot_id": "bench", "session_id": session,
        "model_id": "so101-follower-new-calib", "sequence": sequence,
        "capture_monotonic_ns": 41_000_000_000u64,
        "joints": [{"name": "shoulder_pan", "position": 4.2, "unit": "deg"}]
    })
}

fn pose(session: &str, sequence: u64) -> PoseFrame {
    PoseFrame::decode(
        &key(session, "joint_state"),
        &serde_json::to_vec(&payload(session, sequence)).unwrap(),
    )
    .unwrap()
}

fn descriptor(session: &str) -> Descriptor {
    let bytes = serde_json::to_vec(&json!({
        "version": 1, "robot_id": "bench", "session_id": session,
        "model_id": "so101-follower-new-calib", "nominal_hz": 30.0
    }))
    .unwrap();
    Descriptor::decode(&key(session, "descriptor"), &bytes).unwrap()
}

#[test]
fn wire_validates_topic_and_bounds() {
    let topic = key(SESSION, "joint_state");
    assert_eq!(pose(SESSION, 42).sequence, 42);
    assert!(PoseFrame::decode(&topic, &vec![b' '; MAX_PAYLOAD + 1]).is_err());
    assert!(PoseFrame::decode(&topic, b"broken").is_err());
    assert!(PoseFrame::decode(
        &key(NEXT, "joint_state"),
        payload(SESSION, 1).to_string().as_bytes()
    )
    .is_err());

    for (field, value) in [
        ("version", json!(2)),
        (
            "joints",
            json!([{"name":"a", "position": null, "unit":"rad"}]),
        ),
        (
            "joints",
            json!([{"name":"a", "position": 0, "unit":"unknown"}]),
        ),
        (
            "joints",
            json!([
                {"name":"a", "position": 0, "unit":"rad"},
                {"name":"a", "position": 1, "unit":"m"}
            ]),
        ),
    ] {
        let mut invalid = payload(SESSION, 1);
        invalid[field] = value;
        assert!(PoseFrame::decode(&topic, invalid.to_string().as_bytes()).is_err());
    }
}

#[test]
fn catalog_rejects_out_of_order_and_keeps_latest() {
    let mut catalog = SourceCatalog::default();
    let now = Instant::now();
    let first = pose(SESSION, 42);
    assert!(catalog.add_source(descriptor(SESSION)));
    assert!(catalog.accept_pose(first.clone(), now));
    assert!(catalog.accept_pose(pose(SESSION, 44), now));
    assert!(!catalog.accept_pose(first.clone(), now));
    let latest = catalog.latest(&first.source).unwrap();
    assert_eq!(latest.frame.sequence, 44);
    assert_eq!(latest.missed_samples, 1);
    assert!(catalog.is_stale(&first.source, now + Duration::from_millis(1001)));
}

#[test]
fn catalog_isolates_restarted_session() {
    let mut catalog = SourceCatalog::default();
    let now = Instant::now();
    let first = pose(SESSION, 42);
    let replacement = pose(NEXT, 0);
    assert!(catalog.add_source(descriptor(SESSION)));
    assert!(catalog.accept_pose(first.clone(), now));
    catalog.remove_source(&first.source);
    assert!(catalog.add_source(descriptor(NEXT)));
    assert!(catalog.accept_pose(replacement.clone(), now));
    assert!(!catalog.accept_pose(pose(SESSION, 43), now));
    assert_eq!(
        catalog.latest(&replacement.source).unwrap().frame.sequence,
        0
    );
}

#[test]
fn mirror_selection_holds_pose_until_matching_model_is_ready() {
    use dorobot_studio_core::api::telemetry::TelemetryState;
    let mut mirror = TelemetryState::default();
    let mut catalog = SourceCatalog::default();
    let now = Instant::now();
    let first = pose(SESSION, 42);
    catalog.add_source(descriptor(SESSION));
    catalog.accept_pose(first.clone(), now);
    mirror.update(&catalog, now, |_| true);
    assert_eq!(
        mirror
            .render_frame(&first.source, &first.model_id)
            .unwrap()
            .sequence,
        42
    );

    let replacement = pose(NEXT, 0);
    catalog.add_source(descriptor(NEXT));
    catalog.accept_pose(replacement.clone(), now);
    mirror.update(&catalog, now, |_| true);
    assert!(
        mirror.selected.is_none(),
        "two live sessions need an explicit choice"
    );
    mirror.select(replacement.source.clone());
    mirror.update(&catalog, now, |_| true);
    assert!(mirror
        .render_frame(&first.source, &first.model_id)
        .is_none());
    assert!(mirror
        .render_frame(&replacement.source, "wrong-model")
        .is_none());
    assert_eq!(
        mirror
            .render_frame(&replacement.source, &replacement.model_id)
            .unwrap()
            .sequence,
        0
    );
}
