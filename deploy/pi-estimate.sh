#!/bin/bash
# Estimate the Pi 5's GPU time per frame for the Planet and Solar scenes,
# without a Pi: count every draw's pixels on this machine's GPU driver (it
# must support pipeline statistics queries; Mesa's lavapipe does), then cost
# them with the Pi compiler's numbers in deploy/v3d-shaderdb/baseline.txt.
#
#   deploy/pi-estimate.sh [estimate.py options]   e.g. --ms-per-inst 0.03
#
# PI_ESTIMATE_SEEDS picks the solar systems (default 5,23). Keep the baseline
# current (deploy/v3d-shaderdb/check.sh . --update) so new pipelines have
# rows. See deploy/v3d-shaderdb/estimate.py for the model and calibration.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --release planet::tests::pi_estimate -- --ignored --quiet
cargo test --release solar::tests::pi_estimate -- --ignored --quiet
python3 deploy/v3d-shaderdb/estimate.py target/pi-estimate/planet.tsv target/pi-estimate/solar.tsv "$@"
