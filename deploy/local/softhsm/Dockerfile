# SoftHSM2 dev HSM (spec §4 HSM dev row, §14): PKCS#11-compatible software
# token emulating CloudHSM for local development. Explicitly non-FIPS (§14.4).
FROM alpine:3.22

# softhsm  -> softhsm2-util + libsofthsm2.so (the PKCS#11 module)
# opensc   -> pkcs11-tool (provisioning + smoke tests)
RUN apk add --no-cache softhsm opensc

# PKCS#11 module path inside this container; part of the env contract
# documented in deploy/local/README.md.
ENV MTC_PKCS11_MODULE_PATH=/usr/lib/softhsm/libsofthsm2.so

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
