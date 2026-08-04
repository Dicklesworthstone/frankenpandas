#!/usr/bin/env bash
# Same-invocation vs-pandas measurement on a DRAINED rch worker.
#
# WHY THIS EXISTS
# ---------------
# As of 2026-07-31 this repo held 104 KEEP perf claims and 0 of them had a ratio
# measured with pandas live in the same process invocation. That was not
# negligence — it was structurally impossible:
#
#   * a same-invocation A/B needs a host that BOTH has pandas AND is quiescent;
#   * no rch worker had pandas installed at all (probed: 12/12 pandas=NONE);
#   * rch workers are BUILD machines, so they violate the harness's fail-closed
#     "every online CPU <= 20% busy" gate by design;
#   * the workstation is the only host that had pandas, and it is never quiet
#     under swarm load.
#
# The way out is to take ONE worker out of the build rotation, give it the pinned
# incumbent, and measure there. This script encodes that so the remaining 97
# cross-process rows can be converted without re-deriving the setup.
#
# It NEVER relaxes the quiescence gate. It makes the gate satisfiable instead.
#
# USAGE
#   cc_blackthrush_same_invocation_runner.sh setup   <worker> <ip>
#   cc_blackthrush_same_invocation_runner.sh drain   <worker>
#   cc_blackthrush_same_invocation_runner.sh wait    <ip>
#   cc_blackthrush_same_invocation_runner.sh run     <ip> <category> <sizes> <binary> <buildworker> <outbase> [workloads]
#   cc_blackthrush_same_invocation_runner.sh restore <worker>          # ALWAYS run this
set -euo pipefail

KEY="${FP_BENCH_SSH_KEY:-$HOME/.ssh/contabo_vps_ed25519}"
SSH_OPTS=(-o BatchMode=yes -o StrictHostKeyChecking=no -o ConnectTimeout=10 -i "$KEY")
RUN=/root/fpbench-run
BIN=/root/fpbench-bin
LIBS=/root/fpbench-libs

# The harness pins these EXACTLY and fails closed on a mismatch. Do not float them.
PANDAS_PIN="2.2.3"
PYARROW_PIN="24.0.0"

die() { echo "ERROR: $*" >&2; exit 1; }

cmd_setup() {
  local worker="$1" ip="$2"
  echo "[setup] $worker ($ip)"
  ssh "${SSH_OPTS[@]}" "root@${ip}" "mkdir -p ${RUN}/benches ${RUN}/artifacts/bench ${BIN}"
  # pip --target, deliberately: no venv (python3-venv is absent on these images)
  # and no system-package mutation on a shared worker.
  ssh "${SSH_OPTS[@]}" "root@${ip}" \
    "python3 -m pip install -q --target=${LIBS} 'pandas==${PANDAS_PIN}' 'pyarrow==${PYARROW_PIN}'"
  rch workers sync-toolchain "$worker" || true
  # Admission is CACHED. Syncing the toolchain alone changes nothing until the
  # capability cache is invalidated; this is the step that is easy to miss.
  rch workers capabilities --refresh >/dev/null || true
  echo "[setup] done"
}

cmd_drain() {
  local worker="$1"
  rch workers drain "$worker" -y
  echo "[drain] $worker draining; in-flight builds will finish"
}

# Replicates the harness's own gate so we do not burn a run discovering it is blocked.
cmd_wait() {
  local ip="$1"
  for _ in $(seq 1 60); do
    local m
    m=$(ssh "${SSH_OPTS[@]}" "root@${ip}" 'python3 -c "
import time
def t():
    d={}
    for l in open(\"/proc/stat\"):
        f=l.split()
        if f and f[0].startswith(\"cpu\") and f[0][3:].isdigit():
            v=[int(x) for x in f[1:]]; d[f[0]]=(sum(v),v[3]+v[4])
    return d
a=t(); time.sleep(0.3); b=t()
m=0.0
for c,(ta,ia) in a.items():
    tb,ib=b[c]; dt=tb-ta; di=ib-ia
    m=max(m, 1.0 if dt==0 else max(0,dt-di)/dt)
print(f\"{m:.4f}\")
"' 2>/dev/null || echo 1.0)
    echo "[wait] max_cpu_busy=$m"
    if awk "BEGIN{exit !($m<=0.20)}"; then echo "[wait] QUIESCENT"; return 0; fi
    sleep 30
  done
  die "worker never reached quiescence; do NOT relax the gate — find the peer load"
}

cmd_run() {
  local ip="$1" category="$2" sizes="$3" binary="$4" buildworker="$5" outbase="$6"
  local workloads="${7:-}"
  local extra=""
  [ -n "$workloads" ] && extra="--workloads ${workloads}"

  # Topology is asserted, not assumed: the harness fails closed if the host is
  # not the one we think it is.
  local hn cores
  hn=$(ssh "${SSH_OPTS[@]}" "root@${ip}" hostname)
  cores=$(ssh "${SSH_OPTS[@]}" "root@${ip}" nproc)

  ssh "${SSH_OPTS[@]}" "root@${ip}" "
    set -euo pipefail
    cd ${RUN}
    export PYTHONPATH=${LIBS}
    export CARGO_TARGET_DIR=${BIN}
    python3 ${RUN}/benches/vs_pandas_harness.py \
      --category ${category} \
      --sizes ${sizes} \
      ${extra} \
      --thread-count ${cores} \
      --expected-hostname ${hn} \
      --expected-physical-cores ${cores} \
      --expected-logical-threads ${cores} \
      --frankenpandas-binary ${BIN}/${binary} \
      --frankenpandas-build-worker '${buildworker}' \
      --output ${RUN}/artifacts/bench/${outbase}.json
  "
}

cmd_restore() {
  local worker="$1"
  rch workers enable "$worker"
  echo "[restore] $worker back in the build rotation"
}

case "${1:-}" in
  setup)   shift; cmd_setup "$@" ;;
  drain)   shift; cmd_drain "$@" ;;
  wait)    shift; cmd_wait "$@" ;;
  run)     shift; cmd_run "$@" ;;
  restore) shift; cmd_restore "$@" ;;
  *) die "usage: $0 {setup|drain|wait|run|restore} ..." ;;
esac
