# Contributing to DoRobot Studio

Open a pull request against `main` and link the tracking issue when applicable.
Describe the behavior change, verification, and compatibility impact. For UI
changes, attach a screenshot or recording. For robot changes, record which
hardware or simulator was exercised and what the test measured.

## Checks

Run the same Linux workspace tests as Studio CI.

```sh
cargo test --locked --workspace --no-default-features
```

Branches with the Nexus worker also run its protocol and artifact tests.

```sh
python3 -m unittest discover -s tools/nexus_workflow -p test_worker.py
```

Run relevant video, GPU, macOS, and physical robot checks separately when your
change touches those paths. Use the [functional review guide](docs/FUNCTIONAL_REVIEW.md)
and the [runtime guide](runtime/README.md) for setup.

## Review

A maintainer reviews the change and its test evidence before approval.
Claude reviews ready pull requests from the repository. To enable reviews, a
repository administrator adds `ANTHROPIC_API_KEY` in GitHub under Settings →
Secrets and variables → Actions. Claude reads the GitHub diff and publishes
an advisory review for each reviewed head SHA. CI runs on pull requests from
forks with a read-only token. Maintainers handle approval and merging.
