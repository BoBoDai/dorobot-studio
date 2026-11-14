//! Validate a local v2/v3 dataset without opening the desktop application.
use dorobot_flex::data::LeRobotDataset;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: check_dataset DATASET_DIR")?;
    let ds = LeRobotDataset::open(&path)?;
    let cameras: Vec<_> = ds
        .info
        .features
        .iter()
        .filter(|(_, f)| f.dtype == "video")
        .map(|(k, _)| k)
        .collect();
    let mut count = 0;
    for ep in &ds.episodes {
        let data = ds.load_episode(ep.episode_index)?;
        if data.frames.is_empty()
            || data.frames.iter().any(|f| {
                !f.timestamp.is_finite() || f.state.iter().chain(&f.action).any(|v| !v.is_finite())
            })
        {
            return Err(format!(
                "episode {} contains empty or non-finite data",
                ep.episode_index
            )
            .into());
        }
        if data
            .frames
            .windows(2)
            .any(|w| w[0].timestamp >= w[1].timestamp)
        {
            return Err(
                format!("episode {} timestamps are not increasing", ep.episode_index).into(),
            );
        }
        for key in &cameras {
            if !data.video_paths.get(*key).is_some_and(|p| p.is_file()) {
                return Err(format!("episode {}: missing video {key}", ep.episode_index).into());
            }
        }
        count += data.frames.len();
        println!(
            "episode {}: {} frames, {} cameras",
            ep.episode_index,
            data.frames.len(),
            cameras.len()
        );
    }
    if count as u64 != ds.info.total_frames {
        return Err("metadata total_frames does not match the episode data".into());
    }
    println!(
        "OK: {} episodes, {count} frames, {} fps, {}",
        ds.episodes.len(),
        ds.info.fps,
        ds.info.robot_type
    );
    Ok(())
}
