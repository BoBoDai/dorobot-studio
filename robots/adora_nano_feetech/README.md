# Adora Nano · Feetech base

Supported robot adapter for DoRobot Studio, using the serial protocol, motor
IDs, registers and mixer from the user-supplied `feetech-omni-car` driver.
This is a persistent Dora adapter; the supplied macOS executable and its
machine-specific plugin hooks are not needed or copied into the repository.

**Scope:** three-wheel mobile base, including lateral motion. The package has
no arm/head calibration, camera capture, encoder feedback or odometry API.
Studio keeps the URDF as a reference pose and labels speeds as **commanded
ticks**, rather than animating an invented measured pose. Simulation and
existing LeRobot capture remain separate workflows.

## Studio

To control the **moving robot in AISLE**, choose **Robot setup → Simulation →
Start simulation**. Studio starts the Dora graph and opens **Operate** with the
three cameras, trace map and joint model. R1 selects Drive, Left arm, Right arm
or Head; hold L1 to move in that mode. Use **Record** to capture a take.

The base driver's **Bus test** below is a serial-protocol diagnostic. Its
reference URDF intentionally stays still; it does not run robot physics.

1. Open **Robot setup → Real robot**.
2. Select **Adora Nano**.
3. **Bus test** runs the same Dora graph and protocol against a simulated
   serial bus. It never opens a USB device. Use a PS4/PS5 controller to watch
   the three commanded wheel speeds and acknowledged writes under **More**.
4. For a physical base, enter its serial device, or use **Find port** to cycle
   available USB candidates. **Connect** opens only that explicit port.
   No device is connected automatically at startup.
5. Center the sticks and release L1 once. Hold **L1** to drive: left stick
   forward/back and strafe; right stick turn. Release L1 to stop and disable
   wheel torque. **Options** pauses/resumes; release and center again to rearm.
6. **Disconnect** sends zero speeds and torque-off to all three motors.
   Leaving Robot setup/Operate or connecting an external source also stops this
   driver. A stopped/faulted connection requires an explicit reconnect.

Requires `uv`, Python 3.13 (managed by uv), and `dora` CLI **1.0.1** on PATH.
The Python environment is isolated and pinned by `uv.lock`; no AISLE checkout
is required. `DOROBOT_UV` may select a uv executable for a GUI launch.

## Dora flow

```text
PS4/PS5 → gamepad (50 Hz) → control → guard → driver → status (10 Hz)
                                            │
                                    Feetech serial bus
```

`control` implements the deadman/neutral-start/pause behavior; `guard` checks
finite values, sequence, age, bounds and slew. The serial-owning worker has its
own 250 ms input deadline, independent of Dora receive events. Timeout and
controller reconnect latch a stop until a fresh neutral/released input arrives.
The Studio lease stops the graph if its UI stops responding for one second.
The motor worker independently checks the supervisor process and the same UI
lease (1.25 second backstop), so losing the graph supervisor also disables torque.

Initialization sends zero/torque-off before configuring wheel mode. Every
write checks the motor ID, length, checksum and error status in its ACK.
On faults and orderly shutdown, stop/torque-off is attempted on **every** wheel;
unacknowledged stops are reported as faults, never as confirmed stops.
This requires motors configured to return write acknowledgements, as expected
by the original driver's byte-write initialization. No automatic ACK bypass.

This is a host software watchdog, not a controller-board watchdog. Host power
loss, SIGKILL of the serial worker or a severed USB cable can prevent the final
stop packet from reaching the motors. Hardware emergency stopping is still
needed for those failures. No physical motor validation has been performed by
the integration tests.

## Hardware profile

| Motor ID | Position | URDF joint |
|---|---|---|
| 13 | Left front | `wheel_2_joint` |
| 14 | Rear | `wheel_1_joint` |
| 15 | Right front | `wheel_3_joint` |

1,000,000 baud, 8N1; mode register 33, torque 40, signed-magnitude speed 46,
lock 55. The sign bit is `0x8000`. The supplied mixer is preserved:

```text
13 = -0.866 vx + 0.5 vy + omega
14 =             - vy + omega
15 = +0.866 vx + 0.5 vy + omega
```

All three inputs are vendor ticks, including `omega`; they are **not** m/s or
rad/s. Teleoperation uses a conservative 300-tick cap (source hard limit 1500)
with proportional saturation and 600 ticks/s slew. Wheel dimensions, physical
direction and SI conversion require hardware calibration. Simulation wheel
signs/gains are not reused for hardware.

## CLI and tests

From the repository root:

```sh
uv run --frozen --project robots/adora_nano_feetech python robots/adora_nano_feetech/run.py \
  --mock --output /tmp/nano-base-test

# Physical connection: substitute the actual USB serial device.
uv run --frozen --project robots/adora_nano_feetech python robots/adora_nano_feetech/run.py \
  --port /dev/ttyACM0 --output /tmp/nano-base-physical

uv run --frozen --project robots/adora_nano_feetech python -m unittest discover \
  -s robots/adora_nano_feetech -v

# Scripted motion is accepted only with --mock, never a physical --port.
uv run --frozen --project robots/adora_nano_feetech python robots/adora_nano_feetech/run.py \
  --mock --self-test --output /tmp/nano-base-e2e
```

Use a new output directory for each launch. `status.json` is the current
snapshot; `status.jsonl` records acknowledged commands/status; `dora.log`
contains node logs. These are driver diagnostics, not a LeRobot camera dataset.
Generated runtime data belongs outside the repository and includes local port
paths. No local serial identifiers or credentials are part of the profile.

Tests cover exact packet bytes, ACK faults, initialization order, wheel mixing,
proportional limits, deadman/reconnect/pause, slew, stale/reordered input,
independent watchdog shutdown, and pyserial transport over a pseudo-terminal.
The Dora self-test verifies forward, lateral and turning commands, release to
zero, and confirmed torque-off through all four actual nodes.
