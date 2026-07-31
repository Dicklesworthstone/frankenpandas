#!/usr/bin/env bash
# Wait for a SUSTAINED quiet host, then run the whole-job pipeline arm.
# Does NOT touch the gate: it waits for the gate's own condition to hold on its
# own for long enough to be predictive, then hands off to the harness, which
# re-checks independently before and after every arm.
#
# v2: 5 clear seconds proved not predictive -- the harness re-gated ~1s later
# and hit fresh busy CPUs. The full 1M run is only ~60-90s, so require a
# 20-second sustained lull and RETRY rather than giving up after one attempt.
set -u
cd /data/projects/frankenpandas || exit 1
LOG=/data/tmp/claude-1000/-data-projects-frankenpandas/b5bd99d1-3bf0-4326-9126-c6941198fe90/scratchpad
DEADLINE=$(( $(date +%s) + 480 ))
REQUIRED_CLEAR=15
attempt=0

log() { printf '%s %s\n' "$(date -Is)" "$1" >> "$LOG/watcher.log"; }

consecutive=0
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  busy=$(python3 - <<'PY'
import time
def ticks():
    d={}
    for line in open('/proc/stat'):
        if line.startswith('cpu') and line[3].isdigit():
            f=line.split(); cpu=int(f[0][3:])
            d[cpu]=(int(f[4])+int(f[5]), sum(int(x) for x in f[1:]))
    return d
a=ticks(); time.sleep(1.0); b=ticks()
print(sum(1 for c in a if (b[c][1]-a[c][1])>0
          and 1.0-(b[c][0]-a[c][0])/(b[c][1]-a[c][1])>0.20))
PY
)
  if [ "$busy" = "0" ]; then consecutive=$(( consecutive + 1 )); else consecutive=0; fi

  if [ "$consecutive" -ge "$REQUIRED_CLEAR" ]; then
    attempt=$(( attempt + 1 ))
    log "LAUNCHING harness (attempt $attempt after ${consecutive}s sustained clear)"
    timeout 2400 python3 benches/vs_pandas_harness.py \
        --category pipeline --sizes 1M \
        --expected-hostname thinkstation1 \
        > "$LOG/pipeline_1m_run.attempt${attempt}.log" 2>&1
    rc=$?                                   # capture BEFORE any substitution
    log "harness attempt $attempt exit=$rc"
    if [ "$rc" -eq 0 ]; then
      log "SUCCESS"
      exit 0
    fi
    consecutive=0
  fi
done
log "DEADLINE reached after $attempt attempt(s); host never stayed clear"
exit 3
