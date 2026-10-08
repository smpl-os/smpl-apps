#!/bin/bash
# usage: run.sh RUN SCRIPT [timeout]  -> runs the real daemon (simulate, non-emitting sink) inside the lease driver
F=/home/blin/.copilot/session-state/9147f090-d696-4b70-af0f-aa5a935f364f/files
R=$1; S=$2; T=${3:-300}
SVC=$(python3 -c "import json;print(json.load(open('$R/ready.json'))['service'])")
python3 $F/k23_ctl.py $R "{\"op\":\"exec\",\"timeout\":$T,\"argv\":[\"/mnt/ai/keypad-lab/build/control-surface/control-surfaced\",\"simulate\",\"--kdenlive-service\",\"$SVC\",\"--trace\",\"-c\",\"$F/k23-acceptance/config.jsonc\",\"$S\"]}" $((T+30))
