# Update signatures

ZapFast verifies `checksums.txt.sig`, a raw 64-byte Ed25519 signature over the
exact bytes of `checksums.txt`, before parsing checksums or downloading a package.
The trusted 32-byte public key is embedded from `assets/update-public-key.hex`.
Missing, truncated, or invalid signatures fail closed. There is no unsigned
fallback, runtime key override, or key fetched from the release being verified.
Exact filenames bind each signed checksum to its version and platform.

The `release-signing` GitHub environment holds `ZAPFAST_UPDATE_SIGNING_KEY`
(an Ed25519 PKCS#8 PEM private key). It is restricted to `v*` tags and requires
maintainer approval. Only the final signing/publishing job uses it, never PR
builds or compilation. Approve the candidate's exact commit and build results
before allowing that job to run. Do not bypass the approval gate.

The pinned `native-packages` `sign-release` action generates the manifest,
checks that the secret matches the embedded public key, checks the artifact
hashes, signs the checksums, and verifies its output. The key is read in-process;
no temporary private-key file is created. Never enable shell tracing in this
step or print the signing secret. Signing and attestation machinery are shared;
ZapFast still owns its key, approval environment and updater verification.
See the [shared signing guide](https://github.com/crmne/native-packages/blob/dc0cb1586894ca0239c53fd02d4da8064712671c/docs/_guides/release-signing.md).

Build jobs also publish GitHub artifact attestations. These identify the
repository, workflow and commit that built each file; they do not replace the
updater's publisher-signature check or prove the source is safe. Consumers can
run `gh attestation verify FILE -R crmne/zapfast`. See
[GitHub's attestation documentation](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).

## Key custody and rotation

Keep a protected backup of the private key outside the checkout, and back it up
in a password manager or encrypted storage. GitHub does not permit retrieving
an uploaded secret. Never commit a private key, including a retired one.

The current format trusts one key. Before planned rotation, implement and test
a backward-compatible multi-key/signature transition. Ship an old-key-authorized
bridge version that also trusts the new key, and retain old-key signatures while
older clients migrate. Do not simply replace the secret: already-installed
clients only trust the key embedded in their binary.

If the key is lost or compromised, stop publishing automatic updates, revoke
the environment secret, investigate the affected builds, and publish an
independently verified manual installer with a new key. Old clients cannot
safely discover a replacement trust root from unsigned release metadata.

The initial signature-enforcing version must itself be obtained through a
trusted installation. Older clients that only checked hashes do not gain
signature protection until upgraded. This is separate from Apple notarization
and Windows Authenticode. CI-held signing also remains dependent on GitHub and
the protected workflow, unlike a genuinely offline signing authority.
