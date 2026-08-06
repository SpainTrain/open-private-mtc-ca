# cloud-softhsm: PKCS#11 Hsm backend against SoftHSM2 (ticket softhsm-backend,
# spec §9.3, §14, §18.1). These targets provision a host-side SoftHSM2 token
# and run the crate's tests. The integration tests need a *host* SoftHSM2
# install (a host process cannot dlopen the module inside the docker `softhsm`
# container — see deploy/local/README.md); they self-skip when SoftHSM2 is
# absent, so `softhsm-test` is always safe to run.

.PHONY: softhsm-init softhsm-test softhsm-test-integration

softhsm-init: ## Provision the host-side SoftHSM2 dev token (label mtc-dev, ECDSA P-256 key)
	./scripts/softhsm-init.sh

softhsm-test: ## Run cloud-softhsm unit tests (no SoftHSM2 required)
	cargo test -p cloud-softhsm

softhsm-test-integration: softhsm-init ## Provision SoftHSM2, then run the live PKCS#11 integration tests
	MTC_SOFTHSM_REQUIRE=1 cargo test -p cloud-softhsm --features integration -- --test-threads=1
