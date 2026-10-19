#!/bin/sh
# Live 120 Hz tissue / 30 FPS output on the ARDY jump, as factor-refresh measured:
# selected vs accepted s40-k32-v4 asset, each with the linear and mls+smoothing-4
# skin embedding, interleaved and repeated because host load is uncontrolled.
set -u
O=artifacts/soft-tissue/live120-01
P=artifacts/soft-tissue/lab-jump-01/packets.ndjson
G=../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb
M=../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json
SEL=assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json
V4=assets/tissue/beautiful_assistant.base-calibrated-s40-k32-v4-prototype.json
for r in 4 5; do
  for c in "selected-linear $SEL" "v4-mls4 $V4 mls"; do
    set -- $c; name=$1; asset=$2; embed=${3:-}
    extra=""; [ -n "$embed" ] && extra="CC_AVATAR_TISSUE_SURFACE_EMBEDDING=mls CC_AVATAR_TISSUE_SURFACE_SMOOTHING=4"
    echo "$(date +%T) $name-r$r load $(cut -d' ' -f1 /proc/loadavg)"
    env CC_RENDER_FAST=0 CC_RENDER_CLOCK_TRACE=0 CC_AVATAR_TISSUE_HZ=120 CC_AVATAR_TISSUE_ITERATIONS=32 \
      CC_TISSUE_LAB_FPS=30 CC_TISSUE_LAB_CLOCK=live CC_TISSUE_LAB_TRACE_ONLY=1 CC_AVATAR_TISSUE_TRACE_NODES=0 RUST_LOG=info $extra \
      target/debug/examples/tissue_lab $G $M $P $asset front $O/$name-r$r > $O/$name-r$r.log 2>&1
    echo "  exit $?"
  done
done
