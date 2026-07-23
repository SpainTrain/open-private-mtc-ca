# Dev-environment diagnostics (spec §18.8).
#
# `make doctor` checks: Docker daemon, Compose v2, required ports, Rust
# toolchain, cargo-watch, evcxr_repl, SoftHSM2, LocalStack image, disk space.
# Each check prints PASS/FAIL/WARN with copy-pasteable remediation.

.PHONY: doctor doctor-test

doctor: ## Diagnose the dev environment and suggest fixes
	@scripts/doctor.sh

doctor-test: ## Smoke-test doctor diagnostics (exit codes, output format)
	@scripts/doctor-test.sh
