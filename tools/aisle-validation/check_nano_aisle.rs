//! Exercise Studio's production backend using an AISLE/Nexus Nano recording.
use dorobot_flex::api::{files::FileBackend, Backend, Intent, Tag};
use dorobot_flex::data::{video_decoder::VideoDecoder, LeRobotDataset};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args().nth(1).ok_or("dataset path required")?);
    let ds = LeRobotDataset::open(&root)?;
    assert_eq!(ds.info.robot_type, "adora_nano");
    assert_eq!(ds.info.total_frames, 90);
    assert_eq!(ds.episodes.len(), 3);
    let mut backend = FileBackend::with_open(&root, None);
    assert_eq!(backend.library().datasets.len(), 1);
    assert_eq!(backend.playback().state_names.len(), 17);
    assert_eq!(backend.playback().action_names.len(), 17);
    assert!(backend.playback().robot_urdf.is_some());
    let mut checked_video_frames = 0;
    for episode in 0..3 {
        backend.dispatch(Intent::SelectEpisode(episode));
        let data = ds.load_episode(episode)?;
        let playback = backend.playback();
        assert_eq!(playback.selected, Some(episode));
        assert_eq!(playback.joint_frames.len(), 30);
        assert_eq!(playback.stats.state_channels, 17);
        assert_eq!(playback.stats.action_channels, 17);
        assert!(playback.stats.drift_frames.abs() < 1e-6);
        for (pose, source) in playback.joint_frames.iter().zip(&data.frames) {
            // Nano radians must reach the URDF unchanged, unlike SO100 normalization.
            assert_eq!(pose, &source.state);
        }
        let path = data.video_paths.get("observation.images.front").unwrap();
        let mut video = VideoDecoder::open(path)?;
        for frame_index in [0, 15, 29, 7, 0, 29] {
            let frame = video.get_frame(frame_index)?;
            assert_eq!(frame.frame_index, frame_index);
            assert_eq!((frame.width, frame.height), (320, 240));
            assert_eq!(frame.data.len(), 320 * 240 * 3);
            checked_video_frames += 1;
        }
        backend.dispatch(Intent::Seek(1.5));
        assert!((backend.playback().current_time - 1.5).abs() < 1e-9);
        backend.dispatch(Intent::StepFrames(1));
        assert!((backend.playback().current_time - 1.6).abs() < 1e-9);
        backend.dispatch(Intent::TogglePlay);
        assert!(backend.playback().is_playing);
        backend.dispatch(Intent::TogglePlay);
        assert!(!backend.playback().is_playing);
    }
    let original = fs::read(root.join("data/chunk-000/episode_000000.parquet"))?;
    backend.dispatch(Intent::TagEpisode {
        episode: 0,
        tag: Some(Tag::Good),
    });
    backend.dispatch(Intent::DeleteEpisode(0));
    assert_eq!(backend.playback().episodes.len(), 2);
    backend.dispatch(Intent::UndoExclude);
    assert_eq!(backend.playback().episodes.len(), 3);
    assert_eq!(
        original,
        fs::read(root.join("data/chunk-000/episode_000000.parquet"))?
    );
    drop(backend);
    let mut reopened = FileBackend::with_open(&root, None);
    assert_eq!(reopened.playback().episodes[0].tag, Some(Tag::Good));
    reopened.dispatch(Intent::TagEpisode {
        episode: 0,
        tag: None,
    });
    assert_eq!(reopened.playback().episodes[0].tag, None);
    println!(
        "{}",
        serde_json::json!({
            "ok":true,"episodes":3,"frames":90,"state_channels":17,"action_channels":17,
            "video_seek_checks":checked_video_frames,"pose_values_preserved":true,
            "transport_intents":true,"tag_restart":true,"exclude_undo_preserved_bytes":true,
            "plotted_channels":reopened.playback().state_series.len(),
            "interactive_window_tested":false
        })
    );
    Ok(())
}
