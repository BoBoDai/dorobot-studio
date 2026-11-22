# SO-101 Live mirror

Dorobot2 can publish the physical follower's measured positions while it runs teleop. Studio displays them in **Robot control → Live mirror**. This panel receives joint state. Dorobot2 owns the serial connections, calibration, and motor commands.

## Run on one Linux PC

Set up a Dorobot2 edge environment and install the opt-in Zenoh extra from the Dorobot2 checkout.

```sh
uv pip install --python .venv/bin/python -e '.[telemetry]'
.venv/bin/python tools/telemetry/synthetic_zenoh.py --listen tcp/127.0.0.1:7447
```

From the Studio checkout, run the desktop binary and open **Live mirror**. The no-video build is enough for joint preview.

```sh
cargo run --locked -p dorobot-flex --no-default-features -- --zenoh-connect tcp/127.0.0.1:7447
```

The synthetic arm should appear and move without any serial connections. Stop and restart the synthetic source to see Studio hold the previous pose while disconnected and accept the replacement session.

For the physical follower, stop the synthetic source before reusing its port. Add the following to your local Dorobot2 teleop profile and keep the existing ports and calibration paths. Set `follower_arm` to the configured arm name.

```yaml
telemetry:
  robot_id: so101-bench
  follower_arm: follower
  multicast: false
  listen_endpoints: [tcp/127.0.0.1:7447]
```

Generate the graph first. Confirm that the follower still receives the local teleop action and `zenoh_telemetry` subscribes to its measured `all_joint` output. Clear the follower's workspace and place both arms in similar resting poses before the bounded test.

```sh
.venv/bin/dorobot teleop -c .local/so101-live-mirror.yaml --dry-run --output .local/mirror-flow.yml
.venv/bin/dora run .local/mirror-flow.yml --stop-after 35s
```

Move each leader joint through a small range, including the gripper. Watch the physical follower and viewer to check direction and endpoints. Close Studio during a second run to check that local teleop continues. Wait for Dora to stop before using the ports again.

## Run Studio on macOS

Use the same Studio build and model resources on the Mac. On the Linux robot host, remove the explicit `listen_endpoints` setting and enable `multicast`. Start Studio without `--zenoh-connect` and open **Live mirror**. Test both startup orders on the same LAN. When the network filters multicast, configure a reachable peer on both ends or use a Zenoh router with the client mode option.

For capture-to-viewer measurements, start Studio with `RUST_LOG=dorobot_plugin_control=trace` and use the `capture_to_view_ms` values. Synchronize the two host clocks and record their offset uncertainty before comparing timestamps. This measurement ends when Studio applies the pose to the viewer.

## Linux verification

The Linux no-video Studio build and SO-101 model load passed. The synthetic source reached the desktop over TCP loopback and with automatic peer scouting on the same host. Studio resumed pose updates after a publisher restart in both modes.

| Source | Poses applied | p50 | p95 |
| --- | ---: | ---: | ---: |
| Synthetic, TCP loopback | 195 | 9.3 ms | 17.6 ms |
| Synthetic, same-host peer scouting | 193 | 10.4 ms | 17.7 ms |
| Physical follower, read-only TCP loopback | 229 | 8.8 ms | 18.6 ms |
| Physical follower, powered teleop TCP loopback | 945 | 9.6 ms | 18.2 ms |

The read-only run used measured positions from the calibrated follower with torque off.

The 35-second powered Linux run used the stock Dorobot2 teleop loop and the Studio desktop window. Studio applied 945 measured follower poses. Elbow flex moved through 76° and wrist flex through 62°. The gripper moved only 0.1 percentage points, so this run does not establish its full travel. Dora stopped cleanly, and a read-back verified torque off on all 12 motors. Linux-to-macOS LAN acceptance is pending.
