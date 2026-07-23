#!/bin/bash
# Boot-stage hook: clear a stale init marker so the container healthcheck
# cannot report healthy before the ready.d provisioning re-runs. Relevant on
# `docker restart`, where /tmp persists but LocalStack's in-memory state does not.
rm -f /tmp/mtc_init_done
